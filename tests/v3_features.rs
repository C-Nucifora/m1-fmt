//! v3 printer features: blank-after-when (#97), align_assignments (#96),
//! reflow_comments (#95). Idempotency is asserted for each — the formatter's
//! core invariant.

use m1_fmt::{FormatOptions, format_str, format_str_with};

fn idempotent(src: &str, opts: &FormatOptions) -> String {
    let once = format_str_with(src, opts).unwrap().output;
    let twice = format_str_with(&once, opts).unwrap().output;
    assert_eq!(once, twice, "not idempotent for {src:?}");
    once
}

// ---- #97 blank line after top-level when blocks ---------------------------

#[test]
fn inserts_blank_after_top_level_when() {
    let src = "when (Mode)\n{\n\tis (Off)\n\t{\n\t\tx = 1;\n\t}\n}\ny = 2;\n";
    let out = idempotent(src, &FormatOptions::default());
    assert!(
        out.contains("}\n\ny = 2;"),
        "blank line missing after when block:\n{out}"
    );
}

#[test]
fn no_trailing_blank_when_when_ends_the_file() {
    let src = "when (Mode)\n{\n\tis (Off)\n\t{\n\t\tx = 1;\n\t}\n}\n";
    let out = idempotent(src, &FormatOptions::default());
    assert!(out.ends_with("}\n"), "no blank at EOF:\n{out}");
    assert!(!out.ends_with("\n\n"));
}

#[test]
fn existing_blank_after_when_is_kept_single() {
    let src = "when (Mode)\n{\n\tis (Off)\n\t{\n\t\tx = 1;\n\t}\n}\n\ny = 2;\n";
    let out = idempotent(src, &FormatOptions::default());
    assert!(out.contains("}\n\ny = 2;"));
    assert!(!out.contains("}\n\n\ny = 2;"));
}

#[test]
fn nested_when_inside_if_gets_no_blank() {
    // Only TOP-LEVEL when blocks are "functions"; nothing is inserted inside.
    let src = "if (a)\n{\n\twhen (Mode)\n\t{\n\t\tis (Off)\n\t\t{\n\t\t\tx = 1;\n\t\t}\n\t}\n\ty = 2;\n}\n";
    let out = idempotent(src, &FormatOptions::default());
    assert!(
        !out.contains("\t}\n\n\ty = 2;"),
        "no blank for nested when:\n{out}"
    );
}

#[test]
fn blank_after_when_in_kr_style() {
    let opts = FormatOptions {
        brace_style: m1_fmt::BraceStyle::Kr,
        ..Default::default()
    };
    let src = "when (Mode)\n{\n\tis (Off)\n\t{\n\t\tx = 1;\n\t}\n}\ny = 2;\n";
    let out = idempotent(src, &opts);
    assert!(out.contains("}\n\ny = 2;"), "K&R too:\n{out}");
}

// ---- brace counting ignores comment/string context (regression) -----------
// The blank-line normalize passes count `{`/`}` to track block depth. Braces
// that appear inside `//` comments or string literals are not code and must be
// excluded, or the depth accounting drifts and blank lines land in the wrong
// place (corpus: CAN.Tertiary Transcieve 200Hz.m1scr SBG section).

#[test]
fn stray_close_brace_in_comment_does_not_split_when_block() {
    // A `}` inside a comment inside a top-level when block must NOT make the
    // depth hit 0 early and insert a spurious blank line inside the block.
    let src = "when (Mode)\n{\n\tis (Off)\n\t{\n\t\t// disabled: extra closing }\n\t\tx = 1;\n\t}\n}\ny = 2;\n";
    let out = idempotent(src, &FormatOptions::default());
    assert!(
        !out.contains("\t}\n\n}"),
        "spurious blank inserted inside when block:\n{out}"
    );
    // The blank belongs AFTER the when block.
    assert!(
        out.contains("}\n\ny = 2;"),
        "blank line missing after when block:\n{out}"
    );
}

#[test]
fn stray_open_brace_in_comment_still_gets_blank_after_when() {
    // A `{` inside a comment must NOT inflate depth and suppress the required
    // blank after the top-level when block.
    let src = "when (Mode)\n{\n\tis (Off)\n\t{\n\t\t// opening brace { mentioned\n\t\tx = 1;\n\t}\n}\ny = 2;\n";
    let out = idempotent(src, &FormatOptions::default());
    assert!(
        out.contains("}\n\ny = 2;"),
        "blank line missing after when block:\n{out}"
    );
}

#[test]
fn brace_in_string_literal_does_not_corrupt_depth() {
    // An UNBALANCED brace inside a string literal is not a block delimiter and
    // must not throw off the depth accounting.
    let src = "when (Mode)\n{\n\tis (Off)\n\t{\n\t\tName = \"close } here\";\n\t}\n}\ny = 2;\n";
    let out = idempotent(src, &FormatOptions::default());
    assert!(
        !out.contains("\t}\n\n}"),
        "spurious blank inserted inside when block:\n{out}"
    );
    assert!(
        out.contains("}\n\ny = 2;"),
        "blank line missing after when block:\n{out}"
    );
}

#[test]
fn comment_ending_in_brace_is_not_a_block_opener() {
    // strip_brace_adjacent_blanks must not treat a comment whose text ends in
    // `{` as a block opener and delete the author's following blank line.
    let src = "x = 1;\n// some comment ending in a brace {\n\ny = 2;\n";
    let out = idempotent(src, &FormatOptions::default());
    assert!(
        out.contains("brace {\n\ny = 2;"),
        "author blank after comment was deleted:\n{out}"
    );
}

// ---- #96 align_assignments (opt-in) ----------------------------------------

fn align_opts() -> FormatOptions {
    FormatOptions {
        align_assignments: true,
        ..Default::default()
    }
}

#[test]
fn aligns_a_simple_run() {
    let src = "A = 1;\nLong Name = 2;\nMid = 3;\n";
    let out = idempotent(src, &align_opts());
    assert_eq!(out, "A         = 1;\nLong Name = 2;\nMid       = 3;\n");
}

#[test]
fn off_by_default() {
    let src = "A = 1;\nLong Name = 2;\n";
    let out = format_str(src).unwrap().output;
    assert_eq!(out, src, "no alignment without the opt-in");
}

#[test]
fn blank_line_breaks_the_group() {
    let src = "A = 1;\nLong Name = 2;\n\nB = 3;\nC = 4;\n";
    let out = idempotent(src, &align_opts());
    assert!(out.contains("A         = 1;"));
    assert!(
        out.contains("\nB = 3;"),
        "second group aligns separately:\n{out}"
    );
}

#[test]
fn compound_assignment_is_not_aligned() {
    let src = "A = 1;\nLong Name += 2;\nB = 3;\n";
    let out = idempotent(src, &align_opts());
    assert!(
        out.contains("Long Name += 2;"),
        "compound untouched:\n{out}"
    );
    // A and B are separated by the compound line — no shared group.
    assert!(out.contains("A = 1;") || out.contains("A  = 1;"));
}

#[test]
fn indented_groups_align_within_their_block() {
    let src = "if (a)\n{\n\tX = 1;\n\tLonger = 2;\n}\n";
    let out = idempotent(src, &align_opts());
    assert!(
        out.contains("\tX      = 1;\n\tLonger = 2;"),
        "block-scoped alignment:\n{out}"
    );
}

#[test]
fn group_that_would_overflow_is_left_alone() {
    let long_lhs = "L".repeat(80);
    let src = format!("{long_lhs} = 1;\nA = 22222;\n");
    let out = idempotent(&src, &align_opts());
    assert!(
        out.contains("\nA = 22222;"),
        "overflow group skipped:\n{out}"
    );
}

// ---- #95 reflow_comments (opt-in) ------------------------------------------

fn reflow_opts() -> FormatOptions {
    FormatOptions {
        reflow_comments: true,
        ..Default::default()
    }
}

#[test]
fn splits_an_over_width_line_comment() {
    let long = format!("// {}\nx = 1;\n", "word ".repeat(30).trim_end());
    let out = idempotent(&long, &reflow_opts());
    for line in out.lines() {
        assert!(line.chars().count() <= 88, "still over width: {line:?}");
    }
    assert!(
        out.matches("// ").count() >= 2,
        "split into several:\n{out}"
    );
}

#[test]
fn short_comment_lines_are_never_joined() {
    let src = "// one\n// two\nx = 1;\n";
    let out = idempotent(src, &reflow_opts());
    assert!(out.contains("// one\n// two\n"), "no joining:\n{out}");
}

#[test]
fn annotation_comments_are_never_split() {
    let ann = format!(
        "// @m1:allow(L001) {}\nx = 1;\n",
        "padding ".repeat(20).trim_end()
    );
    let out = format_str_with(&ann, &reflow_opts()).unwrap().output;
    assert_eq!(
        out.lines().filter(|l| l.contains("@m1:allow")).count(),
        1,
        "annotation must stay on one line:\n{out}"
    );
}

#[test]
fn reflow_off_by_default() {
    let long = format!("// {}\nx = 1;\n", "word ".repeat(30).trim_end());
    let out = format_str(&long).unwrap().output;
    assert_eq!(
        out.lines().filter(|l| l.starts_with("// ")).count(),
        1,
        "no reflow without the opt-in"
    );
}

// ---- #98 deepest-statement range formatting --------------------------------

#[test]
fn range_format_snaps_to_the_inner_statement() {
    // Formatting line 4 (the messy assignment) must NOT reformat the whole
    // when block — only that statement, re-indented to its original depth.
    let src = "when (Mode)\n{\n\tis (Off)\n\t{\n\t\tx=1   ;\n\t\ty   =2;\n\t}\n}\n";
    let r = m1_fmt::format_range(src, 4, 4, &FormatOptions::default())
        .unwrap()
        .expect("range must format");
    assert_eq!(r.start_line, 4);
    assert_eq!(r.end_line, 4);
    assert_eq!(r.output, "\t\tx = 1;\n", "re-indented inner statement");
}

#[test]
fn range_touching_a_header_falls_back_to_top_level() {
    let src = "when (Mode)\n{\n\tis (Off)\n\t{\n\t\tx=1;\n\t}\n}\n";
    // Line 2 is the `is (Off)` header — no statement run covers it; the
    // top-level when statement is the snap target.
    let r = m1_fmt::format_range(src, 2, 2, &FormatOptions::default())
        .unwrap()
        .expect("falls back to the top-level statement");
    assert_eq!((r.start_line, r.end_line), (0, 6));
}

#[test]
fn nested_range_respects_width_after_reindent() {
    let long_rhs = "Aaaa + ".repeat(12);
    let src = format!(
        "when (Mode)\n{{\n\tis (Off)\n\t{{\n\t\tTarget Channel = {}Bbbb;\n\t}}\n}}\n",
        long_rhs
    );
    let r = m1_fmt::format_range(&src, 4, 4, &FormatOptions::default())
        .unwrap()
        .expect("range must format");
    for line in r.output.lines() {
        let cols: usize = line.chars().map(|c| if c == '\t' { 4 } else { 1 }).sum();
        assert!(
            cols <= 88,
            "re-indented line over budget ({cols}): {line:?}"
        );
    }
}
