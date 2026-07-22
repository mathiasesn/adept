//! Errors produced by `adept_fmt`.

/// Errors that can occur while formatting a SKILL.md file.
#[derive(Debug, thiserror::Error)]
pub enum FmtError {
    /// The input did not begin with a YAML frontmatter delimiter (`---`).
    #[error("missing frontmatter (input must start with a line containing only '---')")]
    MissingFrontmatter,

    /// The input's frontmatter was opened with `---` but never closed.
    #[error("unterminated frontmatter (no closing '---' line found)")]
    UnterminatedFrontmatter,

    /// The frontmatter block could not be parsed as YAML.
    #[error("invalid YAML frontmatter: {0}")]
    InvalidYaml(#[from] serde_yaml::Error),

    /// The frontmatter parsed as valid YAML but was not a mapping.
    #[error("frontmatter must be a YAML mapping (key: value pairs)")]
    FrontmatterNotMapping,

    /// A required frontmatter field (`name` or `description`) was missing
    /// or was not a string.
    #[error("missing or invalid required frontmatter field `{0}`")]
    InvalidField(&'static str),

    /// An underlying error occurred that doesn't map to any of the other
    /// `FmtError` variants (e.g. an I/O or JSON error surfaced from
    /// [`adept::AdeptError`] in a context where none of those are expected).
    /// Carries the original error's message so the failure stays
    /// diagnosable instead of being reported as a fabricated field name.
    #[error("internal error: {0}")]
    Internal(String),
}
