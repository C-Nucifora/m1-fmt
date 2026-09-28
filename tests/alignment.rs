use m1_fmt::{FormatOptions, format_str_with};

fn formatted(src: &str) -> String {
    formatted_at_width(src, 120)
}

fn formatted_at_width(src: &str, line_width: usize) -> String {
    let opts = FormatOptions {
        align_assignments: true,
        align_conditions: true,
        line_width,
        ..Default::default()
    };
    checked(src, &opts)
}

#[test]
fn typed_declarations_align() {
    assert_eq!(
        formatted("local <Boolean> A = true;\nlocal <Boolean> Longer = false;\n"),
        "local <Boolean> A      = true;\nlocal <Boolean> Longer = false;\n"
    );
}

#[test]
fn counter_updates_do_not_align_with_their_declarations() {
    for declaration in ["local <Integer>", "static local <Integer>"] {
        let expected = format!("{declaration} i = -1;\ni = i eq 199 ? 0 : i + 1;\n");
        assert_eq!(formatted(&expected), expected);
        let padded =
            format!("{declaration} i = -1;\ni                        = i eq 199 ? 0 : i + 1;\n");
        assert_eq!(formatted(&padded), expected);
    }
}

#[test]
fn declarations_and_assignments_form_separate_alignment_groups() {
    assert_eq!(
        formatted(concat!(
            "A = 1;\nLonger = 2;\n",
            "local <Integer> B = 3;\nlocal <Integer> Longest = 4;\n",
            "C = 5;\nMedium = 6;\n",
        )),
        concat!(
            "A      = 1;\nLonger = 2;\n",
            "local <Integer> B       = 3;\nlocal <Integer> Longest = 4;\n",
            "C      = 5;\nMedium = 6;\n",
        )
    );
}

#[test]
fn comparisons_have_their_own_rows_and_columns() {
    assert_eq!(
        formatted_at_width("if (A eq 1 and Longer neq 2) { X = 0; }\n", 24),
        "if (\n\tA      eq  1 and\n\tLonger neq 2\n)\n{\n\tX = 0;\n}\n"
    );
}

#[test]
fn boolean_initializer_keeps_comparisons_together() {
    assert_eq!(
        formatted_at_width("local <Boolean> Ready = A > 0 and Longer >= 1;\n", 32),
        "local <Boolean> Ready =\n\tA      >  0 and\n\tLonger >= 1;\n"
    );
}

#[test]
fn nested_parentheses_stay_visible() {
    assert_eq!(
        formatted_at_width("if (A eq 1 or (B < 2 and Longer < 3)) { X = 0; }\n", 24),
        "if (\n\tA eq 1 or\n\t(\n\t\tB      < 2 and\n\t\tLonger < 3\n\t)\n)\n{\n\tX = 0;\n}\n"
    );
}

#[test]
fn fitting_conditions_stay_inline() {
    for expected in [
        "bRearLeftCommsLost = bRearLeftCommsLost or msgTimeout;\n",
        "msgRecieved = (CanComms.RxFindMessage(dtiRx, 0x433, 0, 0, 0x00) and CanComms.GetLength(dtiRx) >= 8);\n",
        "local <Boolean> Ready = A > 0 and Longer >= 1;\n",
        "if (A eq 1 or (B < 2 and Longer < 3))\n{\n\tX = 0;\n}\n",
    ] {
        assert_eq!(formatted(expected), expected);
    }
    assert_eq!(
        formatted("bRearLeftCommsLost =\n\tbRearLeftCommsLost or\n\tmsgTimeout;\n"),
        "bRearLeftCommsLost = bRearLeftCommsLost or msgTimeout;\n"
    );
}

#[test]
fn condition_width_includes_statement_prefix_and_suffix() {
    for statement in [
        "Ready = First Condition and Second Condition;",
        "Ready = First Condition and Second Condition;  // status",
        "local <Boolean> Ready = First Condition and Second Condition;",
        "local <Boolean> Ready = First Condition and Second Condition;  // status",
    ] {
        for indent_style in [m1_fmt::IndentStyle::Tab, m1_fmt::IndentStyle::Spaces] {
            let indent = if indent_style == m1_fmt::IndentStyle::Tab {
                "\t"
            } else {
                "    "
            };
            let src = format!("if (Enabled)\n{{\n{indent}{statement}\n}}\n");
            let mut opts = FormatOptions {
                align_conditions: true,
                indent_style,
                line_width: 4 + statement.len(),
                ..Default::default()
            };
            assert_eq!(checked(&src, &opts), src);
            opts.line_width -= 1;
            let wrapped = checked(&src, &opts);
            assert!(wrapped.contains("=\n"), "{wrapped}");
        }
    }
}

#[test]
fn condition_width_includes_if_delimiters_and_else_prefix() {
    for brace_style in [m1_fmt::BraceStyle::Allman, m1_fmt::BraceStyle::Kr] {
        let opts = FormatOptions {
            align_conditions: true,
            brace_style,
            ..Default::default()
        };
        for src in [
            "if (First Condition and Second Condition) { X = 0; }\n",
            "if (Enabled) { X = 1; } else if (First Condition and Second Condition) { X = 0; }\n",
        ] {
            let expected = checked(
                src,
                &FormatOptions {
                    align_conditions: false,
                    ..opts.clone()
                },
            );
            let width = expected
                .lines()
                .find(|line| line.contains("First Condition"))
                .unwrap()
                .len();
            let mut opts = FormatOptions {
                line_width: width,
                ..opts.clone()
            };
            assert_eq!(checked(src, &opts), expected);
            opts.line_width -= 1;
            let wrapped = checked(src, &opts);
            assert!(wrapped.contains("if (\n"), "{wrapped}");
        }
    }
}

#[test]
fn pre_brace_comments_do_not_reserve_kr_brace_width() {
    for comment in ["// x\n", "/* x */\n", "/* x */ "] {
        let src = format!("if (A and B) {comment}{{ X = 0; }}\n");
        let expected = format!("if (A and B)\n{}\n{{\n\tX = 0;\n}}\n", comment.trim());
        let mut opts = FormatOptions {
            align_conditions: true,
            brace_style: m1_fmt::BraceStyle::Kr,
            line_width: 12,
            ..Default::default()
        };
        assert_eq!(checked(&src, &opts), expected);
        opts.line_width = 11;
        assert!(checked(&src, &opts).starts_with("if (\n"));
    }
}

#[test]
fn fitting_group_inside_wrapped_condition_stays_inline() {
    let result = formatted_at_width(
        "if (Very Long Condition Name and (A eq 1 or B eq 2)) { X = 0; }\n",
        40,
    );
    assert_eq!(
        result,
        "if (\n\tVery Long Condition Name and\n\t(A eq 1 or B eq 2)\n)\n{\n\tX = 0;\n}\n"
    );
}

#[test]
fn nested_condition_rows_use_full_width_and_reserve_their_separator() {
    for brace_style in [m1_fmt::BraceStyle::Allman, m1_fmt::BraceStyle::Kr] {
        for (condition, row) in [
            ("Enabled and (A eq 1 or B eq 2)", "(A eq 1 or B eq 2)"),
            ("(A eq 1 or B eq 2) and Enabled", "(A eq 1 or B eq 2) and"),
            ("(A eq 1 or B eq 2) or Enabled", "(A eq 1 or B eq 2) or"),
        ] {
            let src = format!("if ({condition}) {{ X = 0; }}\n");
            let mut opts = FormatOptions {
                align_conditions: true,
                brace_style,
                line_width: 4 + row.len(),
                ..Default::default()
            };
            let result = checked(&src, &opts);
            assert!(
                result.lines().any(|line| line == format!("\t{row}")),
                "{result}"
            );
            opts.line_width -= 1;
            let wrapped = checked(&src, &opts);
            assert!(wrapped.contains("\t(\n"), "{wrapped}");
        }
    }
}

#[test]
fn nested_condition_rows_do_not_reserve_suffixes_on_the_closing_line() {
    for src in [
        "Ready = (Enabled and (A eq 1 or B eq 2));\n",
        "Ready = (Enabled and (A eq 1 or B eq 2));  // state\n",
        "Ready = (Enabled and (A eq 1 or B eq 2)) or Other;\n",
    ] {
        let result = formatted_at_width(src, 26);
        assert!(result.contains("\t\t(A eq 1 or B eq 2)\n"), "{result}");
        let wrapped = formatted_at_width(src, 25);
        assert!(wrapped.contains("\t\t(\n"), "{wrapped}");
    }
}

#[test]
fn closing_block_comment_does_not_force_the_condition_to_wrap() {
    let src = "if (A and B)\n{\n\tX = 0;\n}  // block is done\n";
    for (brace_style, expected) in [
        (m1_fmt::BraceStyle::Allman, src),
        (
            m1_fmt::BraceStyle::Kr,
            "if (A and B) {\n\tX = 0;\n}  // block is done\n",
        ),
    ] {
        let opts = FormatOptions {
            align_conditions: true,
            brace_style,
            line_width: 24,
            ..Default::default()
        };
        assert_eq!(checked(src, &opts), expected);
    }
}

#[test]
fn alignment_is_opt_in() {
    let src = "if (A eq 1 and Longer neq 2) { X = 0; }\n";
    let result = format_str_with(src, &FormatOptions::default())
        .unwrap()
        .output;
    assert!(result.starts_with("if (A eq 1 and Longer neq 2)\n"));
}

#[test]
fn spaces_and_kr_work_with_nested_conditions() {
    let opts = FormatOptions {
        align_conditions: true,
        indent_style: m1_fmt::IndentStyle::Spaces,
        indent_width: 2,
        brace_style: m1_fmt::BraceStyle::Kr,
        line_width: 22,
        ..Default::default()
    };
    let src = "if (A eq 1 and (B < 2 or Longer >= 3)) { X = 0; }\n";
    let result = checked(src, &opts);
    assert!(
        result.contains("    B      <  2 or\n    Longer >= 3\n  )\n) {"),
        "{result}"
    );
    assert!(!result.contains('\t'));
}

#[test]
fn embedded_comments_stay_inside_the_expression() {
    for src in [
        "if (A eq 1 and /* guard */ Longer neq 2) { X = 0; }\n",
        "local <Boolean> Ready = A eq 1 and // guard\nLonger neq 2;\n",
    ] {
        let opts = FormatOptions {
            align_conditions: true,
            ..Default::default()
        };
        let result = checked(src, &opts);
        assert!(
            result.contains("and /* guard */ Longer") || result.contains("and // guard\nLonger"),
            "{result}"
        );
    }
}

#[test]
fn comparison_padding_and_boolean_suffix_respect_width() {
    let opts = FormatOptions {
        align_conditions: true,
        line_width: 40,
        ..Default::default()
    };
    for src in [
        "if (Medium Name eq 12345678901234567890 and Longer eq 1) { X = 0; }\n",
        "Ready = Medium Name eq 12345678901234567890 and Longer eq 1;\n",
        "if ((A eq 1 and Longer eq 2) or B eq 3) { X = 0; }\n",
    ] {
        let result = checked(src, &opts);
        for line in result.lines() {
            let width: usize = line.chars().map(|c| if c == '\t' { 4 } else { 1 }).sum();
            assert!(
                width <= opts.line_width,
                "{width} columns: {line}\n{result}"
            );
        }
    }
}

#[test]
fn typed_assignment_padding_respects_tab_width() {
    let opts = FormatOptions {
        align_assignments: true,
        line_width: 40,
        ..Default::default()
    };
    let src = "if (A) { local <Integer> B = 1234567890123; local <Integer> Longer = 1; }\n";
    let result = checked(src, &opts);
    for line in result.lines() {
        let width: usize = line.chars().map(|c| if c == '\t' { 4 } else { 1 }).sum();
        assert!(
            width <= opts.line_width,
            "{width} columns: {line}\n{result}"
        );
    }
}

#[test]
fn assignment_groups_stop_at_blanks_comments_and_compound_operators() {
    let result = formatted(
        "local <Integer> A = 1;\nlocal <Integer> Longer = 2;\n\nB = 3;\n// separate\nLongest = 4;\nC += 1;\nD = 5;\n",
    );
    assert!(result.starts_with("local <Integer> A      = 1;\nlocal <Integer> Longer = 2;"));
    assert!(result.contains("\nB = 3;\n// separate\nLongest = 4;\nC += 1;\nD = 5;"));
}

#[test]
fn mixed_precedence_strings_and_calls_keep_their_tokens() {
    for src in [
        "if (A eq 1 or B eq 2 and C neq 3) { X = 0; }\n",
        "Ready = (A eq 1 or B eq 2) and ((C > 3 and D <= 4));\n",
        "if (Check(A, B) eq 1 and Name eq \"and = or eq\") { X = 0; }\n",
        "if (Ready and A eq 1 or not Blocked) { X = 0; }\n",
    ] {
        formatted(src);
    }
}

fn checked(src: &str, opts: &FormatOptions) -> String {
    assert!(
        m1_core::parse(src).syntax_diagnostics().is_empty(),
        "invalid fixture: {src}"
    );
    let result = format_str_with(src, opts).unwrap().output;
    assert_eq!(
        result,
        format_str_with(&result, opts).unwrap().output,
        "unstable: {src}"
    );
    assert!(m1_core::parse(&result).syntax_diagnostics().is_empty());
    fn leaves(node: m1_core::Node<'_>, out: &mut Vec<(m1_core::Kind, String)>) {
        let children = node.children();
        if children.is_empty() {
            let text = node.text();
            if !text.trim().is_empty() {
                out.push((node.kind(), text.trim().to_owned()));
            }
        } else {
            for child in children {
                leaves(child, out);
            }
        }
    }
    let mut before = Vec::new();
    let mut after = Vec::new();
    leaves(m1_core::parse(src).root(), &mut before);
    leaves(m1_core::parse(&result).root(), &mut after);
    let is_comment = |k| matches!(k, m1_core::Kind::LineComment | m1_core::Kind::BlockComment);
    let comments = |tokens: &Vec<(m1_core::Kind, String)>| {
        tokens
            .iter()
            .filter(|(k, _)| is_comment(*k))
            .map(|(_, text)| {
                text.chars()
                    .filter(|c| !c.is_whitespace())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(
        comments(&before),
        comments(&after),
        "comments changed: {src}"
    );
    before.retain(|(k, _)| !is_comment(*k));
    after.retain(|(k, _)| !is_comment(*k));
    assert_eq!(before, after, "semantic tokens changed: {src}");
    result
}

mod common;
#[test]
fn aligned_corpus_preserves_tokens_and_is_idempotent() {
    for (path, src) in common::corpus_scripts() {
        if !m1_core::parse(&src).syntax_diagnostics().is_empty() {
            continue;
        }
        for width in [88, 120] {
            let opts = FormatOptions {
                align_assignments: true,
                align_conditions: true,
                line_width: width,
                ..Default::default()
            };
            eprintln!("checking {} at {width}", path.display());
            checked(&src, &opts);
        }
    }
}

#[test]
fn typed_declarations_accept_identifier_characters_in_types() {
    assert_eq!(
        formatted("local <Foo_Bar> A = 1;\nlocal <Foo_Bar> Longer = 2;\n"),
        "local <Foo_Bar> A      = 1;\nlocal <Foo_Bar> Longer = 2;\n"
    );
}

#[test]
fn condition_delimiter_comments_stay_inside_the_parentheses() {
    for condition in [
        "/* guard */ A eq 1 and B eq 2",
        "A eq 1 and B eq 2 /* guard */",
        "// guard\nA eq 1 and B eq 2",
        "A eq 1 and B eq 2 // guard\n",
        "(/* guard */ A eq 1 and B eq 2) or C eq 3",
        "(A eq 1 and B eq 2 /* guard */) or C eq 3",
    ] {
        let src = format!("if ({condition}) {{ X = 0; }}\n");
        let result = formatted(&src);
        assert!(
            result.contains(&format!("if ({condition})")),
            "commented condition changed: {result}"
        );
    }
}

#[test]
fn initializer_delimiter_comments_stay_inside_the_statement() {
    for prefix in ["local <Boolean> Ready", "Ready"] {
        for value in [
            "/* guard */ A eq 1 and B eq 2",
            "A eq 1 and B eq 2 /* guard */",
            "// guard\nA eq 1 and B eq 2",
            "A eq 1 and B eq 2 // guard\n",
        ] {
            let src = format!("{prefix} = {value};\n");
            assert_eq!(formatted(&src), src);
        }
    }
}

#[test]
fn typed_declarations_accept_compile_time_interpolation() {
    for ty in ["Foo$(N)", "Foo_$(N)", "Foo$(N) Bar"] {
        let src = format!("local <{ty}> A = 1;\nlocal <{ty}> Longer = 2;\n");
        assert_eq!(
            formatted(&src),
            format!("local <{ty}> A      = 1;\nlocal <{ty}> Longer = 2;\n")
        );
    }
}

#[test]
fn assignment_alignment_does_not_rewrite_block_comment_contents() {
    let src = "/*\nlocal <Foo_Bar> A = 1;\nlocal <Foo_Bar> Longer = 2;\n*/\n";
    assert_eq!(formatted(src), src);
}

#[test]
fn semicolons_inside_trailing_comments_do_not_break_assignment_groups() {
    let src = "A = 1; // alpha; beta\nLonger = 2;\n";
    assert_eq!(formatted(src), "A      = 1;  // alpha; beta\nLonger = 2;\n");
}
