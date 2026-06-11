//! `@m1:fmt(off)` / `@m1:fmt(on)` format-off regions (#102): the lines from an
//! off marker through its closing on marker (inclusive) pass through the
//! formatter byte-for-byte.

use m1_fmt::{FormatOptions, format_range, format_str, format_str_with};

fn fmt(src: &str) -> String {
    format_str(src).expect("format").output
}

#[test]
fn off_region_is_byte_identical_and_rest_is_formatted() {
    let src = "\
a=1;
// @m1:fmt(off)
b   =    2 ;
c\t=3;
// @m1:fmt(on)
d=4;
";
    let out = fmt(src);
    // Region (markers inclusive) preserved verbatim.
    assert!(out.contains("// @m1:fmt(off)\nb   =    2 ;\nc\t=3;\n// @m1:fmt(on)"));
    // Code outside the region is canonicalised.
    assert!(out.starts_with("a = 1;\n"));
    assert!(out.ends_with("d = 4;\n"));
}

#[test]
fn unclosed_off_runs_to_eof() {
    let src = "\
a=1;
// @m1:fmt(off)
b   =    2 ;
c  =3;
";
    let out = fmt(src);
    assert!(out.starts_with("a = 1;\n"));
    assert!(out.ends_with("// @m1:fmt(off)\nb   =    2 ;\nc  =3;\n"));
}

#[test]
fn off_region_inside_a_block_is_preserved() {
    let src = "\
if (a)
{
\tx=1;
\t// @m1:fmt(off)
\ty   =   2 ;
\t// @m1:fmt(on)
\tz=3;
}
";
    let out = fmt(src);
    assert!(
        out.contains("\t// @m1:fmt(off)\n\ty   =   2 ;\n\t// @m1:fmt(on)"),
        "region inside block not preserved:\n{out}"
    );
    assert!(out.contains("\tx = 1;\n"));
    assert!(out.contains("\tz = 3;\n"));
}

#[test]
fn idempotent_and_check_clean_when_only_region_is_unformatted() {
    let src = "\
a = 1;
// @m1:fmt(off)
b   =    2 ;
// @m1:fmt(on)
c = 3;
";
    let once = fmt(src);
    // Everything outside the region is already canonical, so nothing changes.
    assert_eq!(once, src);
    assert!(!format_str(src).unwrap().changed);
    // And formatting is idempotent across the splice.
    assert_eq!(fmt(&once), once);
}

#[test]
fn trailing_marker_is_not_a_region_marker() {
    // A marker sharing a line with code is inert: regions are delimited only by
    // standalone marker comment lines.
    let src = "b   =    2 ; // @m1:fmt(off)\nc=3;\n";
    let out = fmt(src);
    // (two spaces before a trailing comment is the canonical style)
    assert!(out.contains("b = 2;  // @m1:fmt(off)\n"));
    assert!(out.contains("c = 3;\n"));
}

#[test]
fn on_without_off_is_inert() {
    let src = "// @m1:fmt(on)\nb   =   2 ;\n";
    let out = fmt(src);
    assert_eq!(out, "// @m1:fmt(on)\nb = 2;\n");
}

#[test]
fn second_off_inside_region_is_part_of_the_region() {
    let src = "\
// @m1:fmt(off)
a   =1;
// @m1:fmt(off)
b   =2;
// @m1:fmt(on)
c=3;
";
    let out = fmt(src);
    assert!(
        out.starts_with("// @m1:fmt(off)\na   =1;\n// @m1:fmt(off)\nb   =2;\n// @m1:fmt(on)\n")
    );
    assert!(out.ends_with("c = 3;\n"));
}

#[test]
fn crlf_document_with_off_region_round_trips() {
    let src = "a=1;\r\n// @m1:fmt(off)\r\nb   =   2 ;\r\n// @m1:fmt(on)\r\nc=4;\r\n";
    let out = fmt(src);
    assert!(out.contains("// @m1:fmt(off)\r\nb   =   2 ;\r\n// @m1:fmt(on)\r\n"));
    assert!(out.starts_with("a = 1;\r\n"));
    assert!(out.ends_with("c = 4;\r\n"));
    // Idempotent on CRLF too.
    assert_eq!(fmt(&out), out);
}

#[test]
fn line_too_long_warnings_are_suppressed_inside_off_regions() {
    let long = "x".repeat(120);
    let src = format!("// @m1:fmt(off)\nlocal {long} = 1;\n// @m1:fmt(on)\na=1;\n");
    let result = format_str(&src).expect("format");
    assert!(
        result.warnings.is_empty(),
        "off-region lines are deliberate; got {:?}",
        result.warnings
    );
}

#[test]
fn format_range_overlapping_an_off_region_declines() {
    let src = "\
a=1;
// @m1:fmt(off)
b   =    2 ;
// @m1:fmt(on)
c=3;
";
    // Lines 1-3 (0-based) are the off region; a request inside it must decline.
    let r = format_range(src, 2, 2, &FormatOptions::default()).expect("range");
    assert!(r.is_none(), "range inside an off region must be left alone");
    // A request outside it still formats.
    let r = format_range(src, 0, 0, &FormatOptions::default())
        .expect("range")
        .expect("outside the region formats");
    assert_eq!(r.output.trim_end(), "a = 1;");
}

#[test]
fn marker_text_inside_a_block_comment_is_not_a_marker() {
    let src = "/*\n// @m1:fmt(off)\n*/\nb   =   2 ;\n";
    let out = fmt(src);
    assert!(
        out.ends_with("b = 2;\n"),
        "block-comment text tripped the marker:\n{out}"
    );
}

#[test]
fn options_apply_outside_but_not_inside_the_region() {
    let src = "\
if (a)
{
    s=1;
    // @m1:fmt(off)
    t    =     2;
    // @m1:fmt(on)
}
";
    // Default style is tabs; outside the region spaces become tabs, inside stays.
    let out = format_str_with(src, &FormatOptions::default())
        .unwrap()
        .output;
    assert!(out.contains("\ts = 1;\n"));
    assert!(out.contains("    // @m1:fmt(off)\n    t    =     2;\n    // @m1:fmt(on)\n"));
}
