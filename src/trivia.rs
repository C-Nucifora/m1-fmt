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
