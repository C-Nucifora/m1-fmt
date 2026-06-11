use clap::Parser;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process;

mod config_resolve;

#[derive(Parser, Debug)]
#[command(name = "m1-fmt", version, about = "Autoformatter for MoTeC M1 scripts")]
struct Args {
    /// Files or directories to format (directories are searched recursively
    /// for `.m1scr` and require --check, --diff, or -i; a lone `-`, or no
    /// files, reads from stdin)
    files: Vec<PathBuf>,

    /// Check mode: exit 1 if any file would change, don't write
    #[arg(long)]
    check: bool,

    /// Write result to file in place
    #[arg(short = 'i', long = "in-place")]
    in_place: bool,

    /// Print a unified diff instead of formatted output
    #[arg(long)]
    diff: bool,

    /// Filename to use when reading from stdin
    #[arg(long, default_value = "<stdin>")]
    stdin_filename: String,

    /// Maximum consecutive blank lines to keep (default 2; overrides .m1fmt.toml)
    #[arg(long)]
    max_blank_lines: Option<usize>,

    /// Hard column ceiling used for wrapping (default 88; overrides .m1fmt.toml)
    #[arg(long)]
    line_width: Option<usize>,

    /// Format only the given 1-based inclusive line range (`START:END`); the rest
    /// of the buffer is left byte-for-byte unchanged. The range is snapped outward
    /// to whole top-level statements. For LSP/editor format-on-selection.
    #[arg(long, value_name = "START:END")]
    range: Option<String>,
}

/// Read a file's text via the tolerant workspace decoder, but preserve a leading
/// UTF-8 BOM that the decoder strips (m1-workspace#9). A formatter round-trips its
/// input, so a BOM-prefixed file must keep its BOM; `format_str_with` then carries
/// it through to the output.
fn read_text_preserving_bom(path: &std::path::Path) -> std::io::Result<String> {
    let bytes = std::fs::read(path)?;
    let had_bom = bytes.starts_with(&[0xEF, 0xBB, 0xBF]);
    let decoded = m1_workspace::decode(bytes);
    Ok(if had_bom {
        format!("\u{FEFF}{decoded}")
    } else {
        decoded
    })
}

/// Parse a `START:END` (1-based, inclusive) range argument.
fn parse_range(s: &str) -> Option<(usize, usize)> {
    let (a, b) = s.split_once(':')?;
    let a: usize = a.trim().parse().ok()?;
    let b: usize = b.trim().parse().ok()?;
    (a != 0 && b != 0 && a <= b).then_some((a, b))
}

/// Replace input lines `start..=end` (0-based, inclusive) with `replacement`,
/// leaving every other line byte-for-byte unchanged.
fn splice_lines(src: &str, start: usize, end: usize, replacement: &str) -> String {
    let lines: Vec<&str> = src.split('\n').collect();
    let mut out: Vec<String> = Vec::new();
    out.extend(lines[..start].iter().map(|s| s.to_string()));
    out.extend(
        replacement
            .trim_end_matches('\n')
            .split('\n')
            .map(|s| s.to_string()),
    );
    out.extend(lines[end + 1..].iter().map(|s| s.to_string()));
    out.join("\n")
}

/// Format `src`, either whole or (when `range` is set) only the statements
/// overlapping the 1-based inclusive line range. Returns the resulting buffer,
/// whether it changed, and any warnings.
fn format_buffer(
    src: &str,
    opts: &m1_fmt::FormatOptions,
    range: Option<(usize, usize)>,
) -> Result<(String, bool, Vec<m1_fmt::FormatWarning>), m1_fmt::FormatError> {
    match range {
        None => {
            let r = m1_fmt::format_str_with(src, opts)?;
            Ok((r.output, r.changed, r.warnings))
        }
        Some((a, b)) => match m1_fmt::format_range(src, a - 1, b - 1, opts)? {
            None => Ok((src.to_string(), false, Vec::new())),
            Some(rr) => {
                let spliced = splice_lines(src, rr.start_line, rr.end_line, &rr.output);
                let changed = spliced != src;
                Ok((spliced, changed, rr.warnings))
            }
        },
    }
}

/// The CLI flag overrides for [`config_resolve::resolve_opts`], taken from `args`.
fn cli_overrides(args: &Args) -> config_resolve::CliOverrides {
    config_resolve::CliOverrides {
        max_blank_lines: args.max_blank_lines,
        line_width: args.line_width,
    }
}

/// Print a unified diff between `original` and `formatted`.
fn print_diff(name: &str, original: &str, formatted: &str) {
    print!(
        "{}",
        m1_workspace::diff::unified_diff(name, original, formatted)
    );
}

/// Restore the default SIGPIPE disposition. Rust ignores SIGPIPE at startup, so
/// writing to a closed stdout/stderr (e.g. `m1-fmt … | head -1`) returns an
/// error that `print!`/`eprintln!` turn into a panic (exit 101). Resetting it to
/// `SIG_DFL` lets the kernel terminate us quietly on a broken pipe, the
/// conventional behaviour for a Unix filter (#epipe).
#[cfg(unix)]
fn reset_sigpipe() {
    // Safety: a bare `signal(2)` call with a constant handler; no Rust state is
    // touched and it runs before any output is produced.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
}

#[cfg(not(unix))]
fn reset_sigpipe() {}

fn main() {
    reset_sigpipe();
    let mut args = Args::parse();
    let mut any_changed = false;
    let mut any_error = false;
    // A buffer with syntax errors is passed through unchanged (data-preserving);
    // track it so `--check` doesn't report unparseable input as clean (#77).
    let mut any_syntax_error = false;

    // A lone `-` is the conventional spelling for "read standard input"; treat it
    // the same as passing no file arguments at all (matches rustfmt/black/gofmt).
    if args.files.len() == 1 && args.files[0].as_os_str() == "-" {
        args.files.clear();
    }

    let range = match args.range.as_deref() {
        None => None,
        Some(s) => match parse_range(s) {
            Some(r) => Some(r),
            None => {
                eprintln!(
                    "m1-fmt: invalid --range {s:?}; expected START:END (1-based, START<=END)"
                );
                process::exit(2);
            }
        },
    };

    // Directory arguments expand to every `.m1scr` beneath them via the shared
    // hardened walk (#106). Directories need an explicit output mode — bare
    // stdout would concatenate a whole tree — and exclude `--range`, which
    // targets a single buffer. An empty expansion is reported and fails the run
    // (a mistyped path must not look formatted), and must not fall through to
    // the stdin branch below.
    let had_dir = args.files.iter().any(|f| f.is_dir());
    if had_dir {
        if range.is_some() {
            eprintln!("m1-fmt: --range cannot be combined with a directory argument");
            process::exit(2);
        }
        if !(args.check || args.diff || args.in_place) {
            eprintln!("m1-fmt: a directory argument requires --check, --diff, or -i/--in-place");
            process::exit(2);
        }
        let mut expanded = Vec::new();
        for f in &args.files {
            if f.is_dir() {
                let found = m1_workspace::find_scripts(f);
                if found.is_empty() {
                    eprintln!("m1-fmt: no .m1scr files found under {}", f.display());
                    any_error = true;
                } else {
                    expanded.extend(found);
                }
            } else {
                expanded.push(f.clone());
            }
        }
        args.files = expanded;
    }

    if args.files.is_empty() && !had_dir {
        // Read from stdin. Discover .m1fmt.toml from the working directory.
        let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        let opts = config_resolve::resolve_opts(cli_overrides(&args), &cwd);
        // Read stdin as bytes and decode through the same tolerant workspace
        // decoder the file path uses (UTF-8 with a Windows-1252 fallback): a
        // piped `.m1scr` may carry CP1252 bytes (e.g. `°` = 0xB0), and a strict
        // `read_to_string` panicked on them (#67).
        let mut bytes = Vec::new();
        if let Err(e) = std::io::Read::read_to_end(&mut std::io::stdin(), &mut bytes) {
            eprintln!("m1-fmt: {}: {}", args.stdin_filename, e);
            process::exit(2);
        }
        // The tolerant decoder strips a leading UTF-8 BOM (m1-workspace#9). For a
        // formatter that round-trips its input we must NOT silently drop it: detect
        // it on the raw bytes and re-prepend it so `format_str_with` preserves it
        // (on both the normal path and the syntax-error passthrough) (#bom).
        let had_bom = bytes.starts_with(&[0xEF, 0xBB, 0xBF]);
        let decoded = m1_workspace::decode(bytes);
        let src = if had_bom {
            format!("\u{FEFF}{decoded}")
        } else {
            decoded
        };
        match format_buffer(&src, &opts, range).map(|(output, changed, warnings)| {
            m1_fmt::FormatResult {
                output,
                changed,
                warnings,
            }
        }) {
            Ok(result) => {
                for w in &result.warnings {
                    eprintln!("{}:{}: warning: {}", args.stdin_filename, w.line, w.message);
                }
                let serr = m1_fmt::syntax_error_count(&src);
                if serr > 0 {
                    eprintln!(
                        "m1-fmt: {}: {serr} syntax error(s); left unchanged",
                        args.stdin_filename
                    );
                    any_syntax_error = true;
                }
                if args.diff {
                    if result.changed {
                        print_diff(&args.stdin_filename, &src, &result.output);
                    }
                } else if !args.check {
                    print!("{}", result.output);
                }
                if result.changed {
                    any_changed = true;
                    if args.check {
                        eprintln!("{}: would reformat", args.stdin_filename);
                    }
                }
            }
            Err(e) => {
                eprintln!("m1-fmt: {}: {}", args.stdin_filename, e);
                any_error = true;
            }
        }
    } else {
        // `resolve_opts` walks the filesystem upward looking for `.m1fmt.toml` /
        // `m1-tools.toml`; the result is identical for every file sharing a parent
        // directory, so cache it per directory rather than re-walking the tree once
        // per file. With directory arguments (#106) a whole-workspace `--check .`
        // would otherwise re-discover config for every script (#108).
        let overrides = cli_overrides(&args);
        let mut opts_cache: HashMap<PathBuf, m1_fmt::FormatOptions> = HashMap::new();
        for path in &args.files {
            // Discover .m1fmt.toml upward from the file's own directory.
            let dir = path.parent().unwrap_or_else(|| std::path::Path::new("."));
            let opts = opts_cache
                .entry(dir.to_path_buf())
                .or_insert_with(|| config_resolve::resolve_opts(overrides, dir));
            // `.m1scr` may carry Windows-1252 bytes (e.g. `°` in a comment);
            // decode tolerantly via the shared workspace decoder so a valid
            // MoTeC script is not rejected by a strict UTF-8 read (#58). The
            // diff path reuses the same decoded text as the original. The
            // decoder strips a leading BOM (m1-workspace#9); read raw bytes so we
            // can detect and preserve it (a formatter must round-trip its input).
            let read = read_text_preserving_bom(path).map_err(m1_fmt::FormatError::IoError);
            let original = read.as_ref().ok().cloned();
            let outcome = read.and_then(|src| {
                format_buffer(&src, opts, range).map(|(output, changed, warnings)| {
                    m1_fmt::FormatResult {
                        output,
                        changed,
                        warnings,
                    }
                })
            });
            match outcome {
                Ok(result) => {
                    for w in &result.warnings {
                        eprintln!("{}:{}: warning: {}", path.display(), w.line, w.message);
                    }
                    if let Some(src) = &original {
                        let serr = m1_fmt::syntax_error_count(src);
                        if serr > 0 {
                            eprintln!(
                                "m1-fmt: {}: {serr} syntax error(s); left unchanged",
                                path.display()
                            );
                            any_syntax_error = true;
                        }
                    }
                    // The bare-stdout case must print unconditionally (mirroring
                    // the stdin branch); gating it on `changed` truncated an
                    // already-formatted file to empty (#59). `-i`/`--diff`/
                    // `--check` stay gated on `changed`.
                    if result.changed {
                        any_changed = true;
                        if args.check {
                            eprintln!("{}: would reformat", path.display());
                        } else if args.diff {
                            let orig = original.as_deref().unwrap_or("");
                            print_diff(&path.display().to_string(), orig, &result.output);
                        } else if args.in_place {
                            m1_workspace::atomic_write(path, result.output.as_bytes())
                                .unwrap_or_else(|e| {
                                    eprintln!("m1-fmt: {}: {}", path.display(), e);
                                });
                        } else {
                            print!("{}", result.output);
                        }
                    } else if !args.check && !args.diff && !args.in_place {
                        print!("{}", result.output);
                    }
                }
                Err(m1_fmt::FormatError::SyntaxErrors(diags)) => {
                    eprintln!(
                        "m1-fmt: skipping {}: {} syntax error(s)",
                        path.display(),
                        diags.len()
                    );
                    // A skipped, unparseable file is not a clean success either.
                    any_syntax_error = true;
                }
                Err(e) => {
                    eprintln!("m1-fmt: {}: {}", path.display(), e);
                    any_error = true;
                }
            }
        }
    }

    if any_error {
        // A per-file I/O error (unreadable/missing file). The batch already
        // continued past it; exit 1 — the same "has something to report" code
        // m1-lint/m1-typecheck use. Exit 2 is reserved for usage errors (a bad
        // flag/argument), handled inline above. Keeps the three CLIs consistent
        // (#16).
        process::exit(1);
    } else if any_syntax_error {
        // A file with syntax errors is left byte-for-byte unchanged (the original
        // is still emitted, data-preserving), but it is NOT a clean success in any
        // mode: fail loudly so a broken script can't slip through a pipeline,
        // format-on-save, or CI — not just under `--check`.
        process::exit(1);
    } else if args.check && any_changed {
        // --check: exit non-zero if any file would reformat.
        process::exit(1);
    }
}
