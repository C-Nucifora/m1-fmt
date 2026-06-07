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
