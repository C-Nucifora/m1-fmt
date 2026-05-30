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

pub fn format_str(_src: &str) -> Result<FormatResult, FormatError> {
    todo!("implement in Task 2")
}

pub fn format_file(path: &Path) -> Result<FormatResult, FormatError> {
    let src = std::fs::read_to_string(path).map_err(FormatError::IoError)?;
    format_str(&src)
}
