pub mod diagnostics;
pub mod printer;
pub mod rules;
pub mod trivia;

use std::borrow::Cow;
use std::path::Path;

pub use diagnostics::{FormatError, FormatWarning};

#[derive(Debug, Clone)]
pub struct FormatOptions {
    /// Maximum consecutive blank lines to keep.
    pub max_blank_lines: usize,
    /// Hard column ceiling used for wrapping.
    pub line_width: usize,
}

impl Default for FormatOptions {
    fn default() -> Self {
        FormatOptions {
            max_blank_lines: 2,
            line_width: 88,
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

pub fn format_file(path: &Path) -> Result<FormatResult, FormatError> {
    format_file_with(path, &FormatOptions::default())
}

pub fn format_file_with(path: &Path, opts: &FormatOptions) -> Result<FormatResult, FormatError> {
    let src = std::fs::read_to_string(path).map_err(FormatError::IoError)?;
    format_str_with(&src, opts)
}
