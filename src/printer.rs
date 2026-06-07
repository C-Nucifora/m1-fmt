use crate::trivia::{TriviaItem, collect_trivia, format_line_comment, is_eol_comment};
use m1_core::{Cst, Kind, Node};
use std::collections::{HashMap, VecDeque};

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

    /// Push `n` indent levels to the output (a tab each, or `indent_width` spaces).
    fn push_levels(&mut self, n: usize) {
        match self.indent_style {
            crate::IndentStyle::Tab => (0..n).for_each(|_| self.output.push('\t')),
            crate::IndentStyle::Spaces => {
                (0..n * self.indent_width).for_each(|_| self.output.push(' '))
            }
        }
    }

    /// Display width of `s` in columns, expanding tabs to `indent_width`.
    fn visual_width(&self, s: &str) -> usize {
        s.chars()
            .map(|c| if c == '\t' { self.indent_width } else { 1 })
            .sum()
    }

    /// Preserve author blank lines between the previous statement and the one
    /// starting at `start_line`. Over-runs are collapsed later by
    /// `collapse_blank_lines`; brace-adjacent blanks are stripped after that.
    fn emit_blank_gap(&mut self, start_line: usize) {
        if let Some(prev) = self.prev_end_line
            && start_line > prev + 1
        {
            for _ in 0..(start_line - prev - 1) {
                self.emit_newline();
            }
        }
    }

    fn emit(&mut self, s: &str) {
        self.output.push_str(s);
    }

    fn emit_indent(&mut self) {
        let n = self.indent;
        self.push_levels(n);
    }

    fn emit_newline(&mut self) {
        self.output.push('\n');
    }

    /// Display column of the cursor on the current (last) physical line: the
    /// visual width emitted since the most recent newline (tabs expanded).
    fn current_col(&self) -> usize {
        let line = match self.output.rfind('\n') {
            Some(i) => &self.output[i + 1..],
            None => &self.output[..],
        };
        self.visual_width(line)
    }

    /// True if `flat`, placed starting at column `start_col`, would push the
    /// line past the configured width. Multi-line `flat` is measured by its
    /// longest constituent line (its first line offset by `start_col`).
    fn exceeds_limit(&self, start_col: usize, flat: &str) -> bool {
        let lines: Vec<&str> = flat.split('\n').collect();
        let last = lines.len() - 1;
        for (i, line) in lines.iter().enumerate() {
            let mut len = self.visual_width(line) + if i == 0 { start_col } else { 0 };
            // The pending EOL comment lands on the statement's final line.
            if i == last {
                len += self.eol_reserve;
            }
            if len > self.width {
                return true;
            }
        }
        false
    }

    /// Emit a continuation indent: the current block indent plus
    /// `continuation_indent` extra levels (default 1, per the M1 Development
    /// Manual; configurable — see [`crate::FormatOptions::continuation_indent`]).
    fn emit_continuation_indent(&mut self) {
        let n = self.indent + self.continuation_indent;
        self.push_levels(n);
    }

    /// Render `f` into a scratch buffer at the current indent WITHOUT mutating
    /// `self.output` or consuming any trivia, and return what `f` appended.
    /// Used to measure a node's flat width before deciding whether to wrap.
    fn trial(&mut self, f: impl FnOnce(&mut Printer)) -> String {
        let mark = self.output.len();
        let saved_trivia = self.trivia.clone();
        let saved_indent = self.indent;
        let saved_prev_end_line = self.prev_end_line;
        f(self);
        let rendered = self.output[mark..].to_string();
        self.output.truncate(mark);
        self.trivia = saved_trivia;
        self.indent = saved_indent;
        self.prev_end_line = saved_prev_end_line;
        rendered
    }

    /// The cached flat (single-line) rendering of an expression subtree, computed
    /// with `render` on a cache miss. Because a flat render is independent of the
    /// current column, indent, and trivia, the result can be reused at every
    /// enclosing level; this is what keeps nested wrapping from re-rendering the
    /// same subtree once per ancestor (#64). Returns an owned `String` (cheap
    /// relative to the avoided re-render) so the borrow on the cache is released.
    fn flat_of(&mut self, node: Node, render: impl FnOnce(&mut Printer)) -> String {
        let key = (node.byte_range().start, node.byte_range().end);
        if let Some(cached) = self.flat_cache.get(&key) {
            return cached.clone();
        }
        let rendered = self.trial(render);
        self.flat_cache.insert(key, rendered.clone());
        rendered
    }

    /// Emit a single own-line trivia item at the current indentation,
    /// preserving any author blank lines that precede it.
    fn emit_own_line_trivia(&mut self, item: &TriviaItem) {
        self.emit_blank_gap(item.source_line);
        self.emit_indent();
        if item.text.starts_with("//") {
            self.emit(&format_line_comment(&item.text));
        } else {
            // Block comment: re-indent continuation lines.
            self.emit_block_comment(&item.text);
        }
        self.emit_newline();
        // A block comment may span multiple source lines.
        let span = item.text.matches('\n').count();
        self.prev_end_line = Some(item.source_line + span);
    }

    /// Emit a (possibly multi-line) block comment. The first line is emitted
    /// assuming the indent has already been written by the caller.
    ///
    /// Continuation lines are emitted verbatim: a block comment's interior
    /// whitespace is load-bearing (aligned tables, ASCII diagrams, indented
    /// commented-out code) and must round-trip unchanged (#66). The only
    /// reflowing applied is to conventional ` *`-prefixed javadoc lines, which are
    /// re-aligned under the opening `/*` at the current indent; every other line
    /// keeps its original leading whitespace. (Body rewrapping is deferred to v3.)
    fn emit_block_comment(&mut self, text: &str) {
        let mut first = true;
        for line in text.split('\n') {
            if !first {
                self.emit_newline();
                let trimmed = line.trim_start();
                if trimmed.starts_with('*') {
                    // Conventional javadoc continuation: re-align ` *` under the
                    // opening `/*` at the current indent.
                    self.emit_indent();
                    self.emit(" ");
                    self.emit(trimmed.trim_end());
                } else if !line.trim().is_empty() {
                    // Any other content (tables, diagrams, indented code): keep
                    // the original interior whitespace verbatim. Only trailing
                    // whitespace is trimmed, matching the first line.
                    self.emit(line.trim_end());
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

    /// Width of the pending EOL comment for the statement ending on
    /// `stmt_end_line`, as it will be rendered (two spaces + normalized text),
    /// or 0 if none.
    fn pending_eol_width(&self, stmt_end_line: usize) -> usize {
        if let Some(item) = self.trivia.front()
            && is_eol_comment(item, stmt_end_line)
        {
            let rendered = if item.text.starts_with("//") {
                format_line_comment(&item.text)
            } else {
                item.text.trim_end().to_string()
            };
            return 2 + rendered.chars().count();
        }
        0
    }

    /// If the next pending trivia item is on `stmt_end_line` (i.e. it trails the
    /// statement we just printed), consume and return it as an EOL comment.
    fn take_eol_comment(&mut self, stmt_end_line: usize) -> Option<TriviaItem> {
        if let Some(item) = self.trivia.front()
            && is_eol_comment(item, stmt_end_line)
        {
            return self.trivia.pop_front();
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
        let start_line = node.range().start.line as usize;
        let end_line = node.range().end.line as usize;
        self.inject_trivia_before(start);
        self.emit_blank_gap(start_line);
        self.emit_indent();
        // Reserve the trailing `;` (statements that carry one) plus any EOL
        // comment, so a line that is over-budget only because of them wraps.
        let semi = usize::from(ends_with_semicolon(node.kind()));
        self.eol_reserve = self.pending_eol_width(end_line) + semi;
        self.print_statement(node);
        self.eol_reserve = 0;
        let eol = self.take_eol_comment(end_line);
        self.emit_eol(eol);
        self.emit_newline();
        self.prev_end_line = Some(end_line);
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
                Kind::Assign
                | Kind::PlusEq
                | Kind::MinusEq
                | Kind::StarEq
                | Kind::SlashEq
                | Kind::PercentEq
                | Kind::AmpEq
                | Kind::PipeEq
                | Kind::CaretEq
                | Kind::LtLtEq
                | Kind::GtGtEq => {
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

    /// Emit the flat (single-line) form of an expression. This is the
    /// measurement / flat-render path and it must never invoke the wrap decision
    /// of a descendant, or the nested-wrap cost goes exponential (#64). Container
    /// kinds whose normal emitter would re-decide wrapping (call, binary, ternary)
    /// are rendered once through the memoizing cache; the remaining recursive
    /// containers (member, unary, paren) recurse flatly; leaf kinds delegate to
    /// the ordinary emitter (it never wraps).
    fn emit_expr_flat(&mut self, node: Node) {
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
    fn emit_call_flat(&mut self, node: Node) {
        for child in node.children() {
            match child.kind() {
                Kind::ArgumentList => self.emit_arg_list_flat(child),
                Kind::LineComment | Kind::BlockComment => {}
                _ => self.emit_expr_flat(child),
            }
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
    fn emit_arg_list_flat(&mut self, node: Node) {
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
                    self.emit_expr_flat(child);
                }
            }
        }
        self.emit(")");
    }

    /// Wrapped argument list: greedy fill, continuation lines at +8, no trailing
    /// comma before `)`.
    fn emit_arg_list_wrapped(&mut self, node: Node) {
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
        let start_col = self.current_col();
        let flat = self.flat_of(node, |p| p.emit_binary_flat(node));
        if self.exceeds_limit(start_col, &flat) {
            self.emit_binary_wrapped(node);
        } else {
            self.emit(&flat);
        }
    }

    fn emit_binary_flat(&mut self, node: Node) {
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
    fn flatten_binary<'a>(
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

    fn emit_binary_wrapped(&mut self, node: Node) {
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

    fn emit_ternary(&mut self, node: Node) {
        let start_col = self.current_col();
        let flat = self.flat_of(node, |p| p.emit_ternary_flat(node));
        if self.exceeds_limit(start_col, &flat) {
            self.emit_ternary_wrapped(node);
        } else {
            self.emit(&flat);
        }
    }

    /// `condition ? consequence : alternative` on one line.
    fn emit_ternary_flat(&mut self, node: Node) {
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
    fn emit_ternary_wrapped(&mut self, node: Node) {
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

    /// Emit the opening brace of a block. When `attached` (the block follows an
    /// inline opener such as `if (cond)`), the Allman style puts the brace on its
    /// own line aligned with the keyword, while K&R appends ` {`. A standalone
    /// (bare) block just emits `{` at the already-written indent.
    fn emit_block_open(&mut self, attached: bool) {
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
    fn close_paren_reserve(&self) -> usize {
        match self.brace_style {
            crate::BraceStyle::Allman => 1,
            crate::BraceStyle::Kr => 3,
        }
    }

    /// Print a `{ ... }` block. The caller has emitted the opening context up to
    /// (but not including) the brace. Emits the opening brace (per brace style
    /// when `attached`), the indented body, and the closing `}` at the current
    /// indent (without a trailing newline).
    fn print_block(&mut self, node: Node, attached: bool) {
        // A comment between an inline opener (`if (cond)`, `is (X)`, …) and its
        // `{` belongs on its own line *before* the brace — not relocated inside
        // the block (#76). Flush any such trivia first; once a comment has gone on
        // its own line the brace must start a fresh line too (even under K&R,
        // which would otherwise glue `{` onto the comment).
        if attached
            && let Some(lbrace) = self.find_lbrace(node)
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
        // Blank-gap arithmetic for whatever follows this block (a sibling
        // statement, an own-line comment, or the next `is`-clause) must be
        // measured from the closing `}` line, not the last *inner* statement.
        // Inner statements shift their source line when fmt wraps them, but the
        // `}` and any following token shift together, so the brace-relative gap
        // is invariant — fixing the non-idempotent blank before a comment that
        // precedes an `is`-clause (#60).
        if let Some(line) = self.find_rbrace_line(node) {
            self.prev_end_line = Some(line);
        }
    }

    fn find_lbrace(&self, node: Node) -> Option<usize> {
        node.children()
            .into_iter()
            .find(|c| c.kind() == Kind::LBrace)
            .map(|c| c.byte_range().start)
    }

    fn find_rbrace(&self, node: Node) -> Option<usize> {
        node.children()
            .into_iter()
            .find(|c| c.kind() == Kind::RBrace)
            .map(|c| c.byte_range().start)
    }

    fn find_rbrace_line(&self, node: Node) -> Option<usize> {
        node.children()
            .into_iter()
            .find(|c| c.kind() == Kind::RBrace)
            .map(|c| c.range().start.line as usize)
    }

    fn print_block_statement(&mut self, node: Node) {
        if matches!(node.kind(), Kind::LineComment | Kind::BlockComment) {
            return;
        }
        let start = node.byte_range().start;
        let start_line = node.range().start.line as usize;
        let end_line = node.range().end.line as usize;
        self.inject_trivia_before(start);
        self.emit_blank_gap(start_line);
        self.emit_indent();
        // Reserve the trailing `;` (statements that carry one) plus any EOL
        // comment, so a line that is over-budget only because of them wraps.
        let semi = usize::from(ends_with_semicolon(node.kind()));
        self.eol_reserve = self.pending_eol_width(end_line) + semi;
        self.print_statement(node);
        self.eol_reserve = 0;
        let eol = self.take_eol_comment(end_line);
        self.emit_eol(eol);
        self.emit_newline();
        self.prev_end_line = Some(end_line);
    }

    fn print_bare_block(&mut self, node: Node) {
        self.print_block(node, false);
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

    fn print_else_clause(&mut self, node: Node) {
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
    strip_brace_adjacent_blanks(&mut p.output);
    normalize_trailing(&mut p.output, opts.max_blank_lines);
    p.output
}

/// Remove blank lines that immediately follow an opening `{` line or
/// immediately precede a closing `}` line, plus a leading run of blank lines at
/// the very top of the file. These never carry intent.
fn strip_brace_adjacent_blanks(output: &mut String) {
    let lines: Vec<&str> = output.split_inclusive('\n').collect();
    let is_blank = |s: &str| s.strip_suffix('\n').unwrap_or(s).trim().is_empty();
    fn trimmed_end(s: &str) -> &str {
        s.strip_suffix('\n').unwrap_or(s).trim_end()
    }
    let mut keep = vec![true; lines.len()];

    for i in 0..lines.len() {
        if !is_blank(lines[i]) {
            continue;
        }
        // Leading blanks at file top.
        let prev_nonblank = (0..i).rev().find(|&j| !is_blank(lines[j]));
        let next_nonblank = (i + 1..lines.len()).find(|&j| !is_blank(lines[j]));
        match prev_nonblank {
            None => keep[i] = false, // leading run
            Some(p) if trimmed_end(lines[p]).ends_with('{') => keep[i] = false,
            _ => {}
        }
        if let Some(n) = next_nonblank
            && (trimmed_end(lines[n]) == "}" || trimmed_end(lines[n]).starts_with('}'))
        {
            keep[i] = false;
        }
    }

    let mut result = String::with_capacity(output.len());
    for (i, line) in lines.iter().enumerate() {
        if keep[i] {
            result.push_str(line);
        }
    }
    *output = result;
}

/// Ensure exactly one final newline and collapse blank runs to `max_blank`.
fn normalize_trailing(output: &mut String, max_blank: usize) {
    collapse_blank_lines(output, max_blank);
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

fn collapse_blank_lines(output: &mut String, max_blank: usize) {
    let mut result = String::with_capacity(output.len());
    let mut blank_run = 0usize;
    for line in output.split_inclusive('\n') {
        let content = line.strip_suffix('\n').unwrap_or(line);
        if content.trim().is_empty() {
            blank_run += 1;
            if blank_run <= max_blank {
                result.push_str(line);
            }
        } else {
            blank_run = 0;
            result.push_str(line);
        }
    }
    *output = result;
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
