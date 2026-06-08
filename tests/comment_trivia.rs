//! Regression tests for comment/trivia handling and BOM preservation.

use m1_fmt::{FormatOptions, format_str, format_str_with};

fn fmt(src: &str) -> String {
    format_str(src).unwrap().output
}

/// #bug6: when several statements share one source line, a trailing `// comment`
/// must attach to the statement it actually follows — not the first one on the
/// line. The byte-window check on EOL comments is what scopes attachment.
#[test]
fn eol_comment_attaches_to_the_statement_it_follows() {
    let out = fmt("a = 1; b = 2; // tail of b\n");
    assert_eq!(out, "a = 1;\nb = 2;  // tail of b\n", "got:\n{out}");
}

#[test]
fn eol_comment_attaches_to_last_of_three_statements() {
    let out = fmt("a = 1; b = 2; c = 3; // tail of c\n");
    assert_eq!(out, "a = 1;\nb = 2;\nc = 3;  // tail of c\n", "got:\n{out}");
}

#[test]
fn single_statement_eol_comment_still_attaches() {
    // The original single-statement case must keep working.
    let out = fmt("x = 1; // note\n");
    assert_eq!(out, "x = 1;  // note\n", "got:\n{out}");
}

#[test]
fn eol_comment_attachment_is_idempotent() {
    let once = fmt("a = 1; b = 2; // tail of b\n");
    let twice = fmt(&once);
    assert_eq!(once, twice, "formatting is not idempotent");
}

/// #bug9: a leading UTF-8 BOM must round-trip — preserved on output while the
/// body is reformatted (normal path).
#[test]
fn bom_is_preserved_and_body_formatted() {
    let out = fmt("\u{FEFF}local x=1;\n");
    assert_eq!(out, "\u{FEFF}local x = 1;\n", "got:\n{out:?}");
}

/// #bug9: a BOM in front of a syntax-error buffer must pass through byte-for-byte
/// (the documented data-preserving passthrough), BOM included.
#[test]
fn bom_with_syntax_error_is_byte_for_byte_unchanged() {
    let src = "\u{FEFF}x = ;\n";
    let result = format_str(src).unwrap();
    assert_eq!(result.output, src, "got:\n{:?}", result.output);
    assert!(!result.changed);
}

#[test]
fn bom_round_trips_an_already_formatted_buffer() {
    let src = "\u{FEFF}local x = 1;\n";
    let result = format_str(src).unwrap();
    assert_eq!(result.output, src);
    assert!(
        !result.changed,
        "an already-formatted BOM file must not change"
    );
}

/// #bug8: only a genuine javadoc block (opening `/**`) gets its ` *` continuation
/// lines re-aligned. A plain `/*` block whose interior lines start with `*`
/// (banners, ASCII art, commented-out code) must keep its leading whitespace
/// verbatim.
#[test]
fn non_javadoc_star_lines_are_preserved_verbatim() {
    let src = "/*\n    *custom indent\n        *deeper\n*/\nx = 1;\n";
    let out = fmt(src);
    assert_eq!(
        out, "/*\n    *custom indent\n        *deeper\n*/\nx = 1;\n",
        "non-javadoc star lines must not be re-indented; got:\n{out}"
    );
}

#[test]
fn javadoc_star_lines_are_realigned() {
    // A real javadoc block (opens `/**`) still gets its ` *` lines aligned to a
    // single leading space under the opening `/*`.
    let src = "/**\n    *custom indent\n        *deeper\n*/\nx = 1;\n";
    let out = fmt(src);
    assert_eq!(
        out, "/**\n *custom indent\n *deeper\n */\nx = 1;\n",
        "javadoc star lines must be realigned; got:\n{out}"
    );
}

#[test]
fn non_javadoc_block_comment_is_idempotent() {
    let src = "/*\n    *custom indent\n        *deeper\n*/\nx = 1;\n";
    let once = fmt(src);
    let twice = fmt(&once);
    assert_eq!(once, twice, "block-comment handling is not idempotent");
}

/// A BOM with CRLF line endings must keep both the BOM and CRLF on output.
#[test]
fn bom_with_crlf_round_trips() {
    let out = format_str_with("\u{FEFF}local x=1;\r\n", &FormatOptions::default())
        .unwrap()
        .output;
    assert_eq!(out, "\u{FEFF}local x = 1;\r\n", "got:\n{out:?}");
}
