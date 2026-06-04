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
    let mut child = Command::new(env!("CARGO_BIN_EXE_m1-fmt"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn m1-fmt");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
    )
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
    std::fs::write(&path, b"// yaw \xb0/s\n[\n]\n").unwrap();
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
