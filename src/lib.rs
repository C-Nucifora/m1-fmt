pub mod diagnostics;
pub mod printer;
pub mod rules;
pub mod trivia;

use std::path::Path;

pub use diagnostics::{FormatError, FormatWarning};

pub struct FormatResult {
    pub output: String,
    pub changed: bool,
    pub warnings: Vec<FormatWarning>,
}

pub fn format_str(src: &str) -> Result<FormatResult, FormatError> {
    let cst = m1_core::parse(src);

    let diags = cst.syntax_diagnostics();
    if !diags.is_empty() {
        // Safety: pass through unchanged, do not error.
        return Ok(FormatResult {
            output: src.to_string(),
            changed: false,
            warnings: vec![],
        });
    }

    let output = printer::print(&cst);
    let changed = output != src;

    // Emit line-too-long warnings.
    let mut warnings = Vec::new();
    for (line_idx, line) in output.lines().enumerate() {
        if line.len() > 88 {
            warnings.push(FormatWarning {
                kind: diagnostics::WarningKind::LineTooLong,
                line: line_idx + 1,
                col: 89,
                message: format!("line is {} characters (max 88)", line.len()),
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
    let src = std::fs::read_to_string(path).map_err(FormatError::IoError)?;
    format_str(&src)
}
