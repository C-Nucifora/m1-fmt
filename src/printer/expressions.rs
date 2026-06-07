//! Expression printing for the [`Printer`](super::Printer).
//!
//! The flat-vs-wrap emitters for every expression kind: member access, calls and
//! argument lists, unary, binary chains (with precedence-aware flattening),
//! ternaries and parenthesized expressions.

use super::{Printer, binary_op_prec, op_prec};
use m1_core::{Kind, Node};

impl Printer {
    // ---- Expressions ------------------------------------------------------

    pub(super) fn emit_expr(&mut self, node: Node) {
        match node.kind() {
            Kind::Identifier | Kind::Number | Kind::String | Kind::Boolean => {
                self.emit(node.text());
            }
            Kind::MemberExpression => self.emit_member(node),
            Kind::CallExpression => self.emit_call(node),
            Kind::UnaryExpression => self.emit_unary(node),
            Kind::BinaryExpression => self.emit_binary(node),
            Kind::TernaryExpression => self.emit_ternary(node),
            Kind::ParenthesizedExpression => self.emit_paren(node),
            // `true`/`false` may surface as bare keyword tokens.
            Kind::True | Kind::False => self.emit(node.text()),
            _ => self.emit_verbatim(node),
        }
    }

    /// Emit the flat (single-line) form of an expression. This is the
    /// measurement / flat-render path and it must never invoke the wrap decision
    /// of a descendant, or the nested-wrap cost goes exponential (#64). Container
    /// kinds whose normal emitter would re-decide wrapping (call, binary, ternary)
    /// are rendered once through the memoizing cache; the remaining recursive
    /// containers (member, unary, paren) recurse flatly; leaf kinds delegate to
    /// the ordinary emitter (it never wraps).
    pub(super) fn emit_expr_flat(&mut self, node: Node) {
        match node.kind() {
            Kind::CallExpression => {
                let flat = self.flat_of(node, |p| p.emit_call_flat(node));
                self.emit(&flat);
            }
            Kind::BinaryExpression => {
                let flat = self.flat_of(node, |p| p.emit_binary_flat(node));
                self.emit(&flat);
            }
            Kind::TernaryExpression => {
                let flat = self.flat_of(node, |p| p.emit_ternary_flat(node));
                self.emit(&flat);
            }
            Kind::MemberExpression => {
                for child in node.children() {
                    match child.kind() {
                        Kind::Dot => self.emit("."),
                        Kind::LineComment | Kind::BlockComment => {}
                        _ => self.emit_expr_flat(child),
                    }
                }
            }
            Kind::UnaryExpression => {
                for child in node.children() {
                    match child.kind() {
                        Kind::Minus | Kind::Bang => self.emit(child.text()),
                        Kind::Not => self.emit("not "),
                        Kind::LineComment | Kind::BlockComment => {}
                        _ => self.emit_expr_flat(child),
                    }
                }
            }
            Kind::ParenthesizedExpression => {
                self.emit("(");
                for child in node.children() {
                    match child.kind() {
                        Kind::LParen | Kind::RParen => {}
                        Kind::LineComment | Kind::BlockComment => {}
                        _ => self.emit_expr_flat(child),
                    }
                }
                self.emit(")");
            }
            _ => self.emit_expr(node),
        }
    }

    /// Flat form of a call (`function(args)`), measuring the argument list flatly
    /// rather than via the wrap-deciding [`Printer::emit_arg_list`].
    pub(super) fn emit_call_flat(&mut self, node: Node) {
        for child in node.children() {
            match child.kind() {
                Kind::ArgumentList => self.emit_arg_list_flat(child),
                Kind::LineComment | Kind::BlockComment => {}
                _ => self.emit_expr_flat(child),
            }
        }
    }

    pub(super) fn emit_member(&mut self, node: Node) {
        // object `.` property — no spaces around the dot.
        for child in node.children() {
            match child.kind() {
                Kind::Dot => self.emit("."),
                Kind::LineComment | Kind::BlockComment => {}
                _ => self.emit_expr(child),
            }
        }
    }

    pub(super) fn emit_call(&mut self, node: Node) {
        // function argument_list — no space before `(`.
        for child in node.children() {
            match child.kind() {
                Kind::ArgumentList => self.emit_arg_list(child),
                Kind::LineComment | Kind::BlockComment => {}
                _ => self.emit_expr(child),
            }
        }
    }

    pub(super) fn emit_arg_list(&mut self, node: Node) {
        let start_col = self.current_col();
        let flat = self.flat_of(node, |p| p.emit_arg_list_flat(node));
        if self.exceeds_limit(start_col, &flat) {
            self.emit_arg_list_wrapped(node);
        } else {
            self.emit(&flat);
        }
    }

    /// `(` expr (`,` expr)* `)` on a single line — no padding, comma + space.
    /// Nested expressions are emitted via their cached flat form so a flat render
    /// never re-runs the wrap decision of a descendant (#64).
    pub(super) fn emit_arg_list_flat(&mut self, node: Node) {
        self.emit("(");
        for child in node.children() {
            match child.kind() {
                Kind::LParen | Kind::RParen => {}
                Kind::Comma => self.emit(", "),
                Kind::LineComment | Kind::BlockComment => {}
                _ => self.emit_expr_flat(child),
            }
        }
        self.emit(")");
    }

    /// Wrapped argument list: greedy fill, continuation lines at +8, no trailing
    /// comma before `)`.
    pub(super) fn emit_arg_list_wrapped(&mut self, node: Node) {
        self.emit("(");
        let args: Vec<Node> = node
            .children()
            .into_iter()
            .filter(|c| {
                !matches!(
                    c.kind(),
                    Kind::LParen
                        | Kind::RParen
                        | Kind::Comma
                        | Kind::LineComment
                        | Kind::BlockComment
                )
            })
            .collect();
        let n = args.len();
        for (i, arg) in args.iter().enumerate() {
            // Measure placement from the argument's cached *flat* form, not a
            // throwaway wrapped render: only the flat width drives the break
            // decision, and emitting the argument is a single descent that
            // re-runs its own wrap decision at the chosen column. Rendering a
            // wrapped trial here just to measure made the wrapped path O(N^2)
            // over a nested-call tower (#64).
            let flat = self.flat_of(*arg, |p| p.emit_expr_flat(*arg));
            let first_line = flat.split('\n').next().unwrap_or(&flat).chars().count();
            let last = i + 1 == n;
            // The last argument's line also carries `)`, the trailing `;`, and
            // any EOL comment; a non-last argument is followed by a `,`. Reserve
            // for whichever applies so the break decision is honest.
            let tail = if last {
                1 + self.eol_reserve // ")" + ("; comment" already in eol_reserve)
            } else {
                1 // ","
            };
            if i == 0 {
                // The first argument is normally emitted inline right after `(`.
                // But when call-opens have stacked up a long prefix
                // (`Outer(Inner(Innermost(`…), even the first argument's opening
                // line can blow the budget with no break point — #14. If that
                // line overflows *and* the continuation column is further left
                // than where we are, break after `(` and emit the argument at the
                // smaller column so its own nested wrapping recurses.
                let cont_col = (self.indent + self.continuation_indent) * self.indent_width;
                if self.current_col() + first_line + tail > self.width
                    && cont_col < self.current_col()
                {
                    self.emit_newline();
                    self.emit_continuation_indent();
                }
                self.emit_expr(*arg);
            } else {
                // We are after a prior arg; a "," was already emitted for it.
                // Decide: continue on this line (" " + arg) or break.
                let on_same = self.current_col() + 1 + first_line + tail;
                if on_same > self.width {
                    self.emit_newline();
                    self.emit_continuation_indent();
                } else {
                    self.emit(" ");
                }
                self.emit_expr(*arg);
            }
            if !last {
                self.emit(",");
            }
        }
        self.emit(")");
    }

    pub(super) fn emit_unary(&mut self, node: Node) {
        // operator operand. `not` is a word operator (needs a trailing space);
        // `-` and `!` bind tight to the operand.
        for child in node.children() {
            match child.kind() {
                Kind::Minus | Kind::Bang => self.emit(child.text()),
                Kind::Not => self.emit("not "),
                Kind::LineComment | Kind::BlockComment => {}
                _ => self.emit_expr(child),
            }
        }
    }

    pub(super) fn emit_binary(&mut self, node: Node) {
        let start_col = self.current_col();
        let flat = self.flat_of(node, |p| p.emit_binary_flat(node));
        if self.exceeds_limit(start_col, &flat) {
            self.emit_binary_wrapped(node);
        } else {
            self.emit(&flat);
        }
    }

    pub(super) fn emit_binary_flat(&mut self, node: Node) {
        let parts: Vec<Node> = node
            .children()
            .into_iter()
            .filter(|c| !matches!(c.kind(), Kind::LineComment | Kind::BlockComment))
            .collect();
        for (i, child) in parts.iter().enumerate() {
            if i == 1 {
                self.emit(" ");
                self.emit(child.text());
                self.emit(" ");
            } else {
                // Flat children via the cache, not the wrap decision (#64).
                self.emit_expr_flat(*child);
            }
        }
    }

    /// Flatten a left-associative binary chain into the first operand followed
    /// by (operator, operand) pairs, so we can break before each operator.
    pub(super) fn flatten_binary<'a>(
        &self,
        node: Node<'a>,
        ops: &mut Vec<(Node<'a>, Node<'a>)>,
        first: &mut Option<Node<'a>>,
    ) {
        let parts: Vec<Node> = node
            .children()
            .into_iter()
            .filter(|c| !matches!(c.kind(), Kind::LineComment | Kind::BlockComment))
            .collect();
        // parts == [left, operator, right]
        let left = parts[0];
        let op = parts[1];
        let right = parts[2];
        // Only flatten a chain at the SAME operator precedence. A higher-precedence
        // sub-expression (e.g. `a eq b` nested under an `and`) stays an intact
        // operand, so its operator is never orphaned onto a continuation line.
        if left.kind() == Kind::BinaryExpression && binary_op_prec(left) == op_prec(op.text()) {
            self.flatten_binary(left, ops, first);
        } else if first.is_none() {
            *first = Some(left);
        }
        ops.push((op, right));
    }

    pub(super) fn emit_binary_wrapped(&mut self, node: Node) {
        let mut ops: Vec<(Node, Node)> = Vec::new();
        let mut first: Option<Node> = None;
        self.flatten_binary(node, &mut ops, &mut first);
        // Emit the first operand.
        if let Some(f) = first {
            self.emit_expr(f);
        }
        // Then each "op operand", breaking before the operator when the pair
        // would overflow the current line.
        let n = ops.len();
        for (idx, (op, operand)) in ops.into_iter().enumerate() {
            let op_text = op.text().to_string();
            // Measure placement from the operand's cached *flat* first line, not a
            // throwaway wrapped render: the operand is emitted by a single descent
            // that re-runs its own wrap decision at the chosen column, so the
            // wrapped trial that made this path O(N^2) over a deep binary tower is
            // no longer needed (#64).
            let flat = self.flat_of(operand, |p| p.emit_expr_flat(operand));
            // The last operand's line also carries the trailing `;` and any EOL
            // comment; reserve for them on the final pair only.
            let tail = if idx + 1 == n {
                1 + self.eol_reserve
            } else {
                0
            };
            let first_line = flat.split('\n').next().unwrap_or(&flat).chars().count();
            let same_line =
                self.current_col() + 1 + op_text.chars().count() + 1 + first_line + tail;
            if same_line > self.width {
                // Break before the operator; emit the operand at the continuation
                // column so any nested wrapping uses the correct (smaller) column.
                self.emit_newline();
                self.emit_continuation_indent();
                self.emit(&op_text);
                self.emit(" ");
                self.emit_expr(operand);
            } else {
                self.emit(" ");
                self.emit(&op_text);
                self.emit(" ");
                self.emit_expr(operand);
            }
        }
    }

    pub(super) fn emit_ternary(&mut self, node: Node) {
        let start_col = self.current_col();
        let flat = self.flat_of(node, |p| p.emit_ternary_flat(node));
        if self.exceeds_limit(start_col, &flat) {
            self.emit_ternary_wrapped(node);
        } else {
            self.emit(&flat);
        }
    }

    /// `condition ? consequence : alternative` on one line.
    pub(super) fn emit_ternary_flat(&mut self, node: Node) {
        for child in node.children() {
            match child.kind() {
                Kind::Question => self.emit(" ? "),
                Kind::Colon => self.emit(" : "),
                Kind::LineComment | Kind::BlockComment => {}
                // Flat children via the cache, not the wrap decision (#64).
                _ => self.emit_expr_flat(child),
            }
        }
    }

    /// Wrapped ternary: the condition stays on the current line, then `?` and `:`
    /// each start a continuation line (per the manual's ternary layout).
    pub(super) fn emit_ternary_wrapped(&mut self, node: Node) {
        for child in node.children() {
            match child.kind() {
                Kind::Question => {
                    self.emit_newline();
                    self.emit_continuation_indent();
                    self.emit("? ");
                }
                Kind::Colon => {
                    self.emit_newline();
                    self.emit_continuation_indent();
                    self.emit(": ");
                }
                Kind::LineComment | Kind::BlockComment => {}
                _ => self.emit_expr(child),
            }
        }
    }

    pub(super) fn emit_paren(&mut self, node: Node) {
        // `(` expr `)` — no padding.
        self.emit("(");
        for child in node.children() {
            match child.kind() {
                Kind::LParen | Kind::RParen => {}
                Kind::LineComment | Kind::BlockComment => {}
                _ => self.emit_expr(child),
            }
        }
        self.emit(")");
    }
}
