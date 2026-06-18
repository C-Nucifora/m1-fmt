//! Control-flow and block printing for the [`Printer`](super::Printer).
//!
//! Brace placement (Allman / K&R), blocks, and the `if` / `else` / `when` /
//! `is` / `expand` constructs.

use super::Printer;
use m1_core::{Kind, Node};

impl Printer {
    // ---- Block statements -------------------------------------------------

    /// Emit the opening brace of a block. When `attached` (the block follows an
    /// inline opener such as `if (cond)`), the Allman style puts the brace on its
    /// own line aligned with the keyword, while K&R appends ` {`. A standalone
    /// (bare) block just emits `{` at the already-written indent.
    pub(super) fn emit_block_open(&mut self, attached: bool) {
        if !attached {
            self.emit("{");
            return;
        }
        match self.brace_style {
            crate::BraceStyle::Allman => {
                self.emit_newline();
                self.emit_indent();
                self.emit("{");
            }
            crate::BraceStyle::Kr => self.emit(" {"),
        }
    }

    /// Flush a leading own-line comment that sits between an inline opener and
    /// the `token` that follows it (an opening `{`, or an `else` keyword), then
    /// emit that token on its own line at the current indent.
    ///
    /// The opener's line was emitted with no trailing newline, so we first drop
    /// onto a fresh line (`emit_blank_gap` only fills *extra* blank lines, not
    /// the base break before the comment). We then anchor the blank-gap baseline
    /// to the first pending comment's own source line so no blank line is
    /// inserted between the opener and the comment: without this the gap is
    /// measured from a stale `prev_end_line` and, once the opener/comment shift
    /// onto their own lines, a spurious blank appears on the next format pass
    /// (non-idempotent on the real corpus — #76 / #126). Finally we flush the
    /// trivia ahead of `before_byte`, re-indent, and emit `token` on its own
    /// line (so even under K&R the token is not glued onto the comment).
    ///
    /// The distinct guard that decides *whether* a leading comment is present —
    /// and the no-comment fallback (`emit_block_open` vs the Allman `else`
    /// match) — stays at each call site, since those differ per construct.
    pub(super) fn flush_leading_comment_then(&mut self, before_byte: usize, token: &str) {
        self.emit_newline();
        if let Some(first) = self.trivia.front() {
            self.prev_end_line = Some(first.source_line);
        }
        self.flush_trivia_before(before_byte);
        self.emit_indent();
        self.emit(token);
    }

    /// Width reserved on the opener's last line for what follows the `)` before
    /// the block: `) {` (3) for K&R, just `)` (1) for Allman (brace next line).
    pub(super) fn close_paren_reserve(&self) -> usize {
        match self.brace_style {
            crate::BraceStyle::Allman => 1,
            crate::BraceStyle::Kr => 3,
        }
    }

    /// Print a `{ ... }` block. The caller has emitted the opening context up to
    /// (but not including) the brace. Emits the opening brace (per brace style
    /// when `attached`), the indented body, and the closing `}` at the current
    /// indent (without a trailing newline).
    pub(super) fn print_block(&mut self, node: Node, attached: bool) {
        // A comment between an inline opener (`if (cond)`, `is (X)`, …) and its
        // `{` belongs on its own line *before* the brace — not relocated inside
        // the block (#76). Flush any such trivia first; once a comment has gone on
        // its own line the brace must start a fresh line too (even under K&R,
        // which would otherwise glue `{` onto the comment).
        if attached
            && let Some(lbrace) = self
                .find_child_of_kind(node, Kind::LBrace)
                .map(|c| c.byte_range().start)
            && self.trivia.front().is_some_and(|t| t.byte_offset < lbrace)
        {
            self.flush_leading_comment_then(lbrace, "{");
        } else {
            self.emit_block_open(attached);
        }
        self.emit_newline();
        self.indent += 1;
        // Measure blank gaps inside the block from the opening-brace line.
        self.prev_end_line = Some(node.range().start.line as usize);
        let rbrace = self
            .find_child_of_kind(node, Kind::RBrace)
            .map(|c| c.byte_range().start);
        let stmts: Vec<Node> = node
            .children()
            .into_iter()
            .filter(|c| !matches!(c.kind(), Kind::LBrace | Kind::RBrace))
            .collect();
        self.print_statement_lines(&stmts);
        // Flush any trailing comments before the closing brace.
        if let Some(end) = rbrace {
            self.flush_trivia_before(end);
        }
        self.indent -= 1;
        self.emit_indent();
        self.emit("}");
        // Blank-gap arithmetic for whatever follows this block (a sibling
        // statement, an own-line comment, or the next `is`-clause) must be
        // measured from the closing `}` line, not the last *inner* statement.
        // Inner statements shift their source line when fmt wraps them, but the
        // `}` and any following token shift together, so the brace-relative gap
        // is invariant — fixing the non-idempotent blank before a comment that
        // precedes an `is`-clause (#60).
        if let Some(line) = self
            .find_child_of_kind(node, Kind::RBrace)
            .map(|c| c.range().start.line as usize)
        {
            self.prev_end_line = Some(line);
        }
    }

    pub(super) fn print_bare_block(&mut self, node: Node) {
        self.print_block(node, false);
    }

    pub(super) fn print_if(&mut self, node: Node) {
        // `if` `(` condition `)` block else_clause?
        self.emit("if (");
        let mut seen_lparen = false;
        for child in node.children() {
            match child.kind() {
                Kind::If => {}
                Kind::LParen => {
                    seen_lparen = true;
                }
                Kind::RParen => {
                    self.emit(")");
                }
                Kind::Block => self.print_block(child, true),
                Kind::ElseClause => self.print_else_clause(child),
                Kind::LineComment | Kind::BlockComment => {}
                _ => {
                    if seen_lparen {
                        // Reserve room for the trailing `) {` that follows the
                        // condition, so the wrap decision accounts for it.
                        let saved = self.width;
                        self.width = self.width.saturating_sub(self.close_paren_reserve());
                        self.emit_expr(child);
                        self.width = saved;
                    }
                }
            }
        }
    }

    pub(super) fn print_else_clause(&mut self, node: Node) {
        // `else` (if_statement | block). Allman puts `else` on its own line
        // after the closing brace; K&R keeps `} else` together.
        //
        // An own-line comment that sits *before* the `else` keyword must stay
        // before it — not be relocated below the keyword when `print_block`
        // later flushes pending trivia ahead of the opening brace (#126). Flush
        // any such leading trivia first, on its own line at the block indent,
        // and detach `else` from the preceding `}` so the comment is not glued
        // onto the brace line.
        let else_start = self
            .find_child_of_kind(node, Kind::Else)
            .map(|c| c.byte_range().start)
            .unwrap_or_else(|| node.byte_range().start);
        let has_leading_comment = self
            .trivia
            .front()
            .is_some_and(|t| t.byte_offset < else_start);
        if has_leading_comment {
            self.flush_leading_comment_then(else_start, "else");
        } else {
            match self.brace_style {
                crate::BraceStyle::Allman => {
                    self.emit_newline();
                    self.emit_indent();
                    self.emit("else");
                }
                crate::BraceStyle::Kr => self.emit(" else"),
            }
        }
        for child in node.children() {
            match child.kind() {
                Kind::Else => {}
                Kind::Block => self.print_block(child, true),
                Kind::IfStatement => {
                    self.emit(" ");
                    self.print_if(child);
                }
                Kind::LineComment | Kind::BlockComment => {}
                _ => {}
            }
        }
    }

    pub(super) fn print_when(&mut self, node: Node) {
        // `when` `(` subject `)` `{` is_clause* `}`
        self.emit("when (");
        let mut seen_lparen = false;
        let mut in_body = false;
        let rbrace = self
            .find_child_of_kind(node, Kind::RBrace)
            .map(|c| c.byte_range().start);
        for child in node.children() {
            match child.kind() {
                Kind::When => {}
                Kind::LParen => seen_lparen = true,
                Kind::RParen => {
                    self.emit(")");
                }
                Kind::LBrace => {
                    // A comment between the `when (subject)` opener and its `{`
                    // belongs on its own line *before* the brace — not pulled
                    // inside the block and indented ahead of the first `is`-clause
                    // (#76, extended to `when`). Mirror `print_block`'s handling.
                    let lbrace = child.byte_range().start;
                    if self.trivia.front().is_some_and(|t| t.byte_offset < lbrace) {
                        self.flush_leading_comment_then(lbrace, "{");
                    } else {
                        self.emit_block_open(true);
                    }
                    self.emit_newline();
                    self.indent += 1;
                    in_body = true;
                }
                Kind::RBrace => {}
                Kind::IsClause => {
                    self.print_is_clause(child);
                }
                Kind::LineComment | Kind::BlockComment => {}
                _ => {
                    if seen_lparen && !in_body {
                        self.emit_expr(child);
                    }
                }
            }
        }
        if let Some(end) = rbrace {
            self.flush_trivia_before(end);
        }
        self.indent -= 1;
        self.emit_indent();
        self.emit("}");
    }

    pub(super) fn print_is_clause(&mut self, node: Node) {
        // `is` `(` state `)` block
        let start = node.byte_range().start;
        let end_line = node.range().end.line as usize;
        let end_byte = node.byte_range().end;
        self.inject_trivia_before(start);
        self.emit_indent();
        self.emit("is (");
        let mut seen_lparen = false;
        for child in node.children() {
            match child.kind() {
                Kind::Is => {}
                Kind::LParen => seen_lparen = true,
                Kind::RParen => self.emit(")"),
                Kind::Block => self.print_block(child, true),
                Kind::LineComment | Kind::BlockComment => {}
                _ => {
                    if seen_lparen {
                        self.emit_expr(child);
                    }
                }
            }
        }
        let eol = self.take_eol_comment(end_line, end_byte, usize::MAX);
        self.emit_eol(eol);
        self.emit_newline();
    }

    pub(super) fn print_expand(&mut self, node: Node) {
        // `expand` `(` variable `=` start `to` end `)` block
        self.emit("expand (");
        let mut seen_lparen = false;
        let mut expr_index = 0; // 0 = variable, 1 = start, 2 = end
        for child in node.children() {
            match child.kind() {
                Kind::Expand => {}
                Kind::LParen => seen_lparen = true,
                Kind::Assign => self.emit(" = "),
                Kind::To => self.emit(" to "),
                Kind::RParen => self.emit(")"),
                Kind::Block => self.print_block(child, true),
                Kind::Identifier if seen_lparen && expr_index == 0 => {
                    self.emit(child.text());
                    expr_index += 1;
                }
                Kind::LineComment | Kind::BlockComment => {}
                _ => {
                    if seen_lparen {
                        self.emit_expr(child);
                        expr_index += 1;
                    }
                }
            }
        }
    }
}
