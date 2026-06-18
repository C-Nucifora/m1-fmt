//! Pathologically large width knobs must not crash the formatter.
//!
//! `visual_width` sums `indent_width` per tab and the continuation-column math
//! computes `(indent + continuation_indent) * indent_width`. With an absurd
//! `indent_width` / `continuation_indent` these arithmetic ops used to overflow
//! and panic in a debug build (or wrap/hang in release). The CLI clamps such
//! values at option-resolution time; the printer additionally uses saturating
//! arithmetic as defense-in-depth, so even when `FormatOptions` is constructed
//! directly (bypassing the clamp) the formatter degrades gracefully instead of
//! crashing.

use m1_fmt::{FormatOptions, format_str_with};

/// A binary chain whose RHS wraps, exercising the `(indent + continuation_indent)
/// * indent_width` continuation-column math and the tab-expanding `visual_width`.
const WRAPPING_SRC: &str =
    "local myResult = DemoGroup.ValueA + DemoGroup.ValueB + DemoGroup.ValueC + DemoGroup.ValueD;\n";

#[test]
fn huge_indent_width_does_not_panic() {
    // visual_width expands each tab to indent_width; usize::MAX would overflow
    // the per-tab accumulation without saturating arithmetic.
    let opts = FormatOptions {
        indent_width: usize::MAX,
        ..Default::default()
    };
    // Must return (not panic / not abort).
    let _ = format_str_with(WRAPPING_SRC, &opts).expect("must return, not crash");
}

#[test]
fn huge_continuation_indent_does_not_panic() {
    // (indent + continuation_indent) * indent_width would overflow without
    // saturating arithmetic. A space indent_style of 1 keeps the produced
    // indentation finite (saturated column, not usize::MAX spaces emitted).
    let opts = FormatOptions {
        continuation_indent: usize::MAX,
        indent_style: m1_fmt::IndentStyle::Spaces,
        indent_width: 1,
        ..Default::default()
    };
    let _ = format_str_with(WRAPPING_SRC, &opts).expect("must return, not crash");
}
