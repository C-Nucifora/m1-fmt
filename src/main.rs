use clap::Parser;
use std::path::PathBuf;
use std::process;

#[derive(Parser, Debug)]
#[command(name = "m1-fmt", about = "Autoformatter for MoTeC M1 scripts")]
struct Args {
    /// Files to format (a lone `-`, or no files, reads from stdin)
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

/// Resolve [`FormatOptions`] for a file/stdin in `dir`, with precedence
/// CLI flag > `.m1fmt.toml` (discovered upward from `dir`) > built-in default.
fn resolve_opts(args: &Args, dir: &std::path::Path) -> m1_fmt::FormatOptions {
    let cfg = m1_fmt::config::discover(dir).unwrap_or_default();
    m1_fmt::FormatOptions {
        max_blank_lines: args.max_blank_lines.or(cfg.max_blank_lines).unwrap_or(2),
        line_width: args.line_width.or(cfg.max_line_length).unwrap_or(88),
    }
}

/// Print a minimal unified diff between `original` and `formatted`.
fn print_diff(name: &str, original: &str, formatted: &str) {
    println!("--- {} (original)", name);
    println!("+++ {} (formatted)", name);
    let orig_lines: Vec<&str> = original.lines().collect();
    let fmt_lines: Vec<&str> = formatted.lines().collect();
    let max = orig_lines.len().max(fmt_lines.len());
    for i in 0..max {
        match (orig_lines.get(i), fmt_lines.get(i)) {
            (Some(o), Some(f)) if o == f => {}
            (Some(o), Some(f)) => {
                println!("-{}", o);
                println!("+{}", f);
            }
            (Some(o), None) => println!("-{}", o),
            (None, Some(f)) => println!("+{}", f),
            (None, None) => {}
        }
    }
}

fn main() {
    let mut args = Args::parse();
    let mut any_changed = false;
    let mut any_error = false;

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

    if args.files.is_empty() {
        // Read from stdin. Discover .m1fmt.toml from the working directory.
        let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        let opts = resolve_opts(&args, &cwd);
        let mut src = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut src).unwrap();
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
        for path in &args.files {
            // Discover .m1fmt.toml upward from the file's own directory.
            let dir = path.parent().unwrap_or_else(|| std::path::Path::new("."));
            let opts = resolve_opts(&args, dir);
            let original = std::fs::read_to_string(path).ok();
            let read = match &original {
                Some(s) => Ok(s.clone()),
                None => std::fs::read_to_string(path).map_err(m1_fmt::FormatError::IoError),
            };
            let outcome = read.and_then(|src| {
                format_buffer(&src, &opts, range).map(|(output, changed, warnings)| {
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
                    if result.changed {
                        any_changed = true;
                        if args.check {
                            eprintln!("{}: would reformat", path.display());
                        } else if args.diff {
                            let orig = original.as_deref().unwrap_or("");
                            print_diff(&path.display().to_string(), orig, &result.output);
                        } else if args.in_place {
                            std::fs::write(path, &result.output).unwrap_or_else(|e| {
                                eprintln!("m1-fmt: {}: {}", path.display(), e);
                            });
                        } else {
                            print!("{}", result.output);
                        }
                    }
                }
                Err(m1_fmt::FormatError::SyntaxErrors(diags)) => {
                    eprintln!(
                        "m1-fmt: skipping {}: {} syntax error(s)",
                        path.display(),
                        diags.len()
                    );
                }
                Err(e) => {
                    eprintln!("m1-fmt: {}: {}", path.display(), e);
                    any_error = true;
                }
            }
        }
    }

    if any_error {
        process::exit(2);
    } else if any_changed && args.check {
        process::exit(1);
    }
}
