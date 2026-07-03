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

use crate::FormatOptions;
use std::path::Path;

/// Upper bound on the tab/continuation width knobs. A pathologically large value
/// (from a committed config or a stray flag) would otherwise overflow the
/// wrapping column math; the resolver clamps to this.
pub const MAX_INDENT_WIDTH: usize = 64;

/// Resolve the effective [`FormatOptions`] for a file in `dir` from config files
/// alone (no CLI overrides): the unified `m1-tools.toml [format]` section
/// (lowest layer), then the tool-specific `.m1fmt.toml`. This is the shared
/// resolver the CLI wraps with its flag layer, and that library consumers (the
/// LSP, the MCP server) call so they format a project with the *same* settings
/// as the CLI/CI — e.g. honouring a `brace_style = "kr"` — instead of the
/// hard-coded defaults.
pub fn resolve_options(dir: &Path) -> FormatOptions {
    let mut o = FormatOptions::default();

    // Layer 1: the unified m1-tools.toml [format] section (lowest config layer).
    if let Some(tc) = m1_workspace::config::M1ToolsConfig::discover(dir) {
        let f = tc.format;
        if let Some(n) = f.line_width {
            o.line_width = n;
        }
        if let Some(n) = f.max_blank_lines {
            o.max_blank_lines = n;
        }
        if let Some(n) = f.indent_width {
            o.indent_width = n;
        }
        if let Some(s) = f.indent_style.as_deref().and_then(parse_indent_style) {
            o.indent_style = s;
        }
        if let Some(s) = f.brace_style.as_deref().and_then(parse_brace_style) {
            o.brace_style = s;
        }
        if let Some(n) = f.continuation_indent {
            o.continuation_indent = n;
        }
        if let Some(b) = f.align_assignments {
            o.align_assignments = b;
        }
        if let Some(b) = f.reflow_comments {
            o.reflow_comments = b;
        }
        if let Some(b) = f.final_blank_line {
            o.final_blank_line = b;
        }
    }

    // Layer 2: the tool-specific .m1fmt.toml overrides the unified file.
    if let Some(cfg) = discover(dir) {
        if let Some(n) = cfg.max_line_length {
            o.line_width = n;
        }
        if let Some(n) = cfg.max_blank_lines {
            o.max_blank_lines = n;
        }
        if let Some(n) = cfg.indent_width {
            o.indent_width = n;
        }
        if let Some(s) = cfg.indent_style {
            o.indent_style = s;
        }
        if let Some(s) = cfg.brace_style {
            o.brace_style = s;
        }
        if let Some(n) = cfg.continuation_indent {
            o.continuation_indent = n;
        }
        if let Some(b) = cfg.align_assignments {
            o.align_assignments = b;
        }
        if let Some(b) = cfg.reflow_comments {
            o.reflow_comments = b;
        }
        if let Some(b) = cfg.final_blank_line {
            o.final_blank_line = b;
        }
    }

    o.indent_width = o.indent_width.min(MAX_INDENT_WIDTH);
    o.continuation_indent = o.continuation_indent.min(MAX_INDENT_WIDTH);
    o
}

/// Map a `brace_style` string to the enum. Accepts the documented spellings.
/// Shared by [`parse`] and the unified `m1-tools.toml` mapping in the CLI.
/// Delegates to the canonical parser in m1-workspace.
pub fn parse_brace_style(s: &str) -> Option<crate::BraceStyle> {
    crate::BraceStyle::parse(s)
}

/// Map an `indent_style` string to the enum. Accepts the documented spellings.
/// Shared by [`parse`] and the unified `m1-tools.toml` mapping in the CLI.
/// Delegates to the canonical parser in m1-workspace.
pub fn parse_indent_style(s: &str) -> Option<crate::IndentStyle> {
    crate::IndentStyle::parse(s)
}

/// Parsed config; absent keys are `None` so callers can layer precedence.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct FileConfig {
    /// Maps to `FormatOptions::line_width`.
    pub max_line_length: Option<usize>,
    /// Maps to `FormatOptions::max_blank_lines`.
    pub max_blank_lines: Option<usize>,
    /// Maps to `FormatOptions::brace_style` (`"allman"` | `"kr"`).
    pub brace_style: Option<crate::BraceStyle>,
    /// Maps to `FormatOptions::indent_style` (`"tab"` | `"spaces"`).
    pub indent_style: Option<crate::IndentStyle>,
    /// Maps to `FormatOptions::indent_width`.
    pub indent_width: Option<usize>,
    /// Maps to `FormatOptions::continuation_indent`.
    pub continuation_indent: Option<usize>,
    /// Maps to `FormatOptions::align_assignments` (opt-in, #96).
    pub align_assignments: Option<bool>,
    /// Maps to `FormatOptions::reflow_comments` (opt-in, #95).
    pub reflow_comments: Option<bool>,
    /// Maps to `FormatOptions::final_blank_line` (opt-in, #116).
    pub final_blank_line: Option<bool>,
}

/// Parse a `.m1fmt.toml` body. Unknown keys are ignored; missing keys stay
/// `None`. Returns an error string only on malformed TOML or an invalid value.
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
    let brace_style = match value.get("brace_style").and_then(|v| v.as_str()) {
        None => None,
        Some(s) => Some(parse_brace_style(s).ok_or_else(|| format!("invalid brace_style: {s}"))?),
    };
    let indent_style = match value.get("indent_style").and_then(|v| v.as_str()) {
        None => None,
        Some(s) => Some(parse_indent_style(s).ok_or_else(|| format!("invalid indent_style: {s}"))?),
    };
    Ok(FileConfig {
        max_line_length: uint("max_line_length"),
        max_blank_lines: uint("max_blank_lines"),
        brace_style,
        indent_style,
        indent_width: uint("indent_width"),
        continuation_indent: uint("continuation_indent"),
        align_assignments: value.get("align_assignments").and_then(|v| v.as_bool()),
        reflow_comments: value.get("reflow_comments").and_then(|v| v.as_bool()),
        final_blank_line: value.get("final_blank_line").and_then(|v| v.as_bool()),
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
    fn resolve_options_reads_unified_and_tool_config() {
        let tmp = tempfile::tempdir().unwrap();
        // Unified config sets K&R + spaces; the tool file overrides line width.
        std::fs::write(
            tmp.path().join("m1-tools.toml"),
            "[format]\nbrace_style = \"kr\"\nindent_style = \"spaces\"\nindent_width = 2\n",
        )
        .unwrap();
        std::fs::write(tmp.path().join(".m1fmt.toml"), "max_line_length = 100\n").unwrap();
        let o = resolve_options(tmp.path());
        assert_eq!(o.brace_style, crate::BraceStyle::Kr);
        assert_eq!(o.indent_style, crate::IndentStyle::Spaces);
        assert_eq!(o.indent_width, 2);
        assert_eq!(o.line_width, 100);
    }

    #[test]
    fn resolve_options_defaults_when_no_config() {
        let tmp = tempfile::tempdir().unwrap();
        let o = resolve_options(tmp.path());
        assert_eq!(o.brace_style, FormatOptions::default().brace_style);
    }

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
    fn parses_final_blank_line() {
        let c = parse("final_blank_line = true\n").unwrap();
        assert_eq!(c.final_blank_line, Some(true));
        let c = parse("max_line_length = 90\n").unwrap();
        assert_eq!(c.final_blank_line, None);
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
