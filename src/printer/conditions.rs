//! Optional boolean rows. The CST supplies operators and parentheses; no tokens
//! are invented or reordered. Alignment is scoped to consecutive comparisons.
use super::{Printer, binary_op_prec};
use m1_core::{Kind, Node};

fn parts(node: Node<'_>) -> Vec<Node<'_>> {
    node.children()
        .into_iter()
        .filter(|n| !matches!(n.kind(), Kind::LineComment | Kind::BlockComment))
        .collect()
}

fn logical(node: Node<'_>) -> bool {
    node.kind() == Kind::BinaryExpression && matches!(binary_op_prec(node), 1 | 2)
}

fn grouped(node: Node<'_>) -> Option<Node<'_>> {
    if node.kind() != Kind::ParenthesizedExpression {
        return None;
    }
    let children = parts(node);
    children
        .get(1)
        .copied()
        .filter(|n| logical(*n) || grouped(*n).is_some())
}

fn rows<'a>(node: Node<'a>, out: &mut Vec<(Node<'a>, String)>) {
    let children = parts(node);
    if logical(node) && children.len() == 3 {
        rows(children[0], out);
        out.last_mut().unwrap().1 = children[1].text().to_string();
        rows(children[2], out);
    } else {
        out.push((node, String::new()));
    }
}

impl Printer {
    /// Preserve the full interior, including comments outside the expression's
    /// CST span, such as `if (/* guard */ A and B /* tail */)`.
    pub(super) fn emit_commented_between(
        &mut self,
        node: Node<'_>,
        open: Kind,
        close: Kind,
    ) -> bool {
        if !self.align_conditions {
            return false;
        }
        let Some(left) = self.find_child_of_kind(node, open) else {
            return false;
        };
        let Some(right) = self.find_child_of_kind(node, close) else {
            return false;
        };
        let span = left.byte_range().end..right.byte_range().start;
        if !self.trivia.iter().any(|t| span.contains(&t.byte_offset)) {
            return false;
        }
        let offset = node.byte_range().start;
        self.emit(&node.text()[span.start - offset..span.end - offset]);
        self.trivia.retain(|t| !span.contains(&t.byte_offset));
        true
    }

    pub(super) fn condition_layout_enabled(&self, node: Node<'_>) -> bool {
        // Embedded comments need their existing trivia placement. Do not move
        // them to different boolean terms just to produce aligned columns.
        self.align_conditions
            && (logical(node) || grouped(node).is_some())
            && !self.commented_condition(node)
    }

    pub(super) fn commented_condition(&self, node: Node<'_>) -> bool {
        self.align_conditions
            && (logical(node) || grouped(node).is_some())
            && self
                .trivia
                .iter()
                .any(|t| node.byte_range().contains(&t.byte_offset))
    }

    pub(super) fn preserve_commented_condition(&mut self, node: Node<'_>) {
        self.emit_verbatim(node);
        self.trivia
            .retain(|t| !node.byte_range().contains(&t.byte_offset));
    }

    pub(super) fn emit_condition_rhs(&mut self, node: Node<'_>) {
        self.emit_newline();
        self.indent += self.continuation_indent;
        self.emit_condition_rows(node);
        self.indent -= self.continuation_indent;
    }

    pub(super) fn emit_condition_rows(&mut self, node: Node<'_>) {
        let mut terms = Vec::new();
        rows(node, &mut terms);
        let comparisons: Vec<Option<(String, String, String)>> = terms
            .iter()
            .map(|(term, _)| {
                let children = parts(*term);
                if term.kind() == Kind::BinaryExpression
                    && matches!(binary_op_prec(*term), 6 | 7)
                    && children.len() == 3
                {
                    Some((
                        self.flat_of(children[0], |p| p.emit_expr_flat(children[0])),
                        children[1].text().to_string(),
                        self.flat_of(children[2], |p| p.emit_expr_flat(children[2])),
                    ))
                } else {
                    None
                }
            })
            .collect();
        let mut columns = vec![None; terms.len()];
        let mut start = 0;
        while start < terms.len() {
            if comparisons[start].is_none() {
                start += 1;
                continue;
            }
            let end = (start..terms.len())
                .find(|&i| comparisons[i].is_none())
                .unwrap_or(terms.len());
            let left = comparisons[start..end]
                .iter()
                .flatten()
                .map(|c| self.visual_width(&c.0))
                .max()
                .unwrap();
            let op = comparisons[start..end]
                .iter()
                .flatten()
                .map(|c| c.1.len())
                .max()
                .unwrap();
            let fits = (start..end).all(|i| {
                let c = comparisons[i].as_ref().unwrap();
                let tail = if terms[i].1.is_empty() {
                    self.eol_reserve
                } else {
                    1 + terms[i].1.len()
                };
                self.indent.saturating_mul(self.indent_width)
                    + left
                    + 1
                    + op
                    + 1
                    + self.visual_width(&c.2)
                    + tail
                    <= self.width
            });
            if fits {
                columns[start..end].fill(Some((left, op)));
            }
            start = end;
        }
        for (i, (term, separator)) in terms.iter().enumerate() {
            if i > 0 {
                self.emit_newline();
            }
            self.emit_indent();
            if let Some(inner) = grouped(*term) {
                self.emit("(");
                self.emit_newline();
                self.indent += 1;
                self.emit_condition_rows(inner);
                self.indent -= 1;
                self.emit_newline();
                self.emit_indent();
                self.emit(")");
            } else if let (Some((left_col, op_col)), Some((left, op, right))) =
                (columns[i], &comparisons[i])
            {
                self.emit(left);
                self.emit(&" ".repeat(left_col - self.visual_width(left) + 1));
                self.emit(op);
                self.emit(&" ".repeat(op_col - op.len() + 1));
                self.emit(right);
            } else {
                // Let the normal expression printer wrap a comparison that
                // cannot fit. Padding must never create a new width violation.
                let saved = self.eol_reserve;
                if !separator.is_empty() {
                    self.eol_reserve = 1 + separator.len();
                }
                self.emit_expr(*term);
                self.eol_reserve = saved;
            }
            if !separator.is_empty() {
                self.emit(" ");
                self.emit(separator);
            }
        }
    }
}
