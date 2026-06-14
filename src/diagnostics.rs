/// The formatter's only error type. Syntax errors are deliberately NOT errors:
/// an unparseable buffer is returned as a data-preserving `Ok(FormatResult {
/// changed: false, .. })` and surfaced separately via [`crate::syntax_error_count`].
/// The only failure the library ever produces is a file-read I/O error.
#[derive(Debug)]
pub enum FormatError {
    IoError(std::io::Error),
}

impl std::fmt::Display for FormatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FormatError::IoError(e) => write!(f, "IO error: {}", e),
        }
    }
}

impl std::error::Error for FormatError {}

#[derive(Debug, Clone)]
pub struct FormatWarning {
    pub kind: WarningKind,
    pub line: usize,
    pub col: usize,
    pub message: String,
}

#[derive(Debug, Clone)]
pub enum WarningKind {
    LineTooLong,
}
