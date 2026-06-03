//! CLI behaviour tests that exercise the built binary end-to-end.

use std::io::Write;
use std::process::{Command, Stdio};

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
