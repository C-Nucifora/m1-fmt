//! Brace/indent style (#51): manual-correct defaults (Allman + tabs), with
//! `.m1fmt.toml`-equivalent overrides via FormatOptions.

use m1_fmt::{BraceStyle, FormatOptions, IndentStyle, format_str_with};

fn fmt(src: &str, opts: &FormatOptions) -> String {
    format_str_with(src, opts).unwrap().output
}

#[test]
fn default_is_allman_braces_and_tab_indentation() {
    let out = fmt("if (a) {\nValue = 1;\n}\n", &FormatOptions::default());
    assert_eq!(out, "if (a)\n{\n\tValue = 1;\n}\n", "got:\n{out}");
}

#[test]
fn kr_and_spaces_override() {
    let opts = FormatOptions {
        brace_style: BraceStyle::Kr,
        indent_style: IndentStyle::Spaces,
        indent_width: 4,
        ..Default::default()
    };
    let out = fmt("if (a)\n{\nValue = 1;\n}\n", &opts);
    assert_eq!(out, "if (a) {\n    Value = 1;\n}\n", "got:\n{out}");
}

#[test]
fn comment_between_condition_and_brace_stays_before_the_brace() {
    // #76: a comment between `if (...)` and its `{` must stay on its own line
    // before the brace (not be relocated inside the block, and not gain a
    // spurious blank line before the first statement).
    let src = "if (a)\n// note\n{\nValue = 1;\n}\n";
    let out = fmt(src, &FormatOptions::default());
    assert_eq!(out, "if (a)\n// note\n{\n\tValue = 1;\n}\n", "got:\n{out}");
    // Idempotent.
    assert_eq!(fmt(&out, &FormatOptions::default()), out, "not idempotent");
}

#[test]
fn comment_before_is_clause_brace_stays_before_the_brace() {
    let src = "when (m)\n{\nis (X)\n// note\n{\nValue = 1;\n}\n}\n";
    let out = fmt(src, &FormatOptions::default());
    assert!(
        out.contains("is (X)\n\t// note\n\t{"),
        "comment should sit before the is-clause brace, not inside it:\n{out}"
    );
    assert!(
        !out.contains("// note\n\n"),
        "no spurious blank line after the comment:\n{out}"
    );
}

#[test]
fn comment_between_when_subject_and_brace_stays_before_the_brace() {
    // #76 (extended to `when`): a comment between `when (subject)` and its `{`
    // must stay on its own line before the brace — exactly like `if (...)` and
    // `is (...)`. It must NOT be pulled inside the block and indented in front of
    // the first `is`-clause.
    let src = "when (S) // subj\n{\nis (A)\n{\nValue = 1;\n}\n}\n";
    let out = fmt(src, &FormatOptions::default());
    assert!(
        out.contains("when (S)\n// subj\n{"),
        "comment should sit before the when brace, not inside the block:\n{out}"
    );
    assert!(
        !out.contains("{\n\t// subj"),
        "comment must not be relocated inside the block:\n{out}"
    );
    // Idempotent.
    assert_eq!(fmt(&out, &FormatOptions::default()), out, "not idempotent");
}

#[test]
fn allman_puts_else_on_its_own_line() {
    let out = fmt(
        "if (a) {\nValue = 1;\n} else {\nValue = 2;\n}\n",
        &FormatOptions::default(),
    );
    assert_eq!(
        out, "if (a)\n{\n\tValue = 1;\n}\nelse\n{\n\tValue = 2;\n}\n",
        "got:\n{out}"
    );
}
