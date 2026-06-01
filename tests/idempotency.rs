mod common;

#[test]
fn idempotency_corpus() {
    let scripts = common::corpus_scripts();
    if scripts.is_empty() {
        eprintln!("corpus absent; skipping idempotency test");
        return;
    }

    let mut failures = Vec::new();
    for (path, src) in &scripts {
        // Skip files with syntax errors
        let cst = m1_core::parse(src);
        if !cst.syntax_diagnostics().is_empty() {
            continue;
        }

        let once = match m1_fmt::format_str(src) {
            Ok(r) => r.output,
            Err(e) => {
                failures.push(format!("{}: format error: {}", path.display(), e));
                continue;
            }
        };
        let twice = match m1_fmt::format_str(&once) {
            Ok(r) => r.output,
            Err(e) => {
                failures.push(format!("{}: second format error: {}", path.display(), e));
                continue;
            }
        };
        if once != twice {
            failures.push(format!(
                "{}: not idempotent\n--- first ---\n{}\n--- second ---\n{}",
                path.display(),
                &once[..once.len().min(500)],
                &twice[..twice.len().min(500)],
            ));
        }
    }

    if !failures.is_empty() {
        panic!("idempotency failures:\n{}", failures.join("\n\n"));
    }
}

/// #19: stripping brace-adjacent blanks used to run after collapsing blank
/// runs, so a second pass could still change the output. A single pass must be
/// stable.
#[test]
fn brace_adjacent_blank_stripping_is_idempotent() {
    let src = "if (a) {\n\n\n\n    x = 1;\n}\n";
    let once = m1_fmt::format_str(src).unwrap().output;
    let twice = m1_fmt::format_str(&once).unwrap().output;
    assert_eq!(once, twice, "format must be idempotent in one pass");
    // The blanks immediately after `{` are gone.
    assert!(!once.contains("{\n\n"), "brace-adjacent blank not stripped");
}

/// #18: CRLF input must format correctly (brace-adjacent blanks stripped, no
/// spurious trailing newline) and round-trip back to CRLF.
#[test]
fn crlf_input_formats_and_preserves_crlf() {
    let lf = "if (a) {\n    x = 1;\n}\n";
    let crlf = lf.replace('\n', "\r\n");

    let out = m1_fmt::format_str(&crlf).unwrap().output;
    assert!(out.contains("\r\n"), "CRLF should be preserved on output");
    assert!(
        !out.ends_with("\r\n\r\n"),
        "no spurious trailing blank line on CRLF"
    );

    // Equivalent to formatting the LF source and restoring CRLF.
    let lf_out = m1_fmt::format_str(lf).unwrap().output;
    assert_eq!(out, lf_out.replace('\n', "\r\n"));

    // Idempotent on CRLF.
    let out2 = m1_fmt::format_str(&out).unwrap().output;
    assert_eq!(out, out2);
}

/// #18: brace-adjacent blank stripping must work on CRLF files (the old
/// `ends_with('{')` check failed on a `"...{\r"` line).
#[test]
fn crlf_brace_adjacent_blank_is_stripped() {
    let crlf = "if (a) {\r\n\r\n    x = 1;\r\n}\r\n";
    let out = m1_fmt::format_str(crlf).unwrap().output;
    assert!(
        !out.contains("{\r\n\r\n"),
        "brace-adjacent blank not stripped on CRLF input"
    );
}
