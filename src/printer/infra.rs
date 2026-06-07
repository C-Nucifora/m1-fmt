//! Low-level emit / trivia / measurement helpers for the [`Printer`](super::Printer).
//!
//! Indentation, raw output, blank-gap and trivia bookkeeping, the trial-render
//! and flat-cache machinery used by the wrap decision, and the small
//! child-lookup helper. Pure mechanics shared by the statement, expression and
//! control printers.

use super::Printer;
use crate::trivia::{TriviaItem, format_line_comment, is_eol_comment};
use m1_core::{Kind, Node};

impl Printer {
    /// Push `n` indent levels to the output (a tab each, or `indent_width` spaces).
    pub(super) fn push_levels(&mut self, n: usize) {
        match self.indent_style {
            crate::IndentStyle::Tab => (0..n).for_each(|_| self.output.push('\t')),
            crate::IndentStyle::Spaces => {
                (0..n * self.indent_width).for_each(|_| self.output.push(' '))
            }
        }
    }

    /// Display width of `s` in columns, expanding tabs to `indent_width`.
    pub(super) fn visual_width(&self, s: &str) -> usize {
        s.chars()
            .map(|c| if c == '\t' { self.indent_width } else { 1 })
            .sum()
    }

    /// Preserve author blank lines between the previous statement and the one
    /// starting at `start_line`. Over-runs are collapsed later by
    /// `collapse_blank_lines`; brace-adjacent blanks are stripped after that.
    pub(super) fn emit_blank_gap(&mut self, start_line: usize) {
        if let Some(prev) = self.prev_end_line
            && start_line > prev + 1
        {
            for _ in 0..(start_line - prev - 1) {
                self.emit_newline();
            }
        }
    }

    pub(super) fn emit(&mut self, s: &str) {
        self.output.push_str(s);
    }

    pub(super) fn emit_indent(&mut self) {
        let n = self.indent;
        self.push_levels(n);
    }

    pub(super) fn emit_newline(&mut self) {
        self.output.push('\n');
    }

    /// Display column of the cursor on the current (last) physical line: the
    /// visual width emitted since the most recent newline (tabs expanded).
    pub(super) fn current_col(&self) -> usize {
        let line = match self.output.rfind('\n') {
            Some(i) => &self.output[i + 1..],
            None => &self.output[..],
        };
        self.visual_width(line)
    }

    /// True if `flat`, placed starting at column `start_col`, would push the
    /// line past the configured width. Multi-line `flat` is measured by its
    /// longest constituent line (its first line offset by `start_col`).
    pub(super) fn exceeds_limit(&self, start_col: usize, flat: &str) -> bool {
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
    pub(super) fn emit_continuation_indent(&mut self) {
        let n = self.indent + self.continuation_indent;
        self.push_levels(n);
    }

    /// Render `f` into a scratch buffer at the current indent WITHOUT mutating
    /// `self.output` or consuming any trivia, and return what `f` appended.
    /// Used to measure a node's flat width before deciding whether to wrap.
    pub(super) fn trial(&mut self, f: impl FnOnce(&mut Printer)) -> String {
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
    pub(super) fn flat_of(&mut self, node: Node, render: impl FnOnce(&mut Printer)) -> String {
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
    pub(super) fn emit_own_line_trivia(&mut self, item: &TriviaItem) {
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
    pub(super) fn emit_block_comment(&mut self, text: &str) {
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
    pub(super) fn inject_trivia_before(&mut self, before_byte: usize) {
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
    pub(super) fn pending_eol_width(&self, stmt_end_line: usize) -> usize {
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
    pub(super) fn take_eol_comment(&mut self, stmt_end_line: usize) -> Option<TriviaItem> {
        if let Some(item) = self.trivia.front()
            && is_eol_comment(item, stmt_end_line)
        {
            return self.trivia.pop_front();
        }
        None
    }

    /// Flush all remaining trivia whose offset is before `before_byte` as
    /// own-line comments (used before a closing brace or end of file). Same loop
    /// as [`Printer::inject_trivia_before`]; kept as a distinct, intent-revealing
    /// name at the call sites.
    pub(super) fn flush_trivia_before(&mut self, before_byte: usize) {
        self.inject_trivia_before(before_byte);
    }

    pub(super) fn flush_remaining_trivia(&mut self) {
        while let Some(item) = self.trivia.pop_front() {
            self.emit_own_line_trivia(&item);
        }
    }

    /// If `eol` is present, append it as an end-of-line comment.
    pub(super) fn emit_eol(&mut self, eol: Option<TriviaItem>) {
        if let Some(item) = eol {
            self.emit("  ");
            if item.text.starts_with("//") {
                self.emit(&format_line_comment(&item.text));
            } else {
                self.emit(item.text.trim_end());
            }
        }
    }

    /// The first direct child of `node` with the given `kind`, if any. Callers
    /// project the result to the byte offset or line they need.
    pub(super) fn find_child_of_kind<'a>(&self, node: Node<'a>, kind: Kind) -> Option<Node<'a>> {
        node.children().into_iter().find(|c| c.kind() == kind)
    }
}
