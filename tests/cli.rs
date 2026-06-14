//! CLI behaviour tests that exercise the built binary end-to-end.

use std::io::Write;
use std::process::{Command, Stdio};

fn run_with_file(args: &[&str]) -> (Vec<u8>, String, Option<i32>) {
    let out = Command::new(env!("CARGO_BIN_EXE_m1-fmt"))
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run m1-fmt");
    (
        out.stdout,
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
    )
}

fn run_with_stdin(args: &[&str], input: &str) -> (String, String, Option<i32>) {
    run_with_stdin_bytes(args, input.as_bytes())
}

fn run_with_stdin_bytes(args: &[&str], input: &[u8]) -> (String, String, Option<i32>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_m1-fmt"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn m1-fmt");
    {
        // Drop stdin after writing so the child sees EOF. A CLI that rejects its
        // arguments (e.g. an invalid `--range`) exits *before* reading stdin and
        // closes the pipe, so a BrokenPipe here is expected, not a failure —
        // racing the child's exit must not make this harness flaky.
        let mut stdin = child.stdin.take().unwrap();
        if let Err(e) = stdin.write_all(input) {
            assert_eq!(
                e.kind(),
                std::io::ErrorKind::BrokenPipe,
                "writing to m1-fmt stdin failed: {e}"
            );
        }
    }
    let out = child.wait_with_output().unwrap();
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
    )
}

#[test]
fn version_flag_prints_version() {
    // #80: m1-fmt should report its version like the other CLIs.
    let (stdout, _stderr, code) = run_with_stdin(&["--version"], "");
    assert_eq!(code, Some(0));
    assert!(
        stdout.contains(env!("CARGO_PKG_VERSION")),
        "stdout should contain the version: {stdout}"
    );
}

#[test]
fn default_mode_fails_on_syntax_errors() {
    // Syntax errors are a failed output in *every* mode, not just --check: a
    // broken script left byte-for-byte unchanged is not a success. The original
    // content is still emitted (data-preserving), but the exit code signals the
    // failure so it can't slip through a pipeline or a format-on-save.
    let (stdout, stderr, code) = run_with_stdin(&["-"], "x = 1\n");
    assert_ne!(
        code,
        Some(0),
        "default-mode format of unparseable input must exit non-zero; stderr: {stderr}"
    );
    assert_eq!(stdout, "x = 1\n", "the original is still emitted unchanged");
    assert!(
        stderr.to_lowercase().contains("syntax"),
        "stderr should mention syntax errors: {stderr}"
    );
}

#[test]
fn missing_file_exits_one_not_two() {
    // CLI-convention consistency (#16): a per-file I/O error (unreadable/missing
    // file) exits 1, matching m1-lint/m1-typecheck. Exit 2 is reserved for usage
    // errors (a bad flag/argument). The batch still continues past the bad file.
    let (_out, stderr, code) = run_with_file(&["/tmp/m1fmt_definitely_missing_xyz.m1scr"]);
    assert_eq!(
        code,
        Some(1),
        "a missing input file should exit 1, not 2; stderr: {stderr}"
    );
}

#[test]
fn bad_range_argument_is_a_usage_error_exit_two() {
    // A malformed argument is a usage error -> exit 2 (distinct from a findings or
    // I/O exit of 1).
    let (_out, _stderr, code) = run_with_stdin(&["--range", "notarange", "-"], "x = 1;\n");
    assert_eq!(code, Some(2));
}

#[test]
fn check_flags_unparseable_input() {
    // #77: m1-fmt leaves a syntax-error buffer byte-for-byte unchanged (safe),
    // but `--check` must not then report it clean with exit 0 — that hides broken
    // files in CI. It should print a note and exit non-zero.
    let (_out, stderr, code) = run_with_stdin(&["--check", "-"], "local <Integer> = 1;\n");
    assert_ne!(
        code,
        Some(0),
        "--check on unparseable input must exit non-zero; stderr: {stderr}"
    );
    assert!(
        stderr.to_lowercase().contains("syntax"),
        "stderr should mention syntax errors: {stderr}"
    );
}

#[test]
fn dash_argument_reads_stdin() {
    // `-` is the conventional spelling for "read standard input".
    let (stdout, stderr, code) = run_with_stdin(&["-"], "local x=1;\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "local x = 1;\n");
}

/// #59: `m1-fmt <file>` (file-arg → stdout) must echo the formatted buffer even
/// when the file is already canonically formatted. The bare-stdout print used to
/// be gated on `result.changed`, so an already-clean file printed zero bytes —
/// `m1-fmt clean.m1scr > out` then truncated it to empty (data loss). The output
/// of the file-arg path must equal both the source and the stdin path.
#[test]
fn already_formatted_file_arg_prints_content() {
    let canonical = "if (A > B)\n{\n\tValue = 1;\n}\n";
    let dir = std::env::temp_dir();
    let path = dir.join("m1fmt_canon_test.m1scr");
    std::fs::write(&path, canonical).unwrap();

    let (file_stdout, file_stderr, file_code) = run_with_file(&[path.to_str().unwrap()]);
    assert_eq!(file_code, Some(0), "stderr: {file_stderr}");
    assert_eq!(
        file_stdout,
        canonical.as_bytes(),
        "file-arg stdout must echo the already-formatted content byte-for-byte"
    );

    // And it must match the stdin path exactly.
    let (stdin_stdout, _, stdin_code) = run_with_stdin(&[], canonical);
    assert_eq!(stdin_code, Some(0));
    assert_eq!(
        String::from_utf8_lossy(&file_stdout),
        stdin_stdout,
        "file-arg stdout must equal stdin stdout"
    );

    let _ = std::fs::remove_file(&path);
}

/// #58: a valid MoTeC `.m1scr` may contain Windows-1252 bytes (e.g. a degree
/// sign `0xB0` = `°` in a comment). A strict UTF-8 read rejected these with
/// "stream did not contain valid UTF-8" in every mode. The read must route
/// through the shared tolerant decoder (UTF-8 with a Windows-1252 fallback)
/// so fmt processes such files instead of erroring.
#[test]
fn windows1252_byte_is_decoded_not_rejected() {
    let dir = std::env::temp_dir();
    let path = dir.join("m1fmt_w1252_test.m1scr");
    // `\xb0` is the Windows-1252 (and Latin-1) encoding of `°` in a comment.
    // The body is valid M1 so the test isolates the decode behaviour (a syntax
    // error would now correctly fail the format in every mode).
    std::fs::write(&path, b"// yaw \xb0/s\nx = 1;\n").unwrap();
    let arg = path.to_str().unwrap();

    // --check must not emit the UTF-8 decode error.
    let (_, check_stderr, check_code) = run_with_file(&["--check", arg]);
    assert!(
        !check_stderr.contains("valid UTF-8"),
        "--check must not reject Windows-1252 input; stderr: {check_stderr}"
    );
    assert_ne!(check_code, Some(2), "stderr: {check_stderr}");

    // Format (file-arg → stdout) must also succeed and emit the decoded `°`.
    let (fmt_stdout, fmt_stderr, fmt_code) = run_with_file(&[arg]);
    assert!(
        !fmt_stderr.contains("valid UTF-8"),
        "format must not reject Windows-1252 input; stderr: {fmt_stderr}"
    );
    assert_eq!(fmt_code, Some(0), "stderr: {fmt_stderr}");
    assert!(
        String::from_utf8_lossy(&fmt_stdout).contains('°'),
        "the decoded degree sign must survive formatting; stdout: {:?}",
        String::from_utf8_lossy(&fmt_stdout)
    );

    let _ = std::fs::remove_file(&path);
}

/// #68: the in-place rewrite must be atomic — write to a temp file in the same
/// directory then rename over the target — so an interruption can never leave the
/// source truncated. A successful `-i` must produce the formatted content and
/// leave no temp file behind in the directory.
#[test]
fn in_place_write_is_atomic_and_leaves_no_temp_file() {
    let dir = std::env::temp_dir().join(format!("m1fmt_atomic_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("script.m1scr");
    std::fs::write(&path, "local x=1;\n").unwrap();

    let (_, stderr, code) = run_with_file(&["-i", path.to_str().unwrap()]);
    assert_eq!(code, Some(0), "stderr: {stderr}");

    // Content was rewritten in place.
    let written = std::fs::read_to_string(&path).unwrap();
    assert_eq!(written, "local x = 1;\n");

    // No temp/sidecar files were left in the directory — only the target remains.
    let remaining: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(
        remaining,
        vec![std::ffi::OsString::from("script.m1scr")],
        "atomic write left a stray temp file: {remaining:?}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// #113: a failed in-place write must fail the run. Regression: `-i` printed the
/// I/O error to stderr but still exited 0, so a read-only file (or any write
/// failure) was silently swallowed by CI and format-on-save. We force the failure
/// by making the *containing directory* read-only — `atomic_write` writes a temp
/// file in that directory and renames it over the target, so a non-writable dir
/// makes the write fail deterministically. Unix-only: directory write permissions
/// have no equivalent meaning on Windows.
#[cfg(unix)]
#[test]
fn in_place_write_failure_exits_nonzero() {
    use std::os::unix::fs::PermissionsExt;

    let dir = std::env::temp_dir().join(format!("m1fmt_ro_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("script.m1scr");
    // Unformatted, so the file *would* be rewritten — exercising the write path.
    std::fs::write(&path, "local x=1;\n").unwrap();

    // Make the directory read-only so the atomic temp-file create/rename fails.
    let original = std::fs::metadata(&dir).unwrap().permissions();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();

    let (_, stderr, code) = run_with_file(&["-i", path.to_str().unwrap()]);

    // Restore write permission before any assertion so cleanup always succeeds.
    std::fs::set_permissions(&dir, original).unwrap();

    assert_eq!(
        code,
        Some(1),
        "a failed in-place write must exit 1, not 0 (#113); stderr: {stderr}"
    );
    assert!(
        stderr.contains("script.m1scr"),
        "the failing path must be reported on stderr; stderr: {stderr}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// #67: the stdin path must decode non-UTF-8 (Windows-1252) bytes tolerantly,
/// the same way the file path does, instead of panicking on the strict
/// `read_to_string().unwrap()`. A `°` is the single CP1252 byte `0xB0`, which is
/// not valid UTF-8; piping such a valid M1 script must format cleanly (exit 0)
/// and surface the decoded `°`, not crash with "stream did not contain valid
/// UTF-8" (exit 101).
#[test]
fn stdin_windows1252_byte_is_decoded_not_panicked() {
    let input: &[u8] = b"local x = 1; // \xb0/s\n";
    let (stdout, stderr, code) = run_with_stdin_bytes(&[], input);
    assert!(
        !stderr.contains("valid UTF-8") && !stderr.contains("panicked"),
        "stdin must not panic on Windows-1252 input; stderr: {stderr}"
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(
        stdout.contains('°'),
        "the decoded degree sign must survive formatting via stdin; stdout: {stdout:?}"
    );
}

/// A leading UTF-8 BOM must survive formatting via stdin. The tolerant decoder
/// strips a BOM (m1-workspace#9), but a formatter round-trips its input — a
/// BOM-prefixed file must keep its BOM while the body is still reformatted.
fn run_with_stdin_bytes_raw(args: &[&str], input: &[u8]) -> (Vec<u8>, String, Option<i32>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_m1-fmt"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn m1-fmt");
    {
        // Drop stdin after writing so the child sees EOF. A CLI that rejects its
        // arguments (e.g. an invalid `--range`) exits *before* reading stdin and
        // closes the pipe, so a BrokenPipe here is expected, not a failure —
        // racing the child's exit must not make this harness flaky.
        let mut stdin = child.stdin.take().unwrap();
        if let Err(e) = stdin.write_all(input) {
            assert_eq!(
                e.kind(),
                std::io::ErrorKind::BrokenPipe,
                "writing to m1-fmt stdin failed: {e}"
            );
        }
    }
    let out = child.wait_with_output().unwrap();
    (
        out.stdout,
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
    )
}

#[test]
fn stdin_bom_is_preserved_and_body_formatted() {
    // BOM + valid-but-unformatted code: the BOM stays, the body is reformatted.
    let input: &[u8] = b"\xef\xbb\xbflocal x=1;\n";
    let (stdout, stderr, code) = run_with_stdin_bytes_raw(&[], input);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(
        stdout, b"\xef\xbb\xbflocal x = 1;\n",
        "BOM must be preserved while the body is reformatted; got {stdout:?}"
    );
}

#[test]
fn stdin_bom_with_syntax_error_is_byte_for_byte_unchanged() {
    // BOM + syntax error: the documented passthrough must keep the buffer
    // byte-for-byte, BOM included (the BOM is not the parse error).
    let input: &[u8] = b"\xef\xbb\xbfx = ;\n";
    let (stdout, _stderr, code) = run_with_stdin_bytes_raw(&[], input);
    // Syntax errors exit non-zero in every mode (#77), but the bytes are intact.
    assert_ne!(code, Some(0));
    assert_eq!(
        stdout, input,
        "BOM + syntax error must pass through byte-for-byte; got {stdout:?}"
    );
}

/// A truncated pipe (`m1-fmt … | head -1`) must terminate quietly rather than
/// panic on the broken write (Rust's default ignore-then-panic -> exit 101). On
/// Unix we reset SIGPIPE to default, so the process dies on the signal (exit 141)
/// instead. The key assertion is "not a panic": stderr must be free of a panic
/// message and the exit code must not be 101.
#[cfg(unix)]
#[test]
fn broken_pipe_does_not_panic() {
    use std::process::Command;
    // Generate lots of output, pipe it through `head -1`, and observe m1-fmt.
    // A long, repeated statement guarantees many output lines.
    let line = "local reallyQuiteLongVariableName = anotherQuiteLongExpressionValue;\n";
    let big = line.repeat(50_000);

    let mut child = Command::new(env!("CARGO_BIN_EXE_m1-fmt"))
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn m1-fmt");
    // Feed stdin on a thread so a full pipe can't deadlock us.
    let mut stdin = child.stdin.take().unwrap();
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(big.as_bytes());
    });
    // Read only the first line, then drop the reader to close the pipe.
    {
        use std::io::BufRead;
        let stdout = child.stdout.take().unwrap();
        let mut reader = std::io::BufReader::new(stdout);
        let mut first = String::new();
        let _ = reader.read_line(&mut first);
        // Dropping `reader` here closes the read end -> SIGPIPE on next write.
    }
    let out = child.wait_with_output().unwrap();
    let _ = writer.join();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("panicked"),
        "broken pipe must not panic; stderr: {stderr}"
    );
    assert_ne!(
        out.status.code(),
        Some(101),
        "broken pipe must not exit 101 (panic); stderr: {stderr}"
    );
}

/// Create a uniquely-named temp directory tree for the directory-argument
/// tests (#106) and return its path.
fn temp_tree(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("m1fmt-dir-{}-{}", std::process::id(), name));
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    dir
}

const UNFORMATTED: &str = "if (A > B) {\n  Value = 1;\n}\n";
const CANONICAL: &str = "if (A > B)\n{\n\tValue = 1;\n}\n";

#[test]
fn directory_argument_formats_in_place_recursively() {
    // #106: a directory argument expands to every `.m1scr` beneath it.
    let dir = temp_tree("inplace");
    std::fs::write(dir.join("a.m1scr"), UNFORMATTED).unwrap();
    std::fs::write(dir.join("sub").join("b.m1scr"), UNFORMATTED).unwrap();

    let (_, stderr, code) = run_with_file(&["-i", dir.to_str().unwrap()]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(
        std::fs::read_to_string(dir.join("a.m1scr")).unwrap(),
        CANONICAL,
        "top-level script must be reformatted"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("sub").join("b.m1scr")).unwrap(),
        CANONICAL,
        "nested script must be reformatted"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn directory_argument_check_reports_each_file() {
    let dir = temp_tree("check");
    std::fs::write(dir.join("a.m1scr"), UNFORMATTED).unwrap();
    std::fs::write(dir.join("sub").join("b.m1scr"), CANONICAL).unwrap();

    let (_, stderr, code) = run_with_file(&["--check", dir.to_str().unwrap()]);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(code, Some(1), "one dirty file => exit 1. stderr: {stderr}");
    assert!(
        stderr.contains("a.m1scr") && stderr.contains("would reformat"),
        "dirty file reported. stderr: {stderr}"
    );
    assert!(
        !stderr.contains("b.m1scr"),
        "clean file must not be reported. stderr: {stderr}"
    );
}

#[test]
fn check_prints_a_summary_count() {
    // #114: --check ends with one stderr summary line counting the outcomes, so
    // a corpus-sized run is not just a wall of per-file lines with no total.
    let dir = temp_tree("checksummary");
    std::fs::write(dir.join("a.m1scr"), UNFORMATTED).unwrap();
    std::fs::write(dir.join("sub").join("b.m1scr"), CANONICAL).unwrap();

    let (_, stderr, code) = run_with_file(&["--check", dir.to_str().unwrap()]);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(code, Some(1), "stderr: {stderr}");
    assert!(
        stderr.contains("1 file would be reformatted"),
        "summary must count files that would reformat. stderr: {stderr}"
    );
    assert!(
        stderr.contains("1 file already formatted"),
        "summary must count files left unchanged. stderr: {stderr}"
    );
}

#[test]
fn in_place_prints_a_reformatted_summary() {
    // #114: -i prints a summary of how many files it actually rewrote.
    let dir = temp_tree("isummary");
    std::fs::write(dir.join("a.m1scr"), UNFORMATTED).unwrap();
    std::fs::write(dir.join("sub").join("b.m1scr"), CANONICAL).unwrap();

    let (_, stderr, code) = run_with_file(&["-i", dir.to_str().unwrap()]);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(
        stderr.contains("reformatted 1 file"),
        "in-place summary must report the reformatted count. stderr: {stderr}"
    );
}

#[test]
fn single_file_check_prints_no_summary() {
    // The summary is for corpus/multi-file runs; a single file's own line is
    // enough, so don't tack a redundant "1 file would be reformatted" onto it.
    let dir = temp_tree("nosummary");
    let f = dir.join("a.m1scr");
    std::fs::write(&f, UNFORMATTED).unwrap();
    let (_, stderr, code) = run_with_file(&["--check", f.to_str().unwrap()]);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(code, Some(1), "stderr: {stderr}");
    assert!(
        !stderr.contains("would be reformatted"),
        "single-file --check must not print the batch summary. stderr: {stderr}"
    );
}

#[test]
fn directory_argument_requires_an_output_mode() {
    // Bare-stdout mode would concatenate a whole tree to stdout — almost
    // certainly a mistake. Directories require -i, --check, or --diff.
    let dir = temp_tree("nomode");
    std::fs::write(dir.join("a.m1scr"), CANONICAL).unwrap();
    let (_, stderr, code) = run_with_file(&[dir.to_str().unwrap()]);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(code, Some(2), "usage error. stderr: {stderr}");
    assert!(
        stderr.contains("--check") && stderr.contains("-i"),
        "the error must name the valid modes. stderr: {stderr}"
    );
}

#[test]
fn directory_argument_rejects_range() {
    // --range targets one buffer; combined with a directory it is ambiguous.
    let dir = temp_tree("range");
    std::fs::write(dir.join("a.m1scr"), CANONICAL).unwrap();
    let (_, stderr, code) = run_with_file(&["--check", "--range", "1:2", dir.to_str().unwrap()]);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(code, Some(2), "usage error. stderr: {stderr}");
    assert!(
        stderr.contains("--range"),
        "the error must mention --range. stderr: {stderr}"
    );
}

#[test]
fn empty_directory_reports_and_fails() {
    // A directory yielding zero scripts must say so and fail, not pass clean —
    // a mistyped path must not look formatted.
    let dir = temp_tree("empty");
    let (_, stderr, code) = run_with_file(&["--check", dir.to_str().unwrap()]);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(code, Some(1), "stderr: {stderr}");
    assert!(
        stderr.contains("no .m1scr"),
        "empty directory must be reported. stderr: {stderr}"
    );
}

/// The two manual-mandated style knobs (tab indentation, Allman braces) and the
/// width knobs must be settable as one-off CLI flags, not only via a committed
/// `.m1fmt.toml`. `--brace-style kr` over stdin must override the Allman default
/// so an ad-hoc / CI / format-on-selection invocation can preview a house style
/// without writing a config file to disk.
#[test]
fn brace_style_flag_overrides_default_over_stdin() {
    let input = "if (A > B)\n{\n\tValue = 1;\n}\n";

    // Default (Allman): brace stays on its own line.
    let (default_out, default_err, default_code) = run_with_stdin(&[], input);
    assert_eq!(default_code, Some(0), "stderr: {default_err}");
    assert_eq!(
        default_out, input,
        "the default Allman output must keep the brace on its own line"
    );

    // --brace-style kr: opening brace joins the keyword line.
    let (kr_out, kr_err, kr_code) = run_with_stdin(&["--brace-style", "kr"], input);
    assert_eq!(kr_code, Some(0), "stderr: {kr_err}");
    assert_ne!(
        kr_out, default_out,
        "--brace-style kr must change the output vs the Allman default"
    );
    assert!(
        kr_out.contains("if (A > B) {"),
        "K&R must put the opening brace on the keyword line; got: {kr_out:?}"
    );
}

/// `--indent-style spaces` must override the tab default for a one-off run.
#[test]
fn indent_style_flag_overrides_default_over_stdin() {
    let input = "if (A > B)\n{\n\tValue = 1;\n}\n";
    let (out, err, code) = run_with_stdin(&["--indent-style", "spaces"], input);
    assert_eq!(code, Some(0), "stderr: {err}");
    assert!(
        out.contains("\n    Value = 1;"),
        "--indent-style spaces must indent with spaces, not a tab; got: {out:?}"
    );
    assert!(
        !out.contains("\n\tValue = 1;"),
        "the tab default must be overridden; got: {out:?}"
    );
}

/// An invalid `--brace-style` value is a usage error: exit 2 with a message that
/// lists the accepted spellings (matching the existing --jobs/--range pattern).
#[test]
fn invalid_brace_style_flag_is_a_usage_error_exit_two() {
    let (_out, stderr, code) = run_with_stdin(&["--brace-style", "egyptian", "-"], "x = 1;\n");
    assert_eq!(code, Some(2), "stderr: {stderr}");
    assert!(
        stderr.contains("brace-style") && stderr.contains("allman") && stderr.contains("kr"),
        "the error must name the flag and list accepted values; stderr: {stderr}"
    );
}

/// An invalid `--indent-style` value is a usage error: exit 2 with a helpful
/// message listing the accepted spellings.
#[test]
fn invalid_indent_style_flag_is_a_usage_error_exit_two() {
    let (_out, stderr, code) = run_with_stdin(&["--indent-style", "tabz", "-"], "x = 1;\n");
    assert_eq!(code, Some(2), "stderr: {stderr}");
    assert!(
        stderr.contains("indent-style") && stderr.contains("tab") && stderr.contains("spaces"),
        "the error must name the flag and list accepted values; stderr: {stderr}"
    );
}

#[test]
fn multi_file_diff_output_is_ordered_and_deterministic() {
    // #109: files are formatted in parallel, but the captured per-file output must
    // be replayed in input order and never interleaved. Run many files (more than
    // the worker-thread count) several times and assert byte-identical, in-order
    // output every run.
    let dir = temp_tree("paralleldiff");
    let mut paths = Vec::new();
    for i in 0..12 {
        let p = dir.join(format!("f{i:02}.m1scr"));
        std::fs::write(&p, UNFORMATTED).unwrap();
        paths.push(p.to_str().unwrap().to_string());
    }
    let mut args: Vec<&str> = vec!["--diff"];
    args.extend(paths.iter().map(|s| s.as_str()));

    let (first, _, code) = run_with_file(&args);
    assert_eq!(code, Some(0), "--diff exits 0 even when files differ");
    let first = String::from_utf8(first).unwrap();

    // Each file's diff header names the file; the headers must appear in input
    // order f00, f01, ... f11.
    let order: Vec<usize> = first
        .lines()
        .filter_map(|l| l.strip_prefix("--- "))
        // header is e.g. "--- /tmp/.../f03.m1scr (original)"; take the path word.
        .filter_map(|rest| rest.split_whitespace().next())
        .filter_map(|name| {
            name.rsplit('/')
                .next()
                .and_then(|f| f.strip_prefix('f'))
                .and_then(|f| f.strip_suffix(".m1scr"))
                .and_then(|n| n.parse::<usize>().ok())
        })
        .collect();
    assert_eq!(
        order,
        (0..12).collect::<Vec<_>>(),
        "diff blocks must be emitted in input order; got {order:?}"
    );

    // Determinism: repeated runs produce identical output.
    for _ in 0..4 {
        let (again, _, _) = run_with_file(&args);
        assert_eq!(
            String::from_utf8(again).unwrap(),
            first,
            "parallel --diff output must be deterministic across runs"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}
