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
        brace_style: BraceStyle::KAndR,
        indent_style: IndentStyle::Spaces,
        indent_width: 4,
        ..Default::default()
    };
    let out = fmt("if (a)\n{\nValue = 1;\n}\n", &opts);
    assert_eq!(out, "if (a) {\n    Value = 1;\n}\n", "got:\n{out}");
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
