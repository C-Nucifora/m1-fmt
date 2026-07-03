//! Range formatting (#20): only the top-level statements overlapping the
//! requested line range are reformatted; the range snaps outward to whole
//! statements.
use m1_fmt::{FormatOptions, format_range};

fn opts() -> FormatOptions {
    FormatOptions::default()
}

#[test]
fn formats_only_the_targeted_statement() {
    let src = "local a=1;\nlocal b   =   2;\nlocal c=3;\n";
    // line 2 (0-based index 1) only.
    let r = format_range(src, 1, 1, &opts()).unwrap().unwrap();
    assert_eq!((r.start_line, r.end_line), (1, 1));
    assert_eq!(r.output, "local b = 2;\n");
    assert!(r.changed);
}

#[test]
fn snaps_outward_to_a_multiline_statement() {
    // A single statement spanning input lines 0..=1; a range hitting only line 0
    // must cover the whole statement.
    let src = "local x = Foo(\n  a,b);\nlocal y=2;\n";
    let r = format_range(src, 0, 0, &opts()).unwrap().unwrap();
    assert_eq!(r.start_line, 0);
    assert_eq!(
        r.end_line, 1,
        "range should snap to cover the whole statement"
    );
    // and the trailing `local y=2;` must NOT be in the covered output.
    assert!(!r.output.contains("local y"));
    assert!(r.output.starts_with("local x = Foo("));
}

#[test]
fn no_overlap_returns_none() {
    // A buffer whose only statement is on line 0; selecting a trailing blank
    // region overlaps nothing.
    let src = "local a = 1;\n\n\n";
    assert!(format_range(src, 2, 2, &opts()).unwrap().is_none());
}

#[test]
fn already_formatted_region_reports_no_change() {
    let src = "local a = 1;\nlocal b = 2;\n";
    let r = format_range(src, 0, 0, &opts()).unwrap().unwrap();
    assert!(!r.changed, "clean region should not be marked changed");
    assert_eq!(r.output, "local a = 1;\n");
}

#[test]
fn syntax_error_buffer_is_left_alone() {
    // Incomplete buffer: do not attempt a range format.
    let src = "local a = (1 +;\n";
    assert!(format_range(src, 0, 0, &opts()).unwrap().is_none());
}

/// #65: range formatting on a CRLF file must keep CRLF line endings. The range
/// path used to split on `'\n'` and rejoin with `'\n'`, dropping each line's
/// `'\r'`, so the formatted slice came back LF-only and was spliced into the
/// otherwise-CRLF buffer — corrupting line endings even when content was
/// unchanged. The formatted output must carry `\r\n`, not bare `\n`.
#[test]
fn crlf_range_preserves_crlf_line_endings() {
    let src = "local x = 1;\r\nlocal y   =   2;\r\nlocal z = 3;\r\n";
    // Reformat only line 2 (0-based index 1).
    let r = format_range(src, 1, 1, &opts()).unwrap().unwrap();
    assert!(
        r.output.contains("\r\n"),
        "CRLF must be preserved in range output, got {:?}",
        r.output
    );
    assert!(
        !r.output.contains('\n') || r.output.replace("\r\n", "").matches('\n').count() == 0,
        "no bare LF should remain in range output, got {:?}",
        r.output
    );
    assert_eq!(r.output, "local y = 2;\r\n");
    assert!(r.changed);
}

/// #65: a CRLF region that is *already* canonically formatted must report no
/// change (the old code's LF-vs-CRLF mismatch made `--check` see a spurious
/// diff).
#[test]
fn crlf_range_already_formatted_reports_no_change() {
    let src = "local a = 1;\r\nlocal b = 2;\r\n";
    let r = format_range(src, 0, 0, &opts()).unwrap().unwrap();
    assert!(
        !r.changed,
        "clean CRLF region should not be marked changed, got output {:?}",
        r.output
    );
}

#[test]
fn range_warnings_use_file_line_numbers() {
    use m1_fmt::format_str_with;
    // The over-width comment is on file line 5 (0-based index 4).
    let src = "X = 1;\nY = 2;\nif (A)\n{\n\t// aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n}\n";
    let mut o = opts();
    o.line_width = 20;
    let r = format_range(src, 4, 4, &o).unwrap().unwrap();
    // Sanity: the same content, formatted whole, warns on the same file line.
    let whole = format_str_with(src, &o).unwrap();
    assert!(!whole.warnings.is_empty(), "expected an over-width warning");
    // The range warning must report the FILE line (5), not slice-relative (1).
    assert!(
        r.warnings.iter().any(|w| w.line == 5),
        "range warnings must be file-relative, got {:?}",
        r.warnings.iter().map(|w| w.line).collect::<Vec<_>>()
    );
}
