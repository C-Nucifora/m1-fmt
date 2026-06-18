use clap::Parser;
use rayon::prelude::*;
use std::collections::HashMap;
use std::fmt::Write as _;
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

    /// Indentation character: `tab` (default, per the manual) or `spaces`
    /// (overrides .m1fmt.toml / m1-tools.toml)
    #[arg(long, value_name = "tab|spaces")]
    indent_style: Option<String>,

    /// Opening-brace placement: `allman` (default, per the manual) or `kr`
    /// (overrides .m1fmt.toml / m1-tools.toml)
    #[arg(long, value_name = "allman|kr")]
    brace_style: Option<String>,

    /// Columns one indent level occupies (default 4; for tabs this is the
    /// assumed display width used by the wrapping math; overrides .m1fmt.toml)
    #[arg(long, value_name = "N")]
    indent_width: Option<usize>,

    /// Extra indent levels added to wrapped/continuation lines (default 1, per
    /// the manual; overrides .m1fmt.toml)
    #[arg(long, value_name = "N")]
    continuation_indent: Option<usize>,

    /// Align the `=` of contiguous runs of simple single-line assignments
    /// (opt-in; off by default per the manual). Overrides .m1fmt.toml /
    /// m1-tools.toml; pair with a wider --line-width so aligned rows fit.
    #[arg(long, overrides_with = "no_align_assignments")]
    align_assignments: bool,
    /// Turn assignment alignment off, overriding a config that enabled it.
    #[arg(long, hide = true, overrides_with = "align_assignments")]
    no_align_assignments: bool,

    /// Reflow over-width `//` comment lines onto continuation comments (opt-in;
    /// split-only — short lines are never joined). Overrides .m1fmt.toml /
    /// m1-tools.toml.
    #[arg(long, overrides_with = "no_reflow_comments")]
    reflow_comments: bool,
    /// Turn comment reflow off, overriding a config that enabled it.
    #[arg(long, hide = true, overrides_with = "reflow_comments")]
    no_reflow_comments: bool,

    /// End the file with one blank line instead of a bare newline (opt-in; the
    /// formatter pair of m1-lint L027). Overrides .m1fmt.toml / m1-tools.toml.
    #[arg(long, overrides_with = "no_final_blank_line")]
    final_blank_line: bool,
    /// Keep a bare trailing newline, overriding a config that enabled the blank.
    #[arg(long, hide = true, overrides_with = "final_blank_line")]
    no_final_blank_line: bool,

    /// Format only the given 1-based inclusive line range (`START:END`); the rest
    /// of the buffer is left byte-for-byte unchanged. The range is snapped outward
    /// to whole top-level statements. For LSP/editor format-on-selection.
    #[arg(long, value_name = "START:END")]
    range: Option<String>,

    /// Number of worker threads for parallel formatting (default: all available
    /// CPUs). `--jobs 1` forces fully serial processing; useful for reproducible
    /// profiling or constrained CI runners. Only affects directory / multi-file
    /// runs; stdin and single-file runs are inherently serial.
    #[arg(long, value_name = "N")]
    jobs: Option<usize>,
}

/// Read a file's text via the tolerant workspace decoder, preserving a leading
/// UTF-8 BOM that the decoder strips (m1-workspace#9). A formatter round-trips
/// its input, so a BOM-prefixed file must keep its BOM; the shared
/// [`m1_fmt::decode_preserving_bom`] helper restores it and `format_str_with`
/// then carries it through to the output.
fn read_text_preserving_bom(path: &std::path::Path) -> std::io::Result<String> {
    let bytes = std::fs::read(path)?;
    Ok(m1_fmt::decode_preserving_bom(bytes))
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
///
/// The two enum-valued flags (`--indent-style`, `--brace-style`) are parsed and
/// validated eagerly in [`parse_style_overrides`] before this is called, so the
/// already-typed values are passed in rather than re-parsed here.
fn cli_overrides(args: &Args, styles: StyleOverrides) -> config_resolve::CliOverrides {
    config_resolve::CliOverrides {
        max_blank_lines: args.max_blank_lines,
        line_width: args.line_width,
        indent_style: styles.indent_style,
        brace_style: styles.brace_style,
        indent_width: args.indent_width,
        continuation_indent: args.continuation_indent,
        align_assignments: tri_flag(args.align_assignments, args.no_align_assignments),
        reflow_comments: tri_flag(args.reflow_comments, args.no_reflow_comments),
        final_blank_line: tri_flag(args.final_blank_line, args.no_final_blank_line),
    }
}

/// Fold a `--flag` / `--no-flag` pair into a tri-state override. clap's
/// `overrides_with` guarantees at most one is set, so a `Some` always reflects
/// the last flag the user actually passed; neither given yields `None`, which
/// defers to the config files / default.
fn tri_flag(yes: bool, no: bool) -> Option<bool> {
    match (yes, no) {
        (true, _) => Some(true),
        (_, true) => Some(false),
        _ => None,
    }
}

/// The parsed-and-validated enum style flags. Produced once in `main` so an
/// invalid spelling is reported as a usage error before any formatting runs.
#[derive(Clone, Copy, Default)]
struct StyleOverrides {
    indent_style: Option<m1_fmt::IndentStyle>,
    brace_style: Option<m1_fmt::BraceStyle>,
}

/// Validate the `--indent-style` / `--brace-style` flag values eagerly. A bad
/// spelling prints a message listing the accepted values and exits 2 (the usage
/// error code, matching `--range` / `--jobs`) instead of failing later or being
/// silently ignored.
fn parse_style_overrides(args: &Args) -> StyleOverrides {
    let indent_style = args.indent_style.as_deref().map(|s| {
        m1_fmt::config::parse_indent_style(s).unwrap_or_else(|| {
            eprintln!(
                "m1-fmt: invalid --indent-style {s:?}; expected one of: tab, tabs, space, spaces"
            );
            process::exit(2);
        })
    });
    let brace_style = args.brace_style.as_deref().map(|s| {
        m1_fmt::config::parse_brace_style(s).unwrap_or_else(|| {
            eprintln!("m1-fmt: invalid --brace-style {s:?}; expected one of: allman, kr, k&r, knr");
            process::exit(2);
        })
    });
    StyleOverrides {
        indent_style,
        brace_style,
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

/// The captured result of formatting one file, produced inside the parallel loop
/// and replayed serially in input order so output stays deterministic (#109).
/// `stdout`/`stderr` hold the exact bytes the serial loop would have written to
/// those streams for this file; the flags feed the run-wide exit-code reduction.
#[derive(Default)]
struct FileOutcome {
    stdout: String,
    stderr: String,
    changed: bool,
    error: bool,
    syntax_error: bool,
}

/// Read, format, and (for `-i`) write a single file, capturing everything that
/// would be emitted to stdout/stderr rather than printing directly. Share-nothing:
/// it borrows only the pre-resolved `opts` for its directory and the immutable
/// `args`/`range`, so it is safe to run across files in parallel.
fn process_file(
    path: &std::path::Path,
    opts: &m1_fmt::FormatOptions,
    range: Option<(usize, usize)>,
    args: &Args,
) -> FileOutcome {
    let mut out = FileOutcome::default();
    // `.m1scr` may carry Windows-1252 bytes (e.g. `°` in a comment); decode
    // tolerantly via the shared workspace decoder so a valid MoTeC script is not
    // rejected by a strict UTF-8 read (#58). The diff path reuses the same decoded
    // text as the original. The decoder strips a leading BOM (m1-workspace#9); read
    // raw bytes so we can detect and preserve it (a formatter round-trips its input).
    let read = read_text_preserving_bom(path).map_err(m1_fmt::FormatError::IoError);
    let original = read.as_ref().ok().cloned();
    let outcome = read.and_then(|src| {
        format_buffer(&src, opts, range).map(|(output, changed, warnings)| m1_fmt::FormatResult {
            output,
            changed,
            warnings,
        })
    });
    match outcome {
        Ok(result) => {
            for w in &result.warnings {
                let _ = writeln!(
                    out.stderr,
                    "{}:{}: warning: {}",
                    path.display(),
                    w.line,
                    w.message
                );
            }
            if let Some(src) = &original {
                let serr = m1_fmt::syntax_error_count(src);
                if serr > 0 {
                    let _ = writeln!(
                        out.stderr,
                        "m1-fmt: {}: {serr} syntax error(s); left unchanged",
                        path.display()
                    );
                    out.syntax_error = true;
                }
            }
            // The bare-stdout case must print unconditionally (mirroring the stdin
            // branch); gating it on `changed` truncated an already-formatted file to
            // empty (#59). `-i`/`--diff`/`--check` stay gated on `changed`.
            if result.changed {
                out.changed = true;
                if args.check {
                    let _ = writeln!(out.stderr, "{}: would reformat", path.display());
                } else if args.diff {
                    let orig = original.as_deref().unwrap_or("");
                    out.stdout.push_str(&m1_workspace::diff::unified_diff(
                        &path.display().to_string(),
                        orig,
                        &result.output,
                    ));
                } else if args.in_place {
                    if let Err(e) = m1_workspace::atomic_write(path, result.output.as_bytes()) {
                        let _ = writeln!(out.stderr, "m1-fmt: {}: {}", path.display(), e);
                        // A failed in-place write (e.g. a read-only file) must fail
                        // the run: previously the error was printed but `-i` still
                        // exited 0, so CI / format-on-save silently swallowed it
                        // (#113).
                        out.error = true;
                    }
                } else {
                    out.stdout.push_str(&result.output);
                }
            } else if !args.check && !args.diff && !args.in_place {
                out.stdout.push_str(&result.output);
            }
        }
        Err(e) => {
            // The only error `format_buffer` produces is a file-read I/O error;
            // a syntax-error buffer is the data-preserving `Ok` passthrough above
            // (reported via the separate `syntax_error_count` pass).
            let _ = writeln!(out.stderr, "m1-fmt: {}: {}", path.display(), e);
            out.error = true;
        }
    }
    out
}

/// "1 file" / "N files" — pluralize a file count for the `--check`/`-i`
/// batch summary (#114).
fn files(n: usize) -> String {
    format!("{n} file{}", if n == 1 { "" } else { "s" })
}

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

    // Cap the rayon worker pool when --jobs is given (default: all CPUs). Must run
    // before any `par_iter` below initialises the global pool. `--jobs 1` makes the
    // multi-file loop fully serial, which keeps profiling reproducible and matches
    // the serial path byte-for-byte. build_global only fails if the pool is already
    // initialised, which cannot happen this early, so a usage error is reported.
    if let Some(n) = args.jobs {
        if n == 0 {
            eprintln!("m1-fmt: --jobs must be at least 1");
            process::exit(2);
        }
        if rayon::ThreadPoolBuilder::new()
            .num_threads(n)
            .build_global()
            .is_err()
        {
            eprintln!("m1-fmt: failed to configure {n} worker thread(s)");
            process::exit(2);
        }
    }

    // Validate the enum style flags eagerly: a bad `--indent-style` /
    // `--brace-style` spelling is a usage error (exit 2) reported before any
    // formatting runs, alongside the --jobs / --range checks above.
    let styles = parse_style_overrides(&args);

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
        let opts = config_resolve::resolve_opts(cli_overrides(&args, styles), &cwd);
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
        // formatter that round-trips its input we must NOT silently drop it: the
        // shared `decode_preserving_bom` helper re-prepends it so `format_str_with`
        // preserves it (on both the normal path and the syntax-error passthrough).
        let src = m1_fmt::decode_preserving_bom(bytes);
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
        //
        // Resolved serially up front so the parallel loop below borrows the cache
        // read-only and stays share-nothing (#109).
        let overrides = cli_overrides(&args, styles);
        let mut opts_cache: HashMap<PathBuf, m1_fmt::FormatOptions> = HashMap::new();
        for path in &args.files {
            let dir = path.parent().unwrap_or_else(|| std::path::Path::new("."));
            opts_cache
                .entry(dir.to_path_buf())
                .or_insert_with(|| config_resolve::resolve_opts(overrides, dir));
        }

        // Formatting is CPU-bound (parse + print) and files are independent, so
        // process them in parallel (#109). The loop body is share-nothing: it reads
        // the file, formats it against the pre-resolved per-directory options, and
        // returns a `FileOutcome` capturing everything to emit plus the three status
        // flags. Output stays deterministic — outcomes are produced in an indexed
        // Vec and replayed in input order below, never interleaved across files.
        // In-place writes happen inside the closure; they are per-file independent
        // and atomic via m1-workspace's `atomic_write`, so parallel `-i` is safe.
        let outcomes: Vec<FileOutcome> = args
            .files
            .par_iter()
            .map(|path| {
                let dir = path.parent().unwrap_or_else(|| std::path::Path::new("."));
                let opts = &opts_cache[dir];
                process_file(path, opts, range, &args)
            })
            .collect();

        // Replay in input order, aggregating status. stderr is emitted before
        // stdout for each file, matching the serial loop's per-file ordering.
        // Tally the per-file outcomes for the summary (#114); the three flags are
        // mutually exclusive per file (a syntax-error or I/O-error file is never
        // also counted as "changed").
        let total = args.files.len();
        let (mut n_changed, mut n_syntax, mut n_errored) = (0usize, 0usize, 0usize);
        for outcome in outcomes {
            any_changed |= outcome.changed;
            any_error |= outcome.error;
            any_syntax_error |= outcome.syntax_error;
            if outcome.error {
                n_errored += 1;
            } else if outcome.syntax_error {
                n_syntax += 1;
            } else if outcome.changed {
                n_changed += 1;
            }
            if !outcome.stderr.is_empty() {
                eprint!("{}", outcome.stderr);
            }
            if !outcome.stdout.is_empty() {
                print!("{}", outcome.stdout);
            }
        }

        // One summary line after the per-file output, for corpus-sized runs
        // (#114). Only in --check / -i (diff emits diffs; bare stdout is the
        // formatted text itself) and only with more than one file — a lone
        // file's own line already says everything. On stderr, so stdout piping
        // is unaffected.
        if total > 1 && (args.check || args.in_place) {
            let unchanged = total - n_changed - n_syntax - n_errored;
            let mut summary = if args.in_place {
                format!(
                    "reformatted {}, {} already formatted",
                    files(n_changed),
                    files(unchanged)
                )
            } else {
                format!(
                    "{} would be reformatted, {} already formatted",
                    files(n_changed),
                    files(unchanged)
                )
            };
            if n_syntax > 0 {
                summary.push_str(&format!(", {} skipped (syntax errors)", files(n_syntax)));
            }
            if n_errored > 0 {
                summary.push_str(&format!(", {} errored", files(n_errored)));
            }
            eprintln!("{summary}");
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
