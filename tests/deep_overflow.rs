//! Regression for #72: a pathologically deep document must not stack-overflow
//! the recursive printer. The formatter declines such input (leaves it
//! unchanged) instead of aborting the process.

#[test]
fn deeply_nested_input_is_declined_not_crashed() {
    let src = format!("x = {}1{};\n", "(".repeat(50_000), ")".repeat(50_000));

    // Whole-document format: returns (does not abort) and leaves the input as-is.
    let r = m1_fmt::format_str(&src).expect("format must return, not crash");
    assert!(!r.changed, "a too-deep document must be left unchanged");
    assert_eq!(r.output, src);

    // Range format: declines (None) rather than recursing into the printer.
    let rr = m1_fmt::format_range(&src, 0, 0, &m1_fmt::FormatOptions::default())
        .expect("range format must return");
    assert!(rr.is_none());
}
