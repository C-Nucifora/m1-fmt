//! Whole-output post-processing passes run after the [`Printer`](super::Printer)
//! has emitted the document.
//!
//! Free functions (they operate on the finished string, not on `Printer`):
//! strip brace-adjacent blank lines, collapse blank runs, and normalize the
//! trailing newline. Invoked by [`super::print_with`].

/// Return the portion of `line` that is *code* — i.e. with `//` line comments,
/// `/* ... */` block comments and `"..."` string literals removed — so that
/// brace counting and opener/closer detection only ever see real block
/// delimiters. Braces that appear inside a comment or a string literal are not
/// block structure and must not affect depth accounting (corpus: commented-out
/// code with bare `//{` / `//}` lines, `/* TODO: } */` markers and unbalanced
/// braces in strings).
///
/// `in_block_comment` carries `/* ... */` state across lines: it is `true` on
/// entry when a previous line opened a block comment that has not yet closed,
/// and is updated on exit to reflect whether this line left a block comment
/// open. Callers that process a whole document line-by-line must thread this
/// flag through their loop so interior braces of a multi-line block comment are
/// skipped.
fn code_only_carry(line: &str, in_block_comment: &mut bool) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    let mut in_string = false;
    while let Some(c) = chars.next() {
        if *in_block_comment {
            // Consume until the matching `*/`.
            if c == '*' && chars.peek() == Some(&'/') {
                chars.next();
                *in_block_comment = false;
            }
            continue;
        }
        if in_string {
            if c == '\\' {
                // Skip the escaped character verbatim.
                chars.next();
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        match c {
            '"' => in_string = true,
            '/' if chars.peek() == Some(&'/') => break, // `//` line comment
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                *in_block_comment = true;
            }
            _ => out.push(c),
        }
    }
    out
}

/// Count code-only `{` and `}` on a line (comments and strings excluded),
/// threading multi-line block-comment state through `in_block_comment`.
fn code_braces_carry(line: &str, in_block_comment: &mut bool) -> (i32, i32) {
    let code = code_only_carry(line, in_block_comment);
    (
        code.matches('{').count() as i32,
        code.matches('}').count() as i32,
    )
}

pub(super) fn strip_brace_adjacent_blanks(output: &mut String) {
    let lines: Vec<&str> = output.split_inclusive('\n').collect();
    let is_blank = |s: &str| s.strip_suffix('\n').unwrap_or(s).trim().is_empty();
    // Block-comment state on *entry* to each line, so opener/closer detection on
    // an arbitrary line index excludes braces inside a multi-line `/* ... */`.
    let block_comment_entry: Vec<bool> = {
        let mut entry = Vec::with_capacity(lines.len());
        let mut carry = false;
        for line in &lines {
            entry.push(carry);
            let content = line.strip_suffix('\n').unwrap_or(line);
            let _ = code_only_carry(content, &mut carry);
        }
        entry
    };
    // Opener/closer detection runs on the code-only portion of the line so a
    // brace inside a `//` or `/* ... */` comment or a string literal is never
    // mistaken for a block delimiter.
    let code_trimmed_end = |idx: usize| -> String {
        let s = lines[idx];
        let mut carry = block_comment_entry[idx];
        code_only_carry(s.strip_suffix('\n').unwrap_or(s), &mut carry)
            .trim_end()
            .to_string()
    };
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
            Some(p) if code_trimmed_end(p).ends_with('{') => keep[i] = false,
            _ => {}
        }
        if let Some(n) = next_nonblank {
            let code = code_trimmed_end(n);
            if code == "}" || code.starts_with('}') {
                keep[i] = false;
            }
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

/// Collapse blank runs to `max_blank` and normalize the file end: exactly one
/// final newline, or — with `final_blank_line` (#116, the m1-lint L027 pair) —
/// exactly one final *blank line* (`\n\n`). Empty output stays empty either way.
pub(super) fn normalize_trailing(output: &mut String, max_blank: usize, final_blank_line: bool) {
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
    if final_blank_line {
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

/// Manual p.65: "All functions and methods to end with a blank line" — in a
/// script that means top-level `when` blocks (#97). Insert one blank line
/// after each top-level when-block's closing brace when the author omitted it
/// (a trailing blank at EOF is trimmed by [`normalize_trailing`]).
///
/// Works on canonical printer output: depth is tracked by brace counting per
/// line; a top-level statement line starting with `when` (Allman `when (…)` or
/// K&R `when (…) {`) marks the block whose return to depth 0 needs the blank.
pub(super) fn ensure_blank_after_top_level_when(output: &mut String) {
    let lines: Vec<String> = output.split_inclusive('\n').map(str::to_string).collect();
    let mut result = String::with_capacity(output.len() + 8);
    let mut depth: i32 = 0;
    let mut in_top_when = false;
    let mut in_block_comment = false;
    for (i, line) in lines.iter().enumerate() {
        result.push_str(line);
        let content = line.strip_suffix('\n').unwrap_or(line);
        if depth == 0
            && !in_block_comment
            && (content.starts_with("when (") || content.starts_with("when("))
        {
            in_top_when = true;
        }
        // Counts code braces and advances `in_block_comment` across this line so
        // braces inside a multi-line `/* ... */` are never counted as block
        // structure.
        let (opens, closes) = code_braces_carry(content, &mut in_block_comment);
        let before = depth;
        depth += opens - closes;
        if in_top_when && before > 0 && depth == 0 {
            in_top_when = false;
            // Require a blank line before the next non-blank line.
            if let Some(next) = lines.get(i + 1) {
                let next_content = next.strip_suffix('\n').unwrap_or(next);
                if !next_content.trim().is_empty() {
                    result.push('\n');
                }
            }
        }
    }
    *output = result;
}

/// Opt-in `align_assignments` (#96): align the `=` of each contiguous run of
/// two or more simple single-line assignments or local declarations at the
/// same indentation. Declarations and assignments form separate groups. The
/// manual does not mandate column alignment, so this ships off by default; the
/// real corpora use it heavily.
///
/// A "simple assignment" line is `<indent><lhs> = <rhs>;` (optionally with a
/// trailing comment): plain `=` only — a compound operator, a multi-line
/// statement, a comment line or a blank breaks the run. A group is skipped
/// entirely if aligning would push any member past `width`.
pub(super) fn align_assignment_groups(output: &mut String, width: usize, indent_width: usize) {
    use m1_core::Kind;

    #[derive(Clone, Copy)]
    struct Member<'a> {
        kind: Kind,
        indent: &'a str,
        lhs: &'a str,
        rest: &'a str,
        lhs_width: usize,
    }

    // Use the grammar to identify complete statements and their plain `=`.
    // A character whitelist cannot describe M1 identifiers, which can contain
    // spaces and compile-time interpolation. Parsing the whole output also
    // keeps assignment-shaped text inside comments out of these groups.
    let cst = m1_core::parse(output);
    if !cst.syntax_diagnostics().is_empty() {
        return;
    }
    let lines: Vec<&str> = output.split_inclusive('\n').collect();
    let offsets: Vec<usize> = lines
        .iter()
        .scan(0, |offset, line| {
            let start = *offset;
            *offset += line.len();
            Some(start)
        })
        .collect();
    let mut members = vec![None; lines.len()];
    let mut pending = vec![cst.root()];
    while let Some(node) = pending.pop() {
        if !matches!(
            node.kind(),
            Kind::AssignmentStatement | Kind::LocalDeclaration
        ) {
            pending.extend(node.children());
            continue;
        }
        if node.text().contains('\n') {
            continue;
        }
        let Some(assign) = node
            .children()
            .into_iter()
            .find(|child| child.kind() == Kind::Assign)
        else {
            continue;
        };
        let span = node.byte_range();
        let index = offsets.partition_point(|&offset| offset <= span.start) - 1;
        let offset = offsets[index];
        let line = lines[index];
        let indent = &line[..span.start - offset];
        // A comment or another statement sharing the line breaks the group.
        if !indent.chars().all(|c| matches!(c, ' ' | '\t')) {
            continue;
        }
        let after = line[span.end - offset..].trim();
        if !after.is_empty() && !after.starts_with("//") {
            continue;
        }
        let lhs = line[span.start - offset..assign.byte_range().start - offset].trim_end();
        let rest = line[assign.byte_range().end - offset..].trim_start_matches(' ');
        members[index] = Some(Member {
            kind: node.kind(),
            indent,
            lhs,
            rest,
            lhs_width: lhs.chars().count(),
        });
    }

    let visual_width = |text: &str| {
        text.chars()
            .map(|c| if c == '\t' { indent_width } else { 1 })
            .fold(0usize, usize::saturating_add)
    };
    let mut rewritten: Vec<String> = lines.iter().map(|line| line.to_string()).collect();
    let mut start = 0;
    while start < members.len() {
        let Some(first) = members[start] else {
            start += 1;
            continue;
        };
        let end = (start + 1..members.len())
            .find(|&i| {
                members[i]
                    .is_none_or(|member| member.indent != first.indent || member.kind != first.kind)
            })
            .unwrap_or(members.len());
        let group = &members[start..end];
        let target = group
            .iter()
            .flatten()
            .map(|member| member.lhs_width)
            .max()
            .unwrap();
        let fits = group.iter().flatten().all(|member| {
            visual_width(member.indent)
                .saturating_add(target)
                .saturating_add(3) // " = "
                .saturating_add(visual_width(member.rest.trim_end_matches('\n')))
                <= width
        });
        if group.len() >= 2 && fits {
            for (i, member) in group.iter().flatten().enumerate() {
                let pad = " ".repeat(target - member.lhs_width);
                rewritten[start + i] =
                    format!("{}{}{pad} = {}", member.indent, member.lhs, member.rest);
            }
        }
        start = end;
    }
    *output = rewritten.concat();
}

/// Opt-in `reflow_comments` (#95): split over-width `//` comment lines onto
/// continuation comment lines at the same indent. Deliberately split-only —
/// short authored lines (bullets, paragraphs) are never joined — and
/// `///`-doc, `////`-rule and `@m1:` annotation lines are never touched (an
/// annotation must stay on one line to keep its meaning).
pub(super) fn reflow_long_line_comments(output: &mut String, width: usize) {
    let mut result = String::with_capacity(output.len());
    for line in output.split_inclusive('\n') {
        let content = line.strip_suffix('\n').unwrap_or(line);
        let trimmed = content.trim_start();
        let indent = &content[..content.len() - trimmed.len()];
        let eligible = trimmed.starts_with("// ")
            && !trimmed.starts_with("///")
            && !trimmed.contains("@m1:")
            && content.chars().count() > width;
        if !eligible {
            result.push_str(line);
            continue;
        }
        // Greedy word wrap of the comment text at `width`.
        let text = &trimmed[3..];
        let prefix = format!("{indent}// ");
        let budget = width.saturating_sub(prefix.chars().count()).max(8);
        let mut current = String::new();
        for word in text.split_whitespace() {
            if !current.is_empty() && current.chars().count() + 1 + word.chars().count() > budget {
                result.push_str(&prefix);
                result.push_str(&current);
                result.push('\n');
                current.clear();
            }
            if !current.is_empty() {
                current.push(' ');
            }
            current.push_str(word);
        }
        if !current.is_empty() {
            result.push_str(&prefix);
            result.push_str(&current);
            result.push('\n');
        }
    }
    *output = result;
}
