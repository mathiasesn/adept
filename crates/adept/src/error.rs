//! Error types for the `adept` core crate.

use std::path::PathBuf;

/// Errors produced while discovering, reading, or parsing Agent Skills.
///
/// These represent hard failures (I/O errors, malformed input) as opposed to
/// [`crate::diagnostic::Diagnostic`]s, which represent lint findings on
/// otherwise-valid skills.
#[derive(Debug, thiserror::Error)]
pub enum AdeptError {
    /// An I/O error occurred while reading a path.
    #[error("I/O error reading {path}: {source}")]
    Io {
        /// The path that was being read.
        path: PathBuf,
        /// The underlying I/O error.
        #[source]
        source: std::io::Error,
    },

    /// An error occurred while walking a directory tree.
    #[error("failed to walk directory: {0}")]
    WalkDir(#[from] walkdir::Error),

    /// The given path does not exist.
    #[error("path not found: {}", .0.display())]
    NotFound(PathBuf),

    /// The file did not begin with a YAML frontmatter delimiter (`---`).
    #[error("{}: missing frontmatter (file must start with a line containing only '---')", .path.display())]
    MissingFrontmatter {
        /// The file that was being parsed.
        path: PathBuf,
    },

    /// The file's frontmatter was opened with `---` but never closed.
    #[error("{}: unterminated frontmatter (no closing '---' line found)", .path.display())]
    UnterminatedFrontmatter {
        /// The file that was being parsed.
        path: PathBuf,
    },

    /// The frontmatter block could not be parsed as YAML.
    #[error("{}: invalid YAML frontmatter: {source}", .path.display())]
    InvalidYaml {
        /// The file that was being parsed.
        path: PathBuf,
        /// The underlying YAML parse error.
        #[source]
        source: serde_yaml::Error,
    },

    /// The frontmatter parsed as valid YAML but was not a mapping (e.g. a
    /// scalar or a sequence).
    #[error("{}: frontmatter must be a YAML mapping (key: value pairs)", .path.display())]
    FrontmatterNotMapping {
        /// The file that was being parsed.
        path: PathBuf,
    },

    /// A required frontmatter field was not present.
    #[error("{}: missing required frontmatter field `{field}`", .path.display())]
    MissingField {
        /// The file that was being parsed.
        path: PathBuf,
        /// The name of the missing field.
        field: &'static str,
    },

    /// A frontmatter field was present but had the wrong YAML type.
    #[error("{}: frontmatter field `{field}` must be a string", .path.display())]
    InvalidFieldType {
        /// The file that was being parsed.
        path: PathBuf,
        /// The name of the field with the wrong type.
        field: &'static str,
    },

    /// A JSON serialization error, e.g. while rendering diagnostics.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}
