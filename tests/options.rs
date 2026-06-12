#[test]
fn format_str_with_default_matches_format_str() {
    let src = "x=1+2;\n";
    let a = m1_fmt::format_str(src).unwrap().output;
    let b = m1_fmt::format_str_with(src, &m1_fmt::FormatOptions::default())
        .unwrap()
        .output;
    assert_eq!(a, b);
}

#[test]
fn default_options_are_two_blank_lines_and_88() {
    let o = m1_fmt::FormatOptions::default();
    assert_eq!(o.max_blank_lines, 2);
    assert_eq!(o.line_width, 88);
}

#[test]
fn final_blank_line_appends_exactly_one_blank_line() {
    // #116: the formatter pair of m1-lint L027 (manual p.65 at file scope).
    let opts = m1_fmt::FormatOptions {
        final_blank_line: true,
        ..Default::default()
    };
    let out = m1_fmt::format_str_with("local x = 1;\n", &opts)
        .unwrap()
        .output;
    assert!(out.ends_with(";\n\n"), "got {out:?}");
    assert!(!out.ends_with("\n\n\n"), "got {out:?}");
    // Idempotent: formatting the output again changes nothing.
    let again = m1_fmt::format_str_with(&out, &opts).unwrap().output;
    assert_eq!(out, again);
    // A source that already ends with many blank lines collapses to one.
    let out2 = m1_fmt::format_str_with("local x = 1;\n\n\n\n", &opts)
        .unwrap()
        .output;
    assert_eq!(out, out2);
}

#[test]
fn final_blank_line_off_keeps_single_trailing_newline() {
    let out = m1_fmt::format_str("local x = 1;\n\n\n").unwrap().output;
    assert!(out.ends_with(";\n"), "got {out:?}");
    assert!(!out.ends_with("\n\n"), "got {out:?}");
}
