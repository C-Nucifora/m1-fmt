use clap::Parser;
use std::path::PathBuf;
use std::process;

#[derive(Parser, Debug)]
#[command(name = "m1-fmt", version, about = "Autoformatter for MoTeC M1 scripts")]
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

/// Resolve [`FormatOptions`] for a file/stdin in `dir`, layering lowest-first:
/// built-in defaults → the unified `m1-tools.toml` `[format]` section → the
/// tool-specific `.m1fmt.toml` (overrides the unified file) → CLI flags. Both
/// config files are discovered by walking up from `dir`.
fn resolve_opts(args: &Args, dir: &std::path::Path) -> m1_fmt::FormatOptions {
    let mut o = m1_fmt::FormatOptions::default();

    // Layer 1: the unified m1-tools.toml [format] section (lowest config layer).
    if let Some(tc) = m1_workspace::config::M1ToolsConfig::discover(dir) {
        let f = tc.format;
        if let Some(n) = f.line_width {
            o.line_width = n;
        }
        if let Some(n) = f.max_blank_lines {
            o.max_blank_lines = n;
        }
        if let Some(n) = f.indent_width {
            o.indent_width = n;
        }
        if let Some(s) = f
            .indent_style
            .as_deref()
            .and_then(m1_fmt::config::parse_indent_style)
        {
            o.indent_style = s;
        }
        if let Some(s) = f
            .brace_style
            .as_deref()
            .and_then(m1_fmt::config::parse_brace_style)
        {
            o.brace_style = s;
        }
    }

    // Layer 2: the tool-specific .m1fmt.toml overrides the unified file.
    if let Some(cfg) = m1_fmt::config::discover(dir) {
        if let Some(n) = cfg.max_line_length {
            o.line_width = n;
        }
        if let Some(n) = cfg.max_blank_lines {
            o.max_blank_lines = n;
        }
        if let Some(n) = cfg.indent_width {
            o.indent_width = n;
        }
        if let Some(s) = cfg.indent_style {
            o.indent_style = s;
        }
        if let Some(s) = cfg.brace_style {
            o.brace_style = s;
        }
        if let Some(n) = cfg.continuation_indent {
            o.continuation_indent = n;
        }
    }

    // Layer 3: explicit CLI flags win over everything.
    if let Some(n) = args.max_blank_lines {
        o.max_blank_lines = n;
    }
    if let Some(n) = args.line_width {
        o.line_width = n;
    }
    o
}

/// Print a unified diff between `original` and `formatted`.
fn print_diff(name: &str, original: &str, formatted: &str) {
    print!("{}", unified_diff(name, original, formatted));
}

/// One step of the line-level edit script between two files.
#[derive(Clone, Copy, PartialEq)]
enum Op {
    Equal,
    Delete,
    Insert,
}

/// Build a real unified diff between `original` and `formatted`, grouping edits
/// into hunks with surrounding context — not a positional line-pairing, which
/// reports every line after an insertion as changed (#79). Returns the empty
/// string when the inputs are identical (no `--- / +++` header, no hunks).
fn unified_diff(name: &str, original: &str, formatted: &str) -> String {
    const CONTEXT: usize = 3;
    let a: Vec<&str> = original.lines().collect();
    let b: Vec<&str> = formatted.lines().collect();
    let script = edit_script(&a, &b);
    if script.iter().all(|(op, _, _)| *op == Op::Equal) {
        return String::new();
    }

    // Group consecutive non-equal ops (plus up to CONTEXT equal lines on each
    // side) into hunks.
    let mut out = String::new();
    out.push_str(&format!("--- {name} (original)\n"));
    out.push_str(&format!("+++ {name} (formatted)\n"));

    let mut i = 0;
    while i < script.len() {
        if script[i].0 == Op::Equal {
            i += 1;
            continue;
        }
        // Start of a change run: back up CONTEXT equal lines for leading context.
        let mut start = i;
        let mut ctx = 0;
        while start > 0 && script[start - 1].0 == Op::Equal && ctx < CONTEXT {
            start -= 1;
            ctx += 1;
        }
        // Extend to the end of the change run, absorbing gaps of <= 2*CONTEXT
        // equal lines so nearby changes share one hunk, then add trailing context.
        let mut end = i;
        while end < script.len() {
            if script[end].0 != Op::Equal {
                end += 1;
                continue;
            }
            // Count the run of equal lines starting at `end`.
            let mut run = end;
            while run < script.len() && script[run].0 == Op::Equal {
                run += 1;
            }
            let run_len = run - end;
            let more_changes = run < script.len();
            if more_changes && run_len <= 2 * CONTEXT {
                end = run; // bridge the gap into the same hunk
            } else {
                end += run_len.min(CONTEXT); // trailing context, then close
                break;
            }
        }

        // Hunk header line ranges (1-based; 0,0 when a side is empty).
        let (mut a_start, mut b_start) = (0usize, 0usize);
        let (mut a_count, mut b_count) = (0usize, 0usize);
        for (op, ai, bi) in &script[start..end] {
            match op {
                Op::Equal => {
                    if a_count == 0 {
                        a_start = ai + 1;
                    }
                    if b_count == 0 {
                        b_start = bi + 1;
                    }
                    a_count += 1;
                    b_count += 1;
                }
                Op::Delete => {
                    if a_count == 0 {
                        a_start = ai + 1;
                    }
                    a_count += 1;
                }
                Op::Insert => {
                    if b_count == 0 {
                        b_start = bi + 1;
                    }
                    b_count += 1;
                }
            }
        }
        out.push_str(&format!(
            "@@ -{},{} +{},{} @@\n",
            a_start, a_count, b_start, b_count
        ));
        for (op, ai, bi) in &script[start..end] {
            match op {
                Op::Equal => out.push_str(&format!(" {}\n", a[*ai])),
                Op::Delete => out.push_str(&format!("-{}\n", a[*ai])),
                Op::Insert => out.push_str(&format!("+{}\n", b[*bi])),
            }
        }
        i = end;
    }
    out
}

/// The line-level LCS edit script: a sequence of `(Op, a_index, b_index)`. For
/// `Equal` both indices are meaningful; for `Delete` only `a_index`, for
/// `Insert` only `b_index` (the other is the last consumed index, unused).
fn edit_script(a: &[&str], b: &[&str]) -> Vec<(Op, usize, usize)> {
    let (n, m) = (a.len(), b.len());
    // LCS length DP table.
    let mut dp = vec![vec![0usize; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[i][j] = if a[i] == b[j] {
                dp[i + 1][j + 1] + 1
            } else {
                dp[i + 1][j].max(dp[i][j + 1])
            };
        }
    }
    // Walk the table to recover the edit script.
    let mut script = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if a[i] == b[j] {
            script.push((Op::Equal, i, j));
            i += 1;
            j += 1;
        } else if dp[i + 1][j] >= dp[i][j + 1] {
            script.push((Op::Delete, i, j));
            i += 1;
        } else {
            script.push((Op::Insert, i, j));
            j += 1;
        }
    }
    while i < n {
        script.push((Op::Delete, i, j));
        i += 1;
    }
    while j < m {
        script.push((Op::Insert, i, j));
        j += 1;
    }
    script
}

/// Write `bytes` to `path` atomically: write to a temp file in the same
/// directory, flush + fsync it, then rename it over the target. A crash, kill, or
/// I/O error can then only leave the original intact or fully replaced — never a
/// half-written or truncated source (#68). On error the temp file is removed.
fn atomic_write(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;

    // Same-directory temp so the final rename stays on one filesystem (a rename
    // across filesystems is not atomic and would fail). The pid + the file name
    // keep concurrent `m1-fmt` runs from colliding on the same temp path.
    let dir = path.parent().unwrap_or_else(|| std::path::Path::new("."));
    let file_name = path.file_name().map(|s| s.to_owned()).unwrap_or_default();
    let mut tmp_name = std::ffi::OsString::from(".");
    tmp_name.push(&file_name);
    tmp_name.push(format!(".{}.tmp", std::process::id()));
    let tmp = dir.join(tmp_name);

    // Scope the file handle so it is closed before the rename; clean up the temp
    // on any failure so a partial write never lingers.
    let write_result = (|| -> std::io::Result<()> {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.flush()?;
        f.sync_all()?;
        Ok(())
    })();
    if let Err(e) = write_result {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(())
}

fn main() {
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

    if args.files.is_empty() {
        // Read from stdin. Discover .m1fmt.toml from the working directory.
        let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        let opts = resolve_opts(&args, &cwd);
        // Read stdin as bytes and decode through the same tolerant workspace
        // decoder the file path uses (UTF-8 with a Windows-1252 fallback): a
        // piped `.m1scr` may carry CP1252 bytes (e.g. `°` = 0xB0), and a strict
        // `read_to_string` panicked on them (#67).
        let mut bytes = Vec::new();
        if let Err(e) = std::io::Read::read_to_end(&mut std::io::stdin(), &mut bytes) {
            eprintln!("m1-fmt: {}: {}", args.stdin_filename, e);
            process::exit(2);
        }
        let src = m1_workspace::decode(bytes);
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
        for path in &args.files {
            // Discover .m1fmt.toml upward from the file's own directory.
            let dir = path.parent().unwrap_or_else(|| std::path::Path::new("."));
            let opts = resolve_opts(&args, dir);
            // `.m1scr` may carry Windows-1252 bytes (e.g. `°` in a comment);
            // decode tolerantly via the shared workspace decoder so a valid
            // MoTeC script is not rejected by a strict UTF-8 read (#58). The
            // diff path reuses the same decoded text as the original.
            let read = m1_workspace::read_text(path).map_err(m1_fmt::FormatError::IoError);
            let original = read.as_ref().ok().cloned();
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
                            atomic_write(path, result.output.as_bytes()).unwrap_or_else(|e| {
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
        process::exit(2);
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

#[cfg(test)]
mod resolve_tests {
    use super::*;
    use clap::Parser;

    fn args(extra: &[&str]) -> Args {
        let mut v = vec!["m1-fmt"];
        v.extend_from_slice(extra);
        Args::parse_from(v)
    }

    #[test]
    fn unified_tools_toml_drives_brace_style() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("m1-tools.toml"),
            "[format]\nbrace_style = \"kr\"\nindent_style = \"spaces\"\nindent_width = 2\nline_width = 100\n",
        )
        .unwrap();
        let o = resolve_opts(&args(&[]), tmp.path());
        assert_eq!(o.brace_style, m1_fmt::BraceStyle::KAndR);
        assert_eq!(o.indent_style, m1_fmt::IndentStyle::Spaces);
        assert_eq!(o.indent_width, 2);
        assert_eq!(o.line_width, 100);
    }

    #[test]
    fn m1fmt_toml_overrides_unified_file() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("m1-tools.toml"),
            "[format]\nbrace_style = \"kr\"\n",
        )
        .unwrap();
        std::fs::write(tmp.path().join(".m1fmt.toml"), "brace_style = \"allman\"\n").unwrap();
        let o = resolve_opts(&args(&[]), tmp.path());
        assert_eq!(
            o.brace_style,
            m1_fmt::BraceStyle::Allman,
            ".m1fmt.toml wins"
        );
    }

    #[test]
    fn flag_overrides_both() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("m1-tools.toml"),
            "[format]\nline_width = 100\n",
        )
        .unwrap();
        let o = resolve_opts(&args(&["--line-width", "70"]), tmp.path());
        assert_eq!(o.line_width, 70);
    }
}

#[cfg(test)]
mod diff_tests {
    use super::*;

    #[test]
    fn inserted_line_keeps_following_lines_as_context() {
        // #79: a real unified diff. Inserting one line must not report every
        // following identical line as changed; they stay as ` ` context lines,
        // and a `@@` hunk header frames the change.
        let original = "a\nb\nc\n";
        let formatted = "a\nX\nb\nc\n";
        let d = unified_diff("demo.m1scr", original, formatted);
        assert!(d.contains("@@"), "expected a hunk header:\n{d}");
        assert!(d.contains("+X"), "expected the inserted line:\n{d}");
        // `b` and `c` are unchanged: they appear as context, never as -/+ pairs.
        assert!(d.contains(" b"), "b should be a context line:\n{d}");
        assert!(d.contains(" c"), "c should be a context line:\n{d}");
        assert!(!d.contains("-b"), "b must not be reported as deleted:\n{d}");
        assert!(!d.contains("-c"), "c must not be reported as deleted:\n{d}");
    }

    #[test]
    fn identical_inputs_produce_no_hunks() {
        let d = unified_diff("demo.m1scr", "a\nb\n", "a\nb\n");
        assert!(!d.contains("@@"), "no changes -> no hunks:\n{d}");
    }

    #[test]
    fn changed_line_shown_as_delete_then_insert() {
        let d = unified_diff("demo.m1scr", "a\nb\nc\n", "a\nB\nc\n");
        assert!(d.contains("-b"), "{d}");
        assert!(d.contains("+B"), "{d}");
        assert!(
            d.contains(" a") && d.contains(" c"),
            "context around change:\n{d}"
        );
    }
}
