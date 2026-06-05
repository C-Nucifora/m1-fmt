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

/// #60: a statement that fmt *wraps* inside one `is`-block shifts source-line
/// geometry between passes. The blank gap before the own-line comment preceding
/// the next `is`-clause was measured from raw source line numbers (the last
/// inner statement's end vs the comment's start), which ignored the intervening
/// `}` line and drifted under wrapping — adding a blank line on the 2nd pass.
/// `fmt∘fmt` must equal `fmt`.
#[test]
fn wrapped_stmt_then_comment_before_is_clause_is_idempotent() {
    let src = "when (Value)\n\
               {\n\
               \tis (Emergency)\n\
               \t{\n\
               \t\tInverter Active = SomeReallyLongFunctionName.ThatWrapsAcross(ArgumentOne, ArgumentTwo, ArgThree);\n\
               \t}\n\
               \t// a latching fault has occured\n\
               \tis (Latching Fault)\n\
               \t{\n\
               \t\tValue = Drive State.Idle;\n\
               \t}\n\
               }\n";
    let once = m1_fmt::format_str(src).unwrap().output;
    let twice = m1_fmt::format_str(&once).unwrap().output;
    assert_eq!(
        once, twice,
        "format must be idempotent across a wrapped statement before an is-clause comment\n--- first ---\n{once}\n--- second ---\n{twice}"
    );
    // Sanity: the wrapped statement really does span two output lines.
    assert!(
        once.contains("ThatWrapsAcross(ArgumentOne,\n"),
        "test precondition: the long assignment should wrap"
    );
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

/// #66: block-comment interior whitespace is load-bearing (aligned tables, ASCII
/// diagrams, indented commented-out code) and must survive verbatim. The old
/// `trim_start()` on every continuation line flattened the indentation of
/// non-`*`-prefixed lines to column 0. The comment body must round-trip
/// byte-for-byte.
#[test]
fn block_comment_interior_indentation_is_preserved() {
    let src = "/* table:\n     col1    col2\n     a       b\n*/\nlocal x = 1;\n";
    let out = m1_fmt::format_str(src).unwrap().output;
    assert!(
        out.contains("     col1    col2"),
        "block-comment interior indentation must be preserved verbatim:\n{out}"
    );
    assert!(
        out.contains("     a       b"),
        "block-comment interior indentation must be preserved verbatim:\n{out}"
    );
    // And the whole thing is idempotent.
    let twice = m1_fmt::format_str(&out).unwrap().output;
    assert_eq!(out, twice, "block-comment formatting must be idempotent");
}

/// #66: a commented-out, tab-indented code block inside `/* */` (as found in the
/// real corpus) must keep its indentation, not collapse to column 0.
#[test]
fn block_comment_indented_code_block_keeps_indentation() {
    let src = "/*\n\tif (Foo)\n\t{\n\t\tBar = 1;\n\t}\n*/\nlocal y = 2;\n";
    let out = m1_fmt::format_str(src).unwrap().output;
    assert!(
        out.contains("\n\tif (Foo)\n"),
        "tab-indented commented-out code must keep its indentation:\n{out:?}"
    );
    assert!(
        out.contains("\n\t\tBar = 1;\n"),
        "nested tab indentation inside the comment must be preserved:\n{out:?}"
    );
}
