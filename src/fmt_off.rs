//! `@m1:fmt(off)` / `@m1:fmt(on)` format-off regions (#102).
//!
//! A *standalone* line comment `// @m1:fmt(off)` opens a region and the next
//! standalone `// @m1:fmt(on)` closes it; the lines from the off marker through
//! the on marker (inclusive) pass through the formatter byte-for-byte. An
//! unclosed `off` runs to end of file. Markers sharing a line with code, inside
//! block comments, or an `on` with no open region are inert.
//!
//! Mechanics: the printer preserves every comment in source order, so the same
//! markers reappear in the formatted output. The document is formatted as
//! usual, then each marker-to-marker span in the output is replaced by the
//! original source lines — splice anchors are the markers themselves, found by
//! the same scan on both sides.

use m1_core::{Cst, Kind};

/// A line span that ends at end-of-file.
const EOF: usize = usize::MAX;

/// The format-off regions of `src` as 0-based inclusive line spans (marker
/// lines included). An unclosed region's end is [`EOF`].
pub(crate) fn off_regions(src: &str, cst: &Cst) -> Vec<(usize, usize)> {
    let mut regions = Vec::new();
    let mut open: Option<usize> = None;
    for (line, is_off) in marker_lines(src, cst) {
        match (is_off, open) {
            (true, None) => open = Some(line),
            (false, Some(start)) => {
                regions.push((start, line));
                open = None;
            }
            // A second `off` inside a region is part of it; an `on` with no
            // open region is an ordinary comment.
            _ => {}
        }
    }
    if let Some(start) = open {
        regions.push((start, EOF));
    }
    regions
}

/// Every standalone `// @m1:fmt(off|on)` marker, as `(line, is_off)` in source
/// order. CST-driven so block-comment interiors and string contents can never
/// false-positive; "standalone" means nothing but whitespace precedes the
/// comment on its line.
fn marker_lines(src: &str, cst: &Cst) -> Vec<(usize, bool)> {
    let mut markers = Vec::new();
    for node in cst.root().descendants() {
        if node.kind() != Kind::LineComment {
            continue;
        }
        let byte_range = node.byte_range();
        let Some(is_off) = marker_arg(&src[byte_range.clone()]) else {
            continue;
        };
        let line_start = src[..byte_range.start].rfind('\n').map_or(0, |i| i + 1);
        if !src[line_start..byte_range.start]
            .chars()
            .all(char::is_whitespace)
        {
            continue; // trailing comment — inert as a region marker
        }
        markers.push((node.range().start.line as usize, is_off));
    }
    markers.sort_unstable_by_key(|&(line, _)| line);
    markers
}

/// `Some(true)` for an off marker, `Some(false)` for on, `None` otherwise.
/// Trailing prose after the closing paren is allowed.
fn marker_arg(comment: &str) -> Option<bool> {
    let body = comment.strip_prefix("//")?.trim_start();
    let arg = body.strip_prefix("@m1:fmt(")?;
    match arg.split(')').next()? {
        "off" => Some(true),
        "on" => Some(false),
        _ => None,
    }
}

/// Replace every off region in the formatted output `lf_out` with the verbatim
/// lines of `lf_src`, anchoring on the markers found by the same scan in both.
/// Returns the spliced text plus the regions' final line spans (for warning
/// suppression). `None` when the output's markers don't pair up with the
/// input's — the caller falls back to leaving the document unchanged.
pub(crate) fn splice(
    lf_src: &str,
    lf_out: &str,
    src_regions: &[(usize, usize)],
) -> Option<(String, Vec<(usize, usize)>)> {
    let out_cst = m1_core::parse(lf_out);
    let out_regions = off_regions(lf_out, &out_cst);
    if out_regions.len() != src_regions.len() {
        return None;
    }

    let src_lines: Vec<&str> = lf_src.split('\n').collect();
    let out_lines: Vec<&str> = lf_out.split('\n').collect();
    let clamp = |(s, e): (usize, usize), len: usize| (s, e.min(len - 1));

    let mut final_lines: Vec<&str> = Vec::with_capacity(out_lines.len());
    let mut final_spans = Vec::with_capacity(src_regions.len());
    let mut cursor = 0usize; // next output line to copy
    for (&src_region, &out_region) in src_regions.iter().zip(&out_regions) {
        let (src_start, src_end) = clamp(src_region, src_lines.len());
        let (out_start, out_end) = clamp(out_region, out_lines.len());
        if out_start < cursor {
            return None;
        }
        final_lines.extend_from_slice(&out_lines[cursor..out_start]);
        let span_start = final_lines.len();
        final_lines.extend_from_slice(&src_lines[src_start..=src_end]);
        final_spans.push((span_start, final_lines.len() - 1));
        cursor = out_end + 1;
    }
    if cursor < out_lines.len() {
        final_lines.extend_from_slice(&out_lines[cursor..]);
    }
    Some((final_lines.join("\n"), final_spans))
}
