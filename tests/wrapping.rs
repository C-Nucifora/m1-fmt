//! Long-expression wrapping (#50): binary chains must not orphan a
//! higher-precedence operand from its operator, and long ternaries must wrap.

use m1_fmt::{FormatOptions, format_str_with};
use std::time::{Duration, Instant};

fn fmt(src: &str) -> String {
    format_str_with(
        src,
        &FormatOptions {
            max_blank_lines: 2,
            line_width: 88,
            ..Default::default()
        },
    )
    .unwrap()
    .output
}

#[test]
fn binary_chain_keeps_relational_operands_with_their_operator() {
    // A long `and` chain of relational comparisons. Breaking should happen only
    // at the logical (`and`) level; each `X eq N` stays intact.
    let src = "if (Aaaaaaaaaaaaaaaaaaaaaaaaa.Bbbbbb eq 1 and Ccccccccccccccccccccccc.Dddddd eq 2 and Eeeeeeeeeeeeeeee.Ffff eq 3) {\nValue = 1;\n}\n";
    let out = fmt(src);
    for line in out.lines() {
        // No continuation line should start with a bare relational operator
        // (the over-break symptom: `eq 3` orphaned onto its own line).
        let t = line.trim_start();
        assert!(
            !(t.starts_with("eq ") || t.starts_with("neq ")),
            "relational operand orphaned from its operator:\n{out}"
        );
    }
}

#[test]
fn long_ternary_wraps_under_the_line_width() {
    let src = "x = Conditionnnnnnnnnnnnnnnnnnnnn.Foo eq 1 ? AlternativeOneeeeeeeeeeeeeeeee.Value : AlternativeTwooooooooooooo.Value;\n";
    let out = fmt(src);
    for line in out.lines() {
        assert!(
            line.chars().count() <= 88,
            "ternary line exceeds width:\n{out}"
        );
    }
    assert!(
        out.lines().any(|l| l.trim_start().starts_with("? ")),
        "expected a `?`-led continuation line:\n{out}"
    );
    assert!(
        out.lines().any(|l| l.trim_start().starts_with(": ")),
        "expected a `:`-led continuation line:\n{out}"
    );
}

// #64: the wrap decision must measure each sub-expression once, not re-render
// the whole nested subtree at every ancestor. Without memoization the cost is
// O(2^N) and a ~200-byte deeply-nested script hangs the formatter for minutes;
// these inputs each finish in milliseconds once the per-subtree flat render is
// cached. A generous bound keeps the test non-flaky while still failing hard on
// the exponential blowup.
#[test]
fn deeply_nested_calls_format_in_bounded_time() {
    let n = 80;
    let src = format!("x = {}1{};\n", "f(".repeat(n), ")".repeat(n));
    let start = Instant::now();
    let out = fmt(&src);
    let elapsed = start.elapsed();
    assert!(
        elapsed < Duration::from_secs(3),
        "deeply-nested calls took {elapsed:?} (expected < 3s); wrap is exponential"
    );
    // Sanity: a stable, non-empty result that round-trips.
    assert!(out.contains("f("));
    assert_eq!(fmt(&out), out, "formatting is not idempotent");
}

#[test]
fn deeply_nested_binary_formats_in_bounded_time() {
    let n = 80;
    // `local x = (a + (a + (a + ... )))` — a tower of parenthesised binaries.
    let src = format!("local x = {}a{};\n", "(a + ".repeat(n), ")".repeat(n));
    let start = Instant::now();
    let out = fmt(&src);
    let elapsed = start.elapsed();
    assert!(
        elapsed < Duration::from_secs(3),
        "deeply-nested binaries took {elapsed:?} (expected < 3s); wrap is exponential"
    );
    assert_eq!(fmt(&out), out, "formatting is not idempotent");
}
