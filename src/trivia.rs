use m1_core::{Cst, Kind, Node};

#[derive(Debug, Clone)]
pub struct TriviaItem {
    pub byte_offset: usize,
    pub end_offset: usize,
    pub text: String,
    pub source_line: usize,
}

pub fn collect_trivia(cst: &Cst) -> Vec<TriviaItem> {
    let mut items = Vec::new();
    collect_node(cst.root(), cst.source(), &mut items);
    items.sort_by_key(|t| t.byte_offset);
    items
}

fn collect_node(node: Node, source: &str, out: &mut Vec<TriviaItem>) {
    if matches!(node.kind(), Kind::LineComment | Kind::BlockComment) {
        let range = node.byte_range();
        let text = node.text().to_string();
        let source_line = source[..range.start].chars().filter(|&c| c == '\n').count();
        out.push(TriviaItem {
            byte_offset: range.start,
            end_offset: range.end,
            text,
            source_line,
        });
        return;
    }
    for child in node.children() {
        collect_node(child, source, out);
    }
}

/// Normalize a raw line comment: strip the `//` prefix, normalize to `// ` + body.
pub fn format_line_comment(raw: &str) -> String {
    let body = raw.strip_prefix("//").unwrap_or(raw);
    // Trim all leading/trailing whitespace (spaces *and* tabs) so e.g.
    // `//\tfoo` normalises to `// foo`, not `// \tfoo`.
    let trimmed = body.trim();
    if trimmed.is_empty() {
        "//".to_string()
    } else {
        format!("// {}", trimmed)
    }
}

/// Determine if this trivia item should be attached as an EOL comment to a
/// statement ending at `stmt_end_line` whose last byte is at `stmt_end_byte`,
/// where the next statement (if any) begins at `next_stmt_start`.
///
/// A comment attaches only if it is on the statement's end line *and* its byte
/// offset falls in this statement's slot on that line — at or after this
/// statement's end byte, and strictly before the next statement begins. The byte
/// window is what stops the first of several statements sharing one source line
/// from greedily claiming a comment that actually trails a later statement:
/// `a = 1; b = 2; // tail` must attach the comment to `b = 2;`, not `a = 1;`
/// (the comment's offset is past where `b = 2;` begins, so it falls outside
/// `a = 1;`'s window). For the final statement, pass `usize::MAX` as the ceiling.
pub fn is_eol_comment(
    item: &TriviaItem,
    stmt_end_line: usize,
    stmt_end_byte: usize,
    next_stmt_start: usize,
) -> bool {
    item.source_line == stmt_end_line
        && item.byte_offset >= stmt_end_byte
        && item.byte_offset < next_stmt_start
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_line_comment_normalizes() {
        assert_eq!(format_line_comment("//foo"), "// foo");
        assert_eq!(format_line_comment("// foo"), "// foo");
        assert_eq!(format_line_comment("//  foo"), "// foo");
        assert_eq!(format_line_comment("//   foo"), "// foo");
        assert_eq!(format_line_comment("//\tfoo"), "// foo");
        assert_eq!(format_line_comment("//\t  foo \t"), "// foo");
        assert_eq!(format_line_comment("//"), "//");
        assert_eq!(format_line_comment("//\t"), "//");
    }

    #[test]
    fn collect_trivia_finds_comments() {
        let src = "// header\nx = 1;\n";
        let cst = m1_core::parse(src);
        let trivia = collect_trivia(&cst);
        assert_eq!(trivia.len(), 1);
        assert_eq!(trivia[0].text, "// header");
        assert_eq!(trivia[0].source_line, 0);
    }

    #[test]
    fn collect_trivia_eol_comment() {
        let src = "x = 1; // note\n";
        let cst = m1_core::parse(src);
        let trivia = collect_trivia(&cst);
        assert_eq!(trivia.len(), 1);
        assert_eq!(trivia[0].source_line, 0); // same line as statement
    }

    #[test]
    fn is_eol_comment_uses_statement_byte_window() {
        // `a = 1; b = 2; // tail` — the comment sits on line 0 at byte 14.
        let comment = TriviaItem {
            byte_offset: 14, // position of `//`
            end_offset: 21,
            text: "// tail".to_string(),
            source_line: 0,
        };
        // First statement `a = 1;` ends at byte 6; the second statement `b = 2;`
        // begins at byte 7. The comment falls outside `a = 1;`'s window (it is at
        // or after where `b = 2;` begins), so it must NOT attach to the first.
        assert!(!is_eol_comment(&comment, 0, 6, 7));
        // It DOES attach to the last statement `b = 2;` (end byte 13, no next).
        assert!(is_eol_comment(&comment, 0, 13, usize::MAX));
        // A single statement claims a same-line comment after its end byte.
        assert!(is_eol_comment(&comment, 0, 6, usize::MAX));
        // A comment before the statement's end byte never attaches.
        assert!(!is_eol_comment(&comment, 0, 15, usize::MAX));
        // Wrong line never attaches.
        assert!(!is_eol_comment(&comment, 1, 6, usize::MAX));
    }
}
