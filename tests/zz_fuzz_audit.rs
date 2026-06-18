use m1_fmt::{FormatOptions, format_range};

#[test]
fn range_out_of_bounds_lines() {
    let opts = FormatOptions::default();
    let src = "x = 1;\ny = 2;\n";
    // request a range past EOF
    let r = format_range(src, 100, 200, &opts).unwrap();
    // No statement overlaps -> None
    assert!(r.is_none());
    // request line 0
    let _ = format_range(src, 0, 0, &opts).unwrap();
}

#[test]
fn range_crlf_preserved() {
    let opts = FormatOptions::default();
    let src = "x=1;\r\ny=2;\r\n";
    if let Some(rr) = format_range(src, 0, 0, &opts).unwrap() {
        assert!(
            rr.output.contains("\r\n") || !rr.changed,
            "CRLF lost in range output: {:?}",
            rr.output
        );
    }
}

#[test]
fn range_node_end_line_beyond_split() {
    // A node whose end.line could exceed src.split('\n') count?
    // Construct a file with no trailing newline.
    let opts = FormatOptions::default();
    let src = "x = 1;\ny = 2;"; // no trailing newline
    let _ = format_range(src, 1, 1, &opts).unwrap();
    let _ = format_range(src, 0, 1, &opts).unwrap();
}
