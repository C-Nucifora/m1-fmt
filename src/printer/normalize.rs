//! Whole-output post-processing passes run after the [`Printer`](super::Printer)
//! has emitted the document.
//!
//! Free functions (they operate on the finished string, not on `Printer`):
//! strip brace-adjacent blank lines, collapse blank runs, and normalize the
//! trailing newline. Invoked by [`super::print_with`].

pub(super) fn strip_brace_adjacent_blanks(output: &mut String) {
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
pub(super) fn normalize_trailing(output: &mut String, max_blank: usize) {
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
    for (i, line) in lines.iter().enumerate() {
        result.push_str(line);
        let content = line.strip_suffix('\n').unwrap_or(line);
        if depth == 0 && (content.starts_with("when (") || content.starts_with("when(")) {
            in_top_when = true;
        }
        let opens = content.matches('{').count() as i32;
        let closes = content.matches('}').count() as i32;
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
/// two or more simple single-line assignments at the same indentation. The
/// manual does not mandate column alignment, so this ships off by default; the
/// real corpora use it heavily.
///
/// A "simple assignment" line is `<indent><lhs> = <rhs>;` (optionally with a
/// trailing comment): plain `=` only — a compound operator, a multi-line
/// statement, a comment line or a blank breaks the run. A group is skipped
/// entirely if aligning would push any member past `width`.
pub(super) fn align_assignment_groups(output: &mut String, width: usize) {
    #[derive(Clone)]
    struct Member {
        index: usize,
        lhs_width: usize, // chars in the trimmed LHS
    }

    /// `Some((indent, lhs, rest))` when the line is a simple assignment.
    fn split_simple(line: &str) -> Option<(&str, &str, &str)> {
        let content = line.strip_suffix('\n').unwrap_or(line);
        let indent_len = content.len() - content.trim_start_matches(['\t', ' ']).len();
        let (indent, body) = content.split_at(indent_len);
        let eq = body.find(" = ")?;
        let lhs = &body[..eq];
        let rest = &body[eq + 3..];
        // Plain `=` only: the LHS must not end with an operator character
        // (compound assignment) and must look like an object path.
        let trimmed_lhs = lhs.trim_end();
        if trimmed_lhs.is_empty()
            || trimmed_lhs.ends_with(['+', '-', '*', '/', '%', '&', '|', '^', '<', '>', '!'])
        {
            return None;
        }
        if !trimmed_lhs
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, ' ' | '.' | '_' | '$' | '(' | ')'))
        {
            return None;
        }
        // Single-line statement: the rest must close with `;` (a trailing
        // comment after it is fine).
        let semi = rest.rfind(';')?;
        let after = rest[semi + 1..].trim_start();
        if !(after.is_empty() || after.starts_with("//")) {
            return None;
        }
        Some((indent, lhs, rest))
    }

    let lines: Vec<String> = output.split_inclusive('\n').map(str::to_string).collect();
    let mut rewritten = lines.clone();
    let mut group: Vec<Member> = Vec::new();
    let mut group_indent: Option<String> = None;

    let flush = |group: &mut Vec<Member>, rewritten: &mut Vec<String>, width: usize| {
        if group.len() >= 2 {
            let target = group.iter().map(|m| m.lhs_width).max().unwrap_or(0);
            // Skip the whole group if padding would overflow any line.
            let fits = group.iter().all(|m| {
                let line = &rewritten[m.index];
                let content = line.strip_suffix('\n').unwrap_or(line);
                content.chars().count() + (target - m.lhs_width) <= width
            });
            if fits {
                for m in group.iter() {
                    let line = rewritten[m.index].clone();
                    let (head, tail) = (
                        line.strip_suffix('\n').map(|_| "\n").unwrap_or(""),
                        line.strip_suffix('\n').unwrap_or(&line).to_string(),
                    );
                    if let Some((indent, lhs, rest)) = split_simple(&tail) {
                        let pad = " ".repeat(target - lhs.trim_end().chars().count());
                        rewritten[m.index] =
                            format!("{indent}{}{pad} = {rest}{head}", lhs.trim_end());
                    }
                }
            }
        }
        group.clear();
    };

    for (i, line) in lines.iter().enumerate() {
        match split_simple(line) {
            Some((indent, lhs, _)) if group_indent.as_deref().is_none_or(|gi| gi == indent) => {
                group_indent = Some(indent.to_string());
                group.push(Member {
                    index: i,
                    lhs_width: lhs.trim_end().chars().count(),
                });
            }
            Some((indent, lhs, _)) => {
                // Different indentation: close the old run, start a new one.
                flush(&mut group, &mut rewritten, width);
                group_indent = Some(indent.to_string());
                group.push(Member {
                    index: i,
                    lhs_width: lhs.trim_end().chars().count(),
                });
            }
            None => {
                flush(&mut group, &mut rewritten, width);
                group_indent = None;
            }
        }
    }
    flush(&mut group, &mut rewritten, width);
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
