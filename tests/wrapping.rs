//! Long-expression wrapping (#50): binary chains must not orphan a
//! higher-precedence operand from its operator, and long ternaries must wrap.

use m1_fmt::{FormatOptions, format_str_with};

fn fmt(src: &str) -> String {
    format_str_with(
        src,
        &FormatOptions {
            max_blank_lines: 2,
            line_width: 88,
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
