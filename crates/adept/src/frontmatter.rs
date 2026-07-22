//! The parsed and line-annotated frontmatter of a [`crate::Skill`].

use std::collections::BTreeMap;

/// A frontmatter value that adept did not recognize as one of the well-known
/// fields (`name`, `description`, `license`), preserved along with its
/// source line so rules can still inspect or flag it.
#[derive(Debug, Clone, PartialEq)]
pub struct ExtraField {
    /// The raw YAML value of this field.
    pub value: serde_yaml::Value,
    /// The 1-based line number (within the whole SKILL.md file) the key
    /// appears on.
    pub line: usize,
}

/// The parsed YAML frontmatter of a SKILL.md file.
///
/// `name` and `description` are required by the Anthropic Skill format;
/// `license` is optional and commonly used. Any other keys are preserved in
/// `extra` so that rules and other tooling can still see them (and so that
/// `adept fmt` can round-trip them) without this crate needing to know about
/// every possible ecosystem-specific key.
///
/// Every recognized field also records the 1-based source line it was found
/// on, so diagnostics can point at the exact line in the original file.
#[derive(Debug, Clone, PartialEq)]
pub struct Frontmatter {
    /// The skill's name.
    pub name: String,
    /// The 1-based line number the `name` key appears on.
    pub name_line: usize,

    /// The skill's description, used by agents to decide when to trigger
    /// the skill.
    pub description: String,
    /// The 1-based line number the `description` key appears on.
    pub description_line: usize,

    /// An optional license identifier (e.g. an SPDX expression).
    pub license: Option<String>,
    /// The 1-based line number the `license` key appears on, if present.
    pub license_line: Option<usize>,

    /// Any other frontmatter keys, keyed by field name and sorted for
    /// deterministic iteration.
    pub extra: BTreeMap<String, ExtraField>,
}
