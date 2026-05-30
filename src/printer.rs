use crate::trivia::{collect_trivia, format_line_comment, is_eol_comment, TriviaItem};
use m1_core::{Cst, Kind, Node};
use std::collections::VecDeque;

pub struct Printer {
    indent: usize,
    output: String,
    trivia: VecDeque<TriviaItem>,
}

impl Printer {
    fn new(cst: &Cst) -> Self {
        let trivia = VecDeque::from(collect_trivia(cst));
        Self {
            indent: 0,
            output: String::new(),
            trivia,
        }
    }

    fn emit(&mut self, s: &str) {
        self.output.push_str(s);
    }

    fn emit_indent(&mut self) {
        for _ in 0..self.indent {
            self.output.push_str("    ");
        }
    }

    fn emit_newline(&mut self) {
        self.output.push('\n');
    }

    /// Emit a single own-line trivia item at the current indentation.
    fn emit_own_line_trivia(&mut self, item: &TriviaItem) {
        self.emit_indent();
        if item.text.starts_with("//") {
            self.emit(&format_line_comment(&item.text));
        } else {
            // Block comment: re-indent continuation lines.
            self.emit_block_comment(&item.text);
        }
        self.emit_newline();
    }

    /// Emit a (possibly multi-line) block comment, re-indenting continuation
    /// lines to the current depth. The first line is emitted assuming the
    /// indent has already been written by the caller.
    fn emit_block_comment(&mut self, text: &str) {
        let mut first = true;
        for line in text.split('\n') {
            if !first {
                self.emit_newline();
                let trimmed = line.trim_start();
                if !trimmed.is_empty() {
                    self.emit_indent();
                    // Conventional block-comment continuation: align ` *` under
                    // the opening `/*`.
                    if trimmed.starts_with('*') {
                        self.emit(" ");
                    }
                    self.emit(trimmed);
                }
            } else {
                self.emit(line.trim_end());
                first = false;
            }
        }
    }

    /// Consume and emit, as own-line comments, all trivia whose byte offset is
    /// before `before_byte`. Trivia that is positioned before the statement
    /// always lands on its own line (true EOL comments are attached after the
    /// statement is printed, via [`Printer::take_eol_comment`]).
    fn inject_trivia_before(&mut self, before_byte: usize) {
        while let Some(item) = self.trivia.front() {
            if item.byte_offset >= before_byte {
                break;
            }
            let item = self.trivia.pop_front().unwrap();
            self.emit_own_line_trivia(&item);
        }
    }

    /// If the next pending trivia item is on `stmt_end_line` (i.e. it trails the
    /// statement we just printed), consume and return it as an EOL comment.
    fn take_eol_comment(&mut self, stmt_end_line: usize) -> Option<TriviaItem> {
        if let Some(item) = self.trivia.front() {
            if is_eol_comment(item, stmt_end_line) {
                return self.trivia.pop_front();
            }
        }
        None
    }

    /// Flush all remaining trivia whose offset is before `before_byte` as
    /// own-line comments (used before a closing brace or end of file).
    fn flush_trivia_before(&mut self, before_byte: usize) {
        while let Some(item) = self.trivia.front() {
            if item.byte_offset >= before_byte {
                break;
            }
            let item = self.trivia.pop_front().unwrap();
            self.emit_own_line_trivia(&item);
        }
    }

    fn flush_remaining_trivia(&mut self) {
        while let Some(item) = self.trivia.pop_front() {
            self.emit_own_line_trivia(&item);
        }
    }

    /// If `eol` is present, append it as an end-of-line comment.
    fn emit_eol(&mut self, eol: Option<TriviaItem>) {
        if let Some(item) = eol {
            self.emit("  ");
            if item.text.starts_with("//") {
                self.emit(&format_line_comment(&item.text));
            } else {
                self.emit(item.text.trim_end());
            }
        }
    }

    // ---- Source file ------------------------------------------------------

    fn print_source_file(&mut self, root: Node) {
        for child in root.children() {
            self.print_top_statement(child);
        }
        self.flush_remaining_trivia();
    }

    /// A statement at file scope or inside a block: handles own-line trivia
    /// injection, indentation, the statement, and a trailing EOL comment.
    fn print_top_statement(&mut self, node: Node) {
        // Comments are extras: they may show up as direct children. Skip them;
        // they are handled via the trivia list.
        if matches!(node.kind(), Kind::LineComment | Kind::BlockComment) {
            return;
        }
        let start = node.byte_range().start;
        let end_line = node.range().end.line as usize;
        self.inject_trivia_before(start);
        self.emit_indent();
        self.print_statement(node);
        let eol = self.take_eol_comment(end_line);
        self.emit_eol(eol);
        self.emit_newline();
    }

    fn print_statement(&mut self, node: Node) {
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

    fn emit_verbatim(&mut self, node: Node) {
        self.emit(node.text().trim_end());
    }

    // ---- Simple statements ------------------------------------------------

    fn print_local_decl(&mut self, node: Node) {
        // Children in order: optional `static`, `local`, optional
        // type_annotation, name, optional (`=` value), `;`.
        let mut emitted_any = false;
        for child in node.children() {
            match child.kind() {
                Kind::Static => {
                    self.emit("static ");
                }
                Kind::Local => {
                    self.emit("local");
                    emitted_any = true;
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
        let _ = emitted_any;
    }

    fn print_type_annotation(&mut self, node: Node) {
        // `<` identifier `>`
        self.emit("<");
        for child in node.children() {
            if child.kind() == Kind::Identifier {
                self.emit(child.text());
            }
        }
        self.emit(">");
    }

    fn print_assignment(&mut self, node: Node) {
        // target operator value ;
        for child in node.children() {
            match child.kind() {
                Kind::Assign | Kind::PlusEq | Kind::MinusEq | Kind::StarEq | Kind::SlashEq => {
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

    fn print_expression_stmt(&mut self, node: Node) {
        for child in node.children() {
            match child.kind() {
                Kind::Semicolon => self.emit(";"),
                Kind::LineComment | Kind::BlockComment => {}
                _ => self.emit_expr(child),
            }
        }
    }

    // ---- Expressions ------------------------------------------------------

    fn emit_expr(&mut self, node: Node) {
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

    fn emit_member(&mut self, node: Node) {
        // object `.` property — no spaces around the dot.
        for child in node.children() {
            match child.kind() {
                Kind::Dot => self.emit("."),
                Kind::LineComment | Kind::BlockComment => {}
                _ => self.emit_expr(child),
            }
        }
    }

    fn emit_call(&mut self, node: Node) {
        // function argument_list — no space before `(`.
        for child in node.children() {
            match child.kind() {
                Kind::ArgumentList => self.emit_arg_list(child),
                Kind::LineComment | Kind::BlockComment => {}
                _ => self.emit_expr(child),
            }
        }
    }

    fn emit_arg_list(&mut self, node: Node) {
        // `(` expr (`,` expr)* `)` — no padding, comma + single space.
        self.emit("(");
        let mut first = true;
        for child in node.children() {
            match child.kind() {
                Kind::LParen | Kind::RParen => {}
                Kind::Comma => self.emit(", "),
                Kind::LineComment | Kind::BlockComment => {}
                _ => {
                    let _ = first;
                    first = false;
                    self.emit_expr(child);
                }
            }
        }
        self.emit(")");
    }

    fn emit_unary(&mut self, node: Node) {
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

    fn emit_binary(&mut self, node: Node) {
        // left operator right — spaces around the operator.
        let mut parts: Vec<Node> = Vec::new();
        for child in node.children() {
            if matches!(child.kind(), Kind::LineComment | Kind::BlockComment) {
                continue;
            }
            parts.push(child);
        }
        for (i, child) in parts.iter().enumerate() {
            if i == 1 {
                // operator
                self.emit(" ");
                self.emit(child.text());
                self.emit(" ");
            } else {
                self.emit_expr(*child);
            }
        }
    }

    fn emit_ternary(&mut self, node: Node) {
        // condition ? consequence : alternative
        for child in node.children() {
            match child.kind() {
                Kind::Question => self.emit(" ? "),
                Kind::Colon => self.emit(" : "),
                Kind::LineComment | Kind::BlockComment => {}
                _ => self.emit_expr(child),
            }
        }
    }

    fn emit_paren(&mut self, node: Node) {
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

    // ---- Block statements -------------------------------------------------

    /// Print a `{ ... }` block. Assumes the caller has already emitted the
    /// opening context (e.g. `if (cond) `). Emits `{`, the indented body, and
    /// the closing `}` at the current indent (without a trailing newline).
    fn print_block(&mut self, node: Node) {
        self.emit("{");
        self.emit_newline();
        self.indent += 1;
        let rbrace = self.find_rbrace(node);
        for child in node.children() {
            match child.kind() {
                Kind::LBrace | Kind::RBrace => {}
                _ => self.print_block_statement(child),
            }
        }
        // Flush any trailing comments before the closing brace.
        if let Some(end) = rbrace {
            self.flush_trivia_before(end);
        }
        self.indent -= 1;
        self.emit_indent();
        self.emit("}");
    }

    fn find_rbrace(&self, node: Node) -> Option<usize> {
        node.children()
            .into_iter()
            .find(|c| c.kind() == Kind::RBrace)
            .map(|c| c.byte_range().start)
    }

    fn print_block_statement(&mut self, node: Node) {
        if matches!(node.kind(), Kind::LineComment | Kind::BlockComment) {
            return;
        }
        let start = node.byte_range().start;
        let end_line = node.range().end.line as usize;
        self.inject_trivia_before(start);
        self.emit_indent();
        self.print_statement(node);
        let eol = self.take_eol_comment(end_line);
        self.emit_eol(eol);
        self.emit_newline();
    }

    fn print_bare_block(&mut self, node: Node) {
        self.print_block(node);
    }

    fn print_if(&mut self, node: Node) {
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
                    self.emit(") ");
                }
                Kind::Block => self.print_block(child),
                Kind::ElseClause => self.print_else_clause(child),
                Kind::LineComment | Kind::BlockComment => {}
                _ => {
                    if seen_lparen {
                        self.emit_expr(child);
                    }
                }
            }
        }
    }

    fn print_else_clause(&mut self, node: Node) {
        // `else` (if_statement | block)
        self.emit(" else ");
        for child in node.children() {
            match child.kind() {
                Kind::Else => {}
                Kind::Block => self.print_block(child),
                Kind::IfStatement => self.print_if(child),
                Kind::LineComment | Kind::BlockComment => {}
                _ => {}
            }
        }
    }

    fn print_when(&mut self, node: Node) {
        // `when` `(` subject `)` `{` is_clause* `}`
        self.emit("when (");
        let mut seen_lparen = false;
        let mut in_body = false;
        let rbrace = self.find_rbrace(node);
        for child in node.children() {
            match child.kind() {
                Kind::When => {}
                Kind::LParen => seen_lparen = true,
                Kind::RParen => {
                    self.emit(") ");
                }
                Kind::LBrace => {
                    self.emit("{");
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

    fn print_is_clause(&mut self, node: Node) {
        // `is` `(` state `)` block
        let start = node.byte_range().start;
        let end_line = node.range().end.line as usize;
        self.inject_trivia_before(start);
        self.emit_indent();
        self.emit("is (");
        let mut seen_lparen = false;
        for child in node.children() {
            match child.kind() {
                Kind::Is => {}
                Kind::LParen => seen_lparen = true,
                Kind::RParen => self.emit(") "),
                Kind::Block => self.print_block(child),
                Kind::LineComment | Kind::BlockComment => {}
                _ => {
                    if seen_lparen {
                        self.emit_expr(child);
                    }
                }
            }
        }
        let eol = self.take_eol_comment(end_line);
        self.emit_eol(eol);
        self.emit_newline();
    }

    fn print_expand(&mut self, node: Node) {
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
                Kind::RParen => self.emit(") "),
                Kind::Block => self.print_block(child),
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

pub fn print(cst: &Cst) -> String {
    let mut p = Printer::new(cst);
    p.print_source_file(cst.root());
    normalize_trailing(&mut p.output);
    p.output
}

/// Ensure exactly one final newline and collapse 3+ consecutive blank lines
/// to 2.
fn normalize_trailing(output: &mut String) {
    // Collapse runs of 3+ blank lines to 2.
    collapse_blank_lines(output);
    // Trim trailing blank lines, then ensure a single final newline.
    while output.ends_with("\n\n") {
        output.pop();
    }
    if output.is_empty() {
        return;
    }
    if !output.ends_with('\n') {
        output.push('\n');
    }
}

fn collapse_blank_lines(output: &mut String) {
    let mut result = String::with_capacity(output.len());
    let mut blank_run = 0usize;
    for line in output.split_inclusive('\n') {
        let content = line.strip_suffix('\n').unwrap_or(line);
        if content.trim().is_empty() {
            blank_run += 1;
            if blank_run <= 2 {
                result.push_str(line);
            }
        } else {
            blank_run = 0;
            result.push_str(line);
        }
    }
    *output = result;
}
