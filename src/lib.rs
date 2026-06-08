pub mod config;
pub mod diagnostics;
pub mod printer;
pub mod trivia;

use std::borrow::Cow;
use std::path::Path;

pub use diagnostics::{FormatError, FormatWarning};

// Code-style enums are shared with m1-lint and now live in m1-workspace (the
// manual mandates tabs + Allman; both are the defaults). Re-exported here so the
// existing `m1_fmt::IndentStyle` / `m1_fmt::BraceStyle` paths keep working.
pub use m1_workspace::{BraceStyle, IndentStyle};

#[derive(Debug, Clone)]
pub struct FormatOptions {
    /// Maximum consecutive blank lines to keep.
    pub max_blank_lines: usize,
    /// Hard column ceiling used for wrapping.
    pub line_width: usize,
    /// Indentation character (default tabs, per the manual).
    pub indent_style: IndentStyle,
    /// Columns one indent level occupies — the number of spaces when
    /// `indent_style` is `Spaces`, and the assumed display width of a tab when it
    /// is `Tab` (used only for the wrapping column math).
    pub indent_width: usize,
    /// Opening-brace placement (default Allman, per the manual).
    pub brace_style: BraceStyle,
    /// Extra indent levels added to wrapped/continuation lines (binary chains,
    /// argument lists, ternaries), on top of the current block indent. The M1
    /// Development Manual specifies a single extra level, so the default is `1`;
    /// teams preferring the older `+2` can override it.
    pub continuation_indent: usize,
}

impl Default for FormatOptions {
    fn default() -> Self {
        FormatOptions {
            max_blank_lines: 2,
            line_width: 88,
            indent_style: IndentStyle::default(),
            indent_width: 4,
            brace_style: BraceStyle::default(),
            continuation_indent: 1,
        }
    }
}

pub struct FormatResult {
    pub output: String,
    pub changed: bool,
    pub warnings: Vec<FormatWarning>,
}

/// The number of syntax errors in `src`. m1-fmt deliberately leaves a buffer
/// with syntax errors byte-for-byte unchanged (data-preserving — see
/// [`format_str_with`]), so callers can't tell "already formatted" apart from
/// "couldn't be parsed" from the [`FormatResult`] alone. The CLI uses this to
/// flag unparseable input under `--check` instead of reporting it clean (#77).
pub fn syntax_error_count(src: &str) -> usize {
    // A leading BOM is presentation trivia, not a parse error; strip it before
    // counting so a BOM-prefixed but otherwise valid file isn't reported broken.
    let src = src.strip_prefix(BOM).unwrap_or(src);
    let lf: Cow<str> = if src.contains("\r\n") {
        Cow::Owned(src.replace("\r\n", "\n"))
    } else {
        Cow::Borrowed(src)
    };
    m1_core::parse(&lf).syntax_diagnostics().len()
}

pub fn format_str(src: &str) -> Result<FormatResult, FormatError> {
    format_str_with(src, &FormatOptions::default())
}

/// The Unicode byte-order mark (U+FEFF). When present at the very start of a
/// document it must survive formatting untouched — including on the
/// syntax-error / too-deep passthrough that is documented to return the buffer
/// byte-for-byte unchanged. The parser/decoder would otherwise drop it, shifting
/// or silently deleting it (#9-style data loss). We strip it before formatting
/// and re-prepend it to the output.
const BOM: char = '\u{FEFF}';

pub fn format_str_with(src: &str, opts: &FormatOptions) -> Result<FormatResult, FormatError> {
    // Preserve a leading BOM end-to-end: strip it from the working buffer, format
    // the body, then re-prepend it so a BOM-prefixed document round-trips with its
    // BOM intact on every path (normal format *and* the passthrough below).
    let (bom, src) = match src.strip_prefix(BOM) {
        Some(rest) => ("\u{FEFF}", rest),
        None => ("", src),
    };
    if !bom.is_empty() {
        let mut result = format_body(src, opts)?;
        // `changed` must reflect the body only relative to the BOM-stripped input;
        // re-prepending the BOM to both sides keeps that comparison honest.
        result.output.insert_str(0, bom);
        return Ok(result);
    }
    format_body(src, opts)
}

fn format_body(src: &str, opts: &FormatOptions) -> Result<FormatResult, FormatError> {
    // The whole pipeline assumes LF. Normalize CRLF -> LF on input and restore
    // CRLF on output if the input used it, so brace-adjacent blank stripping and
    // trailing-newline normalization behave correctly on CRLF files (#18).
    let uses_crlf = src.contains("\r\n");
    let lf_src: Cow<str> = if uses_crlf {
        Cow::Owned(src.replace("\r\n", "\n"))
    } else {
        Cow::Borrowed(src)
    };

    let cst = m1_core::parse(&lf_src);

    // A document nested deeper than the printer can safely recurse would
    // stack-overflow `printer::print_with` (the printer recurses over expression
    // nesting) — an uncatchable abort that crashes the CLI, CI, and the language
    // server's format request. Decline to format it: pass the original source
    // through unchanged, exactly as for a syntax-error buffer. Real M1 code nests
    // only a handful of levels, so this triggers only on adversarial input (#72).
    // `max_depth` is iterative, so the guard itself never overflows.
    let too_deep = cst.root().max_depth() > m1_core::MAX_RECURSION_DEPTH;

    let diags = cst.syntax_diagnostics();
    if too_deep || !diags.is_empty() {
        // Safety: pass through the ORIGINAL source unchanged, do not error.
        return Ok(FormatResult {
            output: src.to_string(),
            changed: false,
            warnings: vec![],
        });
    }

    let lf_output = printer::print_with(&cst, opts);
    let output = if uses_crlf {
        lf_output.replace('\n', "\r\n")
    } else {
        lf_output
    };
    let changed = output != src;

    // Emit line-too-long warnings for lines that remain over budget after
    // wrapping (e.g. an unbreakable atom). `str::lines()` strips the trailing
    // `\r`, so char counts are correct on CRLF output too.
    let mut warnings = Vec::new();
    for (line_idx, line) in output.lines().enumerate() {
        if line.chars().count() > opts.line_width {
            warnings.push(FormatWarning {
                kind: diagnostics::WarningKind::LineTooLong,
                line: line_idx + 1,
                col: opts.line_width + 1,
                message: format!(
                    "line is {} characters (max {})",
                    line.chars().count(),
                    opts.line_width
                ),
            });
        }
    }

    Ok(FormatResult {
        output,
        changed,
        warnings,
    })
}

/// Result of formatting a line range: the replacement text and the 0-based,
/// inclusive line span in the *input* that it replaces (snapped outward to whole
/// top-level statement boundaries).
pub struct RangeResult {
    pub output: String,
    pub start_line: usize,
    pub end_line: usize,
    pub changed: bool,
    pub warnings: Vec<FormatWarning>,
}

/// Format only the top-level statements overlapping the requested line range
/// (`req_start_line..=req_end_line`, 0-based inclusive).
///
/// M1 expression fragments are not independently parseable, but a contiguous run
/// of *complete* top-level statements is. So the range is snapped outward to the
/// statement boundaries it touches, those whole lines are formatted as their own
/// document with the existing pipeline, and the covered span is returned for the
/// caller (LSP `rangeFormatting` / the `--range` CLI) to splice back in.
///
/// Returns `Ok(None)` when no statement overlaps the range, or when the buffer
/// has syntax errors (an incomplete buffer can't be safely range-formatted — the
/// caller should fall back to leaving it untouched).
pub fn format_range(
    src: &str,
    req_start_line: usize,
    req_end_line: usize,
    opts: &FormatOptions,
) -> Result<Option<RangeResult>, FormatError> {
    let cst = m1_core::parse(src);
    if !cst.syntax_diagnostics().is_empty() {
        return Ok(None);
    }
    // Decline to range-format a pathologically deep document — the printer would
    // overflow the stack (#72). The caller leaves the buffer untouched.
    if cst.root().max_depth() > m1_core::MAX_RECURSION_DEPTH {
        return Ok(None);
    }

    // Snap the requested range outward to the span of every top-level statement
    // it intersects. Comments are handled as trivia within those lines.
    let mut covered: Option<(usize, usize)> = None;
    for child in cst.root().children() {
        if matches!(
            child.kind(),
            m1_core::Kind::LineComment | m1_core::Kind::BlockComment
        ) {
            continue;
        }
        let s = child.range().start.line as usize;
        let e = child.range().end.line as usize;
        if s <= req_end_line && e >= req_start_line {
            covered = Some(match covered {
                Some((cs, ce)) => (cs.min(s), ce.max(e)),
                None => (s, e),
            });
        }
    }

    let Some((start_line, end_line)) = covered else {
        return Ok(None);
    };

    // Preserve the document's line ending across the range path. Splitting on
    // `'\n'` leaves a trailing `'\r'` on each line of a CRLF file; rejoining with
    // `'\n'` would drop it on the slice's last line, so the slice loses its
    // `\r\n` sequence and `format_str_with` emits LF-only output — corrupting the
    // CRLF buffer when the caller splices it back (#65). Strip the `'\r'` per line
    // to build a clean LF slice, then restore CRLF on the output if the source
    // used it.
    let uses_crlf = src.contains("\r\n");
    let lines: Vec<&str> = src.split('\n').collect();
    let slice = lines[start_line..=end_line]
        .iter()
        .map(|l| l.strip_suffix('\r').unwrap_or(l))
        .collect::<Vec<_>>()
        .join("\n");
    let result = format_str_with(&slice, opts)?;
    // The extracted slice has no trailing newline (it is rejoined from lines) but
    // the formatter always emits one, so compare content ignoring that artifact —
    // the caller splices `output` back over whole lines regardless. Compare on the
    // LF form so the line-ending normalization below is not itself a "change".
    let changed = result.output.trim_end_matches('\n') != slice.trim_end_matches('\n');
    let output = if uses_crlf {
        result.output.replace('\n', "\r\n")
    } else {
        result.output
    };
    Ok(Some(RangeResult {
        output,
        start_line,
        end_line,
        changed,
        warnings: result.warnings,
    }))
}

pub fn format_file(path: &Path) -> Result<FormatResult, FormatError> {
    format_file_with(path, &FormatOptions::default())
}

pub fn format_file_with(path: &Path, opts: &FormatOptions) -> Result<FormatResult, FormatError> {
    // `.m1scr` is MoTeC source: UTF-8 in practice but may carry Windows-1252
    // bytes (e.g. `°` = 0xB0 in a comment). Decode tolerantly via the shared
    // workspace decoder rather than failing the strict UTF-8 read (#58). The
    // decoder strips a leading BOM (m1-workspace#9); read raw bytes so we can
    // re-prepend it and round-trip a BOM-prefixed file.
    let bytes = std::fs::read(path).map_err(FormatError::IoError)?;
    let had_bom = bytes.starts_with(&[0xEF, 0xBB, 0xBF]);
    let decoded = m1_workspace::decode(bytes);
    let src = if had_bom {
        format!("\u{FEFF}{decoded}")
    } else {
        decoded
    };
    format_str_with(&src, opts)
}
