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
    child.stdin.take().unwrap().write_all(input).unwrap();
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
    child.stdin.take().unwrap().write_all(input).unwrap();
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
