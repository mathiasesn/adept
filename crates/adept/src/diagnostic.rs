//! The [`Diagnostic`] type used by lint rules (owned by a sibling crate) to
//! report findings, plus their [`Severity`].

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// The severity of a [`Diagnostic`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// A finding serious enough to fail CI by default.
    Error,
    /// A finding worth surfacing but not failing CI on by default.
    Warning,
    /// An informational note.
    Info,
}

impl std::fmt::Display for Severity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Info => "info",
        };
        f.write_str(s)
    }
}

/// A single lint finding, ruff-style: a stable rule `code`, a human-readable
/// `message`, a `severity`, and a precise source location.
///
/// This type is produced by rule implementations (owned by a sibling crate)
/// but defined here so that the core crate can render it consistently via
/// [`crate::reporting`].
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Diagnostic {
    /// The stable rule code, e.g. `"SL001"`.
    pub code: &'static str,
    /// A human-readable description of the finding.
    pub message: String,
    /// How severe the finding is.
    pub severity: Severity,
    /// The file the finding applies to.
    pub path: PathBuf,
    /// The 1-based line number the finding applies to.
    pub line: usize,
    /// The 1-based column number the finding applies to.
    pub column: usize,
    /// An optional human-readable suggestion for how to fix the finding.
    pub fix_suggestion: Option<String>,
}

impl Diagnostic {
    /// Construct a new diagnostic with no fix suggestion.
    pub fn new(
        code: &'static str,
        message: impl Into<String>,
        severity: Severity,
        path: impl Into<PathBuf>,
        line: usize,
        column: usize,
    ) -> Self {
        Self {
            code,
            message: message.into(),
            severity,
            path: path.into(),
            line,
            column,
            fix_suggestion: None,
        }
    }

    /// Attach a fix suggestion to this diagnostic, builder-style.
    #[must_use]
    pub fn with_fix_suggestion(mut self, suggestion: impl Into<String>) -> Self {
        self.fix_suggestion = Some(suggestion.into());
        self
    }
}
