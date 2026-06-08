//! Statement-level printing for the [`Printer`](super::Printer).
//!
//! Top-level / block statement dispatch (own-line trivia, indentation, the
//! trailing EOL comment), plus the simple statements: local declarations, type
//! annotations, assignments and expression statements.

use super::{Printer, ends_with_semicolon};
use m1_core::{Kind, Node};

impl Printer {
    // ---- Source file ------------------------------------------------------

    pub(super) fn print_source_file(&mut self, root: Node) {
        let children = root.children();
        Self::print_statement_lines(self, &children);
        self.flush_remaining_trivia();
    }

    /// Print a run of sibling statements, computing each statement's EOL-comment
    /// byte ceiling (the start byte of the next non-comment statement) so a
    /// statement only claims a trailing comment that falls in its own slot.
    pub(super) fn print_statement_lines(&mut self, children: &[Node]) {
        for (i, child) in children.iter().enumerate() {
            // The next statement on the same line bounds this statement's EOL
            // comment window; comments are not statements, so skip past them.
            let next_start = children[i + 1..]
                .iter()
                .find(|c| !matches!(c.kind(), Kind::LineComment | Kind::BlockComment))
                .map(|c| c.byte_range().start)
                .unwrap_or(usize::MAX);
            self.print_statement_line(*child, next_start);
        }
    }

    /// A statement at file scope or inside a block: handles own-line trivia
    /// injection, indentation, the statement, and a trailing EOL comment. The
    /// handling is identical at file scope and inside a block. `next_start` is
    /// the byte offset where the next sibling statement begins (or `usize::MAX`
    /// for the last one), used to scope the trailing-comment window.
    pub(super) fn print_statement_line(&mut self, node: Node, next_start: usize) {
        // Comments are extras: they may show up as direct children. Skip them;
        // they are handled via the trivia list.
        if matches!(node.kind(), Kind::LineComment | Kind::BlockComment) {
            return;
        }
        let start = node.byte_range().start;
        let start_line = node.range().start.line as usize;
        let end_line = node.range().end.line as usize;
        let end_byte = node.byte_range().end;
        self.inject_trivia_before(start);
        self.emit_blank_gap(start_line);
        self.emit_indent();
        // Reserve the trailing `;` (statements that carry one) plus any EOL
        // comment, so a line that is over-budget only because of them wraps.
        let semi = usize::from(ends_with_semicolon(node.kind()));
        self.eol_reserve = self.pending_eol_width(end_line, end_byte, next_start) + semi;
        self.print_statement(node);
        self.eol_reserve = 0;
        let eol = self.take_eol_comment(end_line, end_byte, next_start);
        self.emit_eol(eol);
        self.emit_newline();
        self.prev_end_line = Some(end_line);
    }

    pub(super) fn print_statement(&mut self, node: Node) {
        match node.kind() {
            Kind::LocalDeclaration => self.print_local_decl(node),
            Kind::AssignmentStatement => self.print_assignment(node),
            Kind::ExpressionStatement => self.print_expression_stmt(node),
            Kind::IfStatement => self.print_if(node),
            Kind::WhenStatement => self.print_when(node),
            Kind::ExpandStatement => self.print_expand(node),
            Kind::Block => self.print_bare_block(node),
            // A stray bare semicolon: preserved verbatim to keep the token
            // sequence identical (semantic-preservation invariant).
            Kind::EmptyStatement => self.emit(";"),
            _ => self.emit_verbatim(node),
        }
    }

    pub(super) fn emit_verbatim(&mut self, node: Node) {
        self.emit(node.text().trim_end());
    }

    // ---- Simple statements ------------------------------------------------

    pub(super) fn print_local_decl(&mut self, node: Node) {
        // Children in order: optional `static`, `local`, optional
        // type_annotation, name, optional (`=` value), `;`.
        for child in node.children() {
            match child.kind() {
                Kind::Static => {
                    self.emit("static ");
                }
                Kind::Local => {
                    self.emit("local");
                }
                Kind::TypeAnnotation => {
                    self.emit(" ");
                    self.print_type_annotation(child);
                }
                Kind::Identifier => {
                    self.emit(" ");
                    self.emit(child.text());
                }
                Kind::Assign => {
                    self.emit(" = ");
                }
                Kind::Semicolon => {
                    self.emit(";");
                }
                Kind::LineComment | Kind::BlockComment => {}
                // The value expression (any expression kind).
                _ => {
                    self.emit_expr(child);
                }
            }
        }
    }

    pub(super) fn print_type_annotation(&mut self, node: Node) {
        // `<` identifier `>`
        self.emit("<");
        for child in node.children() {
            if child.kind() == Kind::Identifier {
                self.emit(child.text());
            }
        }
        self.emit(">");
    }

    pub(super) fn print_assignment(&mut self, node: Node) {
        // target operator value ;
        for child in node.children() {
            match child.kind() {
                // Plain `=` plus the compound assignments (`+=`, `<<=`, …). The
                // compound set comes from the shared m1-core predicate so it stays
                // in lock-step with the grammar; `Kind::Assign` is not a compound
                // assignment, so it is matched explicitly.
                Kind::Assign => {
                    self.emit(" ");
                    self.emit(child.text());
                    self.emit(" ");
                }
                k if m1_core::is_compound_assign(k) => {
                    self.emit(" ");
                    self.emit(child.text());
                    self.emit(" ");
                }
                Kind::Semicolon => self.emit(";"),
                Kind::LineComment | Kind::BlockComment => {}
                _ => self.emit_expr(child),
            }
        }
    }

    pub(super) fn print_expression_stmt(&mut self, node: Node) {
        for child in node.children() {
            match child.kind() {
                Kind::Semicolon => self.emit(";"),
                Kind::LineComment | Kind::BlockComment => {}
                _ => self.emit_expr(child),
            }
        }
    }
}
