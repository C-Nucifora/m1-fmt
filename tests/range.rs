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

#[test]
fn range_declines_truncated_multiline_comment() {
    let src = "if (A)\n{\n\tX=1; /*\nhi\n*/\n}\n";
    assert_eq!(m1_fmt::syntax_error_count(src), 0);
    for _ in 0..2 {
        assert!(format_range(src, 2, 2, &opts()).unwrap().is_none());
    }
}

#[test]
fn complete_comment_ranges_format_and_preserve_comment_layout() {
    let comments = [
        ("/* cafe\n  L\n*/", false),
        ("/* café\n\t  λ\n\n\n\n*/", false),
        ("/*\n    *custom indent\n\t*deeper\n  nonstar\n*/", false),
        ("/**\n    *doc\n\t*more\n  raw café\n\tλ\n\n*/", true),
    ];
    for (comment, is_javadoc) in comments {
        for newline in ["\n", "\r\n"] {
            for (unit, style) in [
                ("\t", m1_fmt::IndentStyle::Tab),
                ("  ", m1_fmt::IndentStyle::Spaces),
            ] {
                for depth in [0, 1, 2] {
                    let options = FormatOptions {
                        indent_style: style,
                        indent_width: 2,
                        ..opts()
                    };
                    let prefix = unit.repeat(depth);
                    let mut before = String::new();
                    let mut after = String::new();
                    for level in 0..depth {
                        let indent = unit.repeat(level);
                        before.push_str(&format!("{indent}if (A)\n{indent}{{\n"));
                    }
                    for level in (0..depth).rev() {
                        after.push_str(&format!("{}}}\n", unit.repeat(level)));
                    }
                    let body = format!("{prefix}X=1;\n{prefix}{comment}\n{prefix}Y=2;\n");
                    let src = format!("{before}{body}{after}").replace('\n', newline);
                    let start = depth * 2;
                    let end = start + body.lines().count() - 1;
                    let r = format_range(&src, start, end, &options)
                        .unwrap()
                        .expect("complete comment range must format");
                    assert_eq!((r.start_line, r.end_line), (start, end));
                    assert!(r.changed, "assignments must be formatted");

                    let expected_comment = comment
                        .split('\n')
                        .enumerate()
                        .map(|(line_idx, line)| {
                            if is_javadoc && line_idx > 0 && line.trim_start().starts_with('*') {
                                format!("{prefix} {}", line.trim_start())
                            } else {
                                line.to_string()
                            }
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    let expected =
                        format!("{prefix}X = 1;\n{prefix}{expected_comment}\n{prefix}Y = 2;\n")
                            .replace('\n', newline);
                    assert_eq!(r.output, expected, "depth {depth}, {style:?}, {newline:?}");
                    let once = format!(
                        "{}{expected}{}",
                        before.replace('\n', newline),
                        after.replace('\n', newline)
                    );
                    assert_eq!(m1_fmt::syntax_error_count(&once), 0, "{once}");
                    let comment_texts = |text: &str| {
                        m1_core::parse(text)
                            .root()
                            .descendants()
                            .filter(|node| node.kind() == m1_core::Kind::BlockComment)
                            .map(|node| node.text().to_owned())
                            .collect::<Vec<_>>()
                    };
                    assert_eq!(
                        comment_texts(&once),
                        vec![expected_comment.replace('\n', newline)]
                    );
                    if !is_javadoc {
                        assert_eq!(comment_texts(&once), comment_texts(&src));
                    }
                    let twice = format_range(&once, start, end, &options)
                        .unwrap()
                        .expect("second pass must still format");
                    assert_eq!(twice.output, r.output);
                    assert!(!twice.changed);
                }
            }
        }
    }
}

#[test]
fn range_comment_spans_ignore_delimiters_in_strings_and_line_comments() {
    let src = "if (A)\n{\n\tX=\"/*\"; // /**\n\tY=\"*/\";\n\t/* raw\n  keep\n*/\n\tZ=3;\n}\n";
    let r = format_range(src, 2, 7, &opts()).unwrap().unwrap();
    assert_eq!(
        r.output,
        "\tX = \"/*\";  // /**\n\tY = \"*/\";\n\t/* raw\n  keep\n*/\n\tZ = 3;\n"
    );
    assert!(r.changed);
    let once = format!("if (A)\n{{\n{}}}\n", r.output);
    assert_eq!(m1_fmt::syntax_error_count(&once), 0);
    let twice = format_range(&once, 2, 7, &opts()).unwrap().unwrap();
    assert_eq!(twice.output, r.output);
    assert!(!twice.changed);
}

#[test]
fn range_preserves_comment_inside_a_nested_statement() {
    let src = "if (A)\n{\n\tX=1;\n\tif (B)\n\t{\n\t\tY=2;\n\t\t/* table\n *ordinary\n  raw\n*/\n\t\tZ=3;\n\t}\n\tW=4;\n}\n";
    let r = format_range(src, 2, 12, &opts()).unwrap().unwrap();
    let expected = "\tX = 1;\n\tif (B)\n\t{\n\t\tY = 2;\n\t\t/* table\n *ordinary\n  raw\n*/\n\t\tZ = 3;\n\t}\n\tW = 4;\n";
    assert_eq!(r.output, expected);
    assert!(r.changed);
    let once = format!("if (A)\n{{\n{}}}\n", r.output);
    assert_eq!(m1_fmt::syntax_error_count(&once), 0);
    let twice = format_range(&once, 2, 12, &opts()).unwrap().unwrap();
    assert_eq!(twice.output, expected);
    assert!(!twice.changed);
}

#[test]
fn complete_trailing_comment_and_javadoc_share_a_range() {
    let src = "if (A)\n{\n\tX=1; /* café\n  λ\n*/\n\t/**\n    *doc\n  raw\n*/\n\tY=2;\n}\n";
    let r = format_range(src, 2, 9, &opts()).unwrap().unwrap();
    let expected = "\tX = 1;  /* café\n  λ\n*/\n\n\n\t/**\n\t *doc\n  raw\n\t */\n\tY = 2;\n";
    assert_eq!(r.output, expected);
    assert!(r.changed);
    let once = format!("if (A)\n{{\n{}}}\n", r.output);
    assert_eq!(m1_fmt::syntax_error_count(&once), 0);
    let end = 2 + r.output.lines().count() - 1;
    let twice = format_range(&once, 2, end, &opts()).unwrap().unwrap();
    assert_eq!(twice.output, expected);
    assert!(!twice.changed);
}
