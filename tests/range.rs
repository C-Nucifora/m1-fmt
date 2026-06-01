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
