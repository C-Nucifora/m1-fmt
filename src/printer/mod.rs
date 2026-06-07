//! The M1 source printer, split across focused submodules.
//!
//! [`Printer`] holds all formatting state; its inherent `impl` is spread over:
//! - `infra`: emit / trivia / measurement helpers,
//! - `statements`: statement dispatch + simple statements,
//! - `expressions`: the flat-vs-wrap expression emitters,
//! - `control`: blocks and control-flow constructs,
//! - `normalize`: whole-output post-processing (free functions).
//!
//! The crate entry points are `print` and `print_with`.

use crate::trivia::{TriviaItem, collect_trivia};
use m1_core::{Cst, Kind, Node};
use std::collections::{HashMap, VecDeque};

mod control;
mod expressions;
mod infra;
mod normalize;
mod statements;

pub struct Printer {
    indent: usize,
    output: String,
    trivia: VecDeque<TriviaItem>,
    width: usize,
    /// Memoized flat rendering of an expression subtree, keyed by its byte span.
    /// The flat form of a node is invariant (single line, no indent/column/trivia
    /// dependence), so caching it makes the wrap decision measure each subtree
    /// once instead of re-rendering it at every enclosing level — turning the
    /// otherwise O(2^N) nested-wrap cost into linear work (#64).
    flat_cache: HashMap<(usize, usize), String>,
    /// Source end line of the most recently emitted statement, used to preserve
    /// author blank lines between statements.
    prev_end_line: Option<usize>,
    /// Width (in columns) the pending end-of-line comment will occupy on the
    /// final line of the current statement. Counted against the budget only for
    /// the last element of a wrapped construct (and in the flat-vs-wrap
    /// decision), so a trailing comment can force a wrap without pushing the
    /// greedy fill of earlier lines too far left.
    eol_reserve: usize,
    indent_style: crate::IndentStyle,
    indent_width: usize,
    brace_style: crate::BraceStyle,
    /// Extra indent levels for wrapped/continuation lines (default 1, per the
    /// manual; configurable). See [`crate::FormatOptions::continuation_indent`].
    continuation_indent: usize,
}

impl Printer {
    fn new(cst: &Cst, opts: &crate::FormatOptions) -> Self {
        let trivia = VecDeque::from(collect_trivia(cst));
        Self {
            indent: 0,
            output: String::new(),
            trivia,
            width: opts.line_width,
            flat_cache: HashMap::new(),
            prev_end_line: None,
            eol_reserve: 0,
            indent_style: opts.indent_style,
            indent_width: opts.indent_width,
            brace_style: opts.brace_style,
            continuation_indent: opts.continuation_indent,
        }
    }
}

/// Relative binding precedence of a binary operator (higher binds tighter),
/// mirroring the grammar's precedence table. Used to flatten only same-level
/// chains so wrapping never splits a tighter-binding sub-expression.
fn op_prec(op: &str) -> u8 {
    match op {
        "or" | "||" => 1,
        "and" | "&&" => 2,
        "|" => 3,
        "^" => 4,
        "&" => 5,
        "==" | "!=" | "eq" | "neq" => 6,
        "<" | ">" | "<=" | ">=" => 7,
        "<<" | ">>" => 8,
        "+" | "-" => 9,
        "*" | "/" | "%" => 10,
        _ => 0,
    }
}

/// Precedence of a `BinaryExpression` node's operator (its middle child).
fn binary_op_prec(node: Node) -> u8 {
    let op = node
        .children()
        .into_iter()
        .filter(|c| !matches!(c.kind(), Kind::LineComment | Kind::BlockComment))
        .nth(1)
        .map(|c| c.text().to_string())
        .unwrap_or_default();
    op_prec(&op)
}

/// Statement kinds whose printed form ends in a `;` on the same line as their
/// (potentially wrapped) expression.
fn ends_with_semicolon(kind: Kind) -> bool {
    matches!(
        kind,
        Kind::LocalDeclaration | Kind::AssignmentStatement | Kind::ExpressionStatement
    )
}

pub fn print(cst: &Cst) -> String {
    print_with(cst, &crate::FormatOptions::default())
}

pub fn print_with(cst: &Cst, opts: &crate::FormatOptions) -> String {
    let mut p = Printer::new(cst, opts);
    p.print_source_file(cst.root());
    // Strip brace-adjacent blanks BEFORE collapsing blank runs: stripping can
    // leave two previously-separated blank lines adjacent, and only a later
    // collapse pass would merge them — which made a second format pass change
    // the output (non-idempotent, #19). Doing the strip first is stable in one
    // pass.
    normalize::strip_brace_adjacent_blanks(&mut p.output);
    normalize::normalize_trailing(&mut p.output, opts.max_blank_lines);
    p.output
}

#[cfg(test)]
mod wrap_tests {
    use super::*;

    fn printer() -> Printer {
        let cst = m1_core::parse("x = 1;\n");
        Printer::new(&cst, &crate::FormatOptions::default())
    }

    #[test]
    fn current_col_counts_since_last_newline() {
        let mut p = printer();
        p.emit("abc");
        assert_eq!(p.current_col(), 3);
        p.emit_newline();
        p.emit("de");
        assert_eq!(p.current_col(), 2);
    }

    #[test]
    fn exceeds_limit_boundary() {
        let p = printer();
        // 88 chars exactly: not over. 89: over.
        let s88 = "a".repeat(88);
        let s89 = "a".repeat(89);
        assert!(!p.exceeds_limit(0, &s88));
        assert!(p.exceeds_limit(0, &s89));
        // Landing column counts toward the budget.
        assert!(p.exceeds_limit(1, &s88));
    }

    #[test]
    fn trial_does_not_mutate_output_or_trivia() {
        let cst = m1_core::parse("// c\nx = 1;\n");
        let mut p = Printer::new(&cst, &crate::FormatOptions::default());
        p.emit("before");
        let trivia_before = p.trivia.len();
        let rendered = p.trial(|p| {
            p.emit("inside");
            p.flush_remaining_trivia();
        });
        // `rendered` captures everything `f` appended (including the flushed
        // comment), but `trial` must restore `output` and `trivia` afterwards.
        assert!(rendered.starts_with("inside"));
        assert_eq!(p.output, "before");
        assert_eq!(p.trivia.len(), trivia_before);
    }

    #[test]
    fn continuation_indent_is_block_plus_one_level() {
        // #78: the manual specifies one extra indent level for continuations, so
        // at block level 1 (4 spaces) the continuation is block + one level.
        let mut p = printer();
        p.indent_style = crate::IndentStyle::Spaces;
        p.indent_width = 4;
        p.indent = 1; // 4 spaces of block indent
        p.emit_continuation_indent();
        assert_eq!(p.output, " ".repeat(4 + 4));
    }

    #[test]
    fn continuation_indent_uses_tabs_by_default() {
        let mut p = printer();
        p.indent = 1;
        p.emit_continuation_indent();
        assert_eq!(p.output, "\t".repeat(1 + 1)); // block + one level, as tabs (#78)
    }

    #[test]
    fn continuation_indent_honors_configured_levels() {
        // The +2 convention remains available via config.
        let cst = m1_core::parse("x = 1;\n");
        let mut p = Printer::new(
            &cst,
            &crate::FormatOptions {
                continuation_indent: 2,
                ..Default::default()
            },
        );
        p.indent = 1;
        p.emit_continuation_indent();
        assert_eq!(p.output, "\t".repeat(1 + 2));
    }

    // #14: nested call-opens that stack a long prefix on the opening line must
    // break after `(` rather than leaving a line over budget. The whole-pipeline
    // check is the meaningful one here.
    #[test]
    fn nested_wrapped_args_break_after_open_paren() {
        let src = "local result = Outer.Compute(Inner.Deep(Innermost.EvenDeeper(\
                   reallyQuiteLongArgumentNameOne, reallyQuiteLongArgumentNameTwo, \
                   reallyQuiteLongArgumentNameThree, reallyQuiteLongArgumentNameFour)));\n";
        let out = crate::format_str(src).unwrap().output;
        for line in out.lines() {
            assert!(
                line.chars().count() <= 88,
                "line over budget after wrapping: {line:?}"
            );
        }
        // and the result must be stable under a second pass.
        let again = crate::format_str(&out).unwrap();
        assert_eq!(again.output, out, "formatting is not idempotent");
        assert!(!again.changed);
    }
}
