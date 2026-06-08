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
            // Drop off the opener's line first: the condition was emitted with no
            // trailing newline, and `emit_blank_gap` only fills *extra* blank
            // lines, not the base line break before the comment.
            self.emit_newline();
            // Anchor the blank-gap baseline to the first comment's own line so no
            // blank line is inserted between the opener and the comment. Without
            // this the gap is measured from a stale `prev_end_line` and, once the
            // opener/comment shift onto their own lines, a spurious blank appears
            // on the next format pass (non-idempotent on the real corpus).
            if let Some(first) = self.trivia.front() {
                self.prev_end_line = Some(first.source_line);
            }
            self.flush_trivia_before(lbrace);
            self.emit_indent();
            self.emit("{");
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
        match self.brace_style {
            crate::BraceStyle::Allman => {
                self.emit_newline();
                self.emit_indent();
                self.emit("else");
            }
            crate::BraceStyle::Kr => self.emit(" else"),
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
                    self.emit_block_open(true);
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
