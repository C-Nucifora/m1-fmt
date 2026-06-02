//! `.m1fmt.toml` project configuration discovery and parsing (#13).
//!
//! A project may commit a `.m1fmt.toml` (or `m1fmt.toml`) at its root:
//!
//! ```toml
//! max_line_length = 88
//! max_blank_lines = 1
//! ```
//!
//! Values are merged into [`crate::FormatOptions`] with precedence
//! CLI flag > config file > built-in default.

use std::path::Path;

/// Parsed config; absent keys are `None` so callers can layer precedence.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct FileConfig {
    /// Maps to `FormatOptions::line_width`.
    pub max_line_length: Option<usize>,
    /// Maps to `FormatOptions::max_blank_lines`.
    pub max_blank_lines: Option<usize>,
}

/// Parse a `.m1fmt.toml` body. Unknown keys are ignored; missing keys stay
/// `None`. Returns an error string only on malformed TOML.
pub fn parse(s: &str) -> Result<FileConfig, String> {
    // Parse as a TOML table: toml 1.x changed `str::parse::<Value>` to expect a
    // bare value (not a `key = val` document), so parsing a config into `Value`
    // fails with "unexpected content". A `Table` parses the document directly.
    let value: toml::Table = s.parse().map_err(|e: toml::de::Error| e.to_string())?;
    let uint = |key: &str| {
        value
            .get(key)
            .and_then(|v| v.as_integer())
            .filter(|i| *i >= 0)
            .map(|i| i as usize)
    };
    Ok(FileConfig {
        max_line_length: uint("max_line_length"),
        max_blank_lines: uint("max_blank_lines"),
    })
}

/// Walk upward from `start` looking for `.m1fmt.toml` (then `m1fmt.toml`) and
/// return the first one that parses. Returns `None` if none is found.
pub fn discover(start: &Path) -> Option<FileConfig> {
    let mut dir = Some(start);
    while let Some(d) = dir {
        for name in [".m1fmt.toml", "m1fmt.toml"] {
            let path = d.join(name);
            if path.is_file()
                && let Ok(body) = std::fs::read_to_string(&path)
                && let Ok(cfg) = parse(&body)
            {
                return Some(cfg);
            }
        }
        dir = d.parent();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_both_keys() {
        let c = parse("max_line_length = 100\nmax_blank_lines = 1\n").unwrap();
        assert_eq!(c.max_line_length, Some(100));
        assert_eq!(c.max_blank_lines, Some(1));
    }

    #[test]
    fn missing_keys_are_none() {
        let c = parse("max_line_length = 72\n").unwrap();
        assert_eq!(c.max_line_length, Some(72));
        assert_eq!(c.max_blank_lines, None);
    }

    #[test]
    fn unknown_keys_ignored() {
        let c = parse("max_blank_lines = 0\nfuture_option = \"x\"\n").unwrap();
        assert_eq!(c.max_blank_lines, Some(0));
    }

    #[test]
    fn malformed_is_error() {
        assert!(parse("max_line_length = = 3").is_err());
    }

    #[test]
    fn discover_finds_config_up_the_tree() {
        let tmp = std::env::temp_dir().join(format!("m1fmt_cfg_{}", std::process::id()));
        let nested = tmp.join("a/b");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(tmp.join(".m1fmt.toml"), "max_line_length = 120\n").unwrap();
        let found = discover(&nested).expect("should find config up the tree");
        assert_eq!(found.max_line_length, Some(120));
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
