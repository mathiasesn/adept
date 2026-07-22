//! Rendering [`Diagnostic`]s for human consumption or as JSON.

use std::io::IsTerminal;

use owo_colors::OwoColorize;

use crate::diagnostic::{Diagnostic, Severity};
use crate::error::AdeptError;

/// Render diagnostics ruff-style: one line per diagnostic in the form
/// `path:line:col: CODE message`, followed by an indented fix suggestion
/// line if present.
///
/// Colors are enabled automatically when stdout is a terminal, and disabled
/// otherwise (e.g. when piped to a file or another program). Use
/// [`render_human_colored`] to control this explicitly, e.g. for tests.
pub fn render_human(diagnostics: &[Diagnostic]) -> String {
    render_human_colored(diagnostics, std::io::stdout().is_terminal())
}

/// Like [`render_human`], but with explicit control over whether ANSI color
/// codes are emitted.
pub fn render_human_colored(diagnostics: &[Diagnostic], color: bool) -> String {
    let mut out = String::new();
    for d in diagnostics {
        let loc = format!("{}:{}:{}", d.path.display(), d.line, d.column);
        if color {
            let code = match d.severity {
                Severity::Error => d.code.red().bold().to_string(),
                Severity::Warning => d.code.yellow().bold().to_string(),
                Severity::Info => d.code.cyan().bold().to_string(),
            };
            out.push_str(&format!("{}: {code} {}\n", loc.bold(), d.message));
        } else {
            out.push_str(&format!("{loc}: {} {}\n", d.code, d.message));
        }
        if let Some(fix) = &d.fix_suggestion {
            out.push_str(&format!("  fix: {fix}\n"));
        }
    }
    out
}

/// Render diagnostics as a pretty-printed JSON array.
///
/// # Errors
/// Returns [`AdeptError::Json`] if serialization fails (which should not
/// happen for this type).
pub fn render_json(diagnostics: &[Diagnostic]) -> Result<String, AdeptError> {
    Ok(serde_json::to_string_pretty(diagnostics)?)
}
