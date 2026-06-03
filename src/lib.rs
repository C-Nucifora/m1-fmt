pub mod config;
pub mod diagnostics;
pub mod printer;
pub mod rules;
pub mod trivia;

use std::borrow::Cow;
use std::path::Path;

pub use diagnostics::{FormatError, FormatWarning};

/// Indentation character. The M1 manual mandates tabs, so that is the default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum IndentStyle {
    #[default]
    Tab,
    Spaces,
}

/// Brace placement. The M1 manual mandates Allman ("a separate line for each
/// brace"), so that is the default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BraceStyle {
    #[default]
    Allman,
    /// K&R / "one true brace": opening brace on the keyword line.
    KAndR,
}

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
}

impl Default for FormatOptions {
    fn default() -> Self {
        FormatOptions {
            max_blank_lines: 2,
            line_width: 88,
            indent_style: IndentStyle::default(),
            indent_width: 4,
            brace_style: BraceStyle::default(),
        }
    }
}

pub struct FormatResult {
    pub output: String,
    pub changed: bool,
    pub warnings: Vec<FormatWarning>,
}

pub fn format_str(src: &str) -> Result<FormatResult, FormatError> {
    format_str_with(src, &FormatOptions::default())
}

pub fn format_str_with(src: &str, opts: &FormatOptions) -> Result<FormatResult, FormatError> {
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

    let diags = cst.syntax_diagnostics();
    if !diags.is_empty() {
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

    let lines: Vec<&str> = src.split('\n').collect();
    let slice = lines[start_line..=end_line].join("\n");
    let result = format_str_with(&slice, opts)?;
    // The extracted slice has no trailing newline (it is rejoined from lines) but
    // the formatter always emits one, so compare content ignoring that artifact —
    // the caller splices `output` back over whole lines regardless.
    let changed = result.output.trim_end_matches('\n') != slice.trim_end_matches('\n');
    Ok(Some(RangeResult {
        output: result.output,
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
    let src = std::fs::read_to_string(path).map_err(FormatError::IoError)?;
    format_str_with(&src, opts)
}
