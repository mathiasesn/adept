//! The [`Skill`] data model: a single parsed SKILL.md.

use std::path::PathBuf;

use crate::frontmatter::Frontmatter;

/// A single parsed SKILL.md file.
///
/// `body_line_offset` lets consumers translate a line number within `body`
/// (as returned by e.g. a markdown parser operating only on `body`) back
/// into a line number within the original file: `file_line = body_line_offset
/// + body_relative_line - 1`.
#[derive(Debug, Clone, PartialEq)]
pub struct Skill {
    /// The path to the SKILL.md file this was parsed from.
    pub path: PathBuf,
    /// The parsed and line-annotated frontmatter.
    pub frontmatter: Frontmatter,
    /// The markdown body, i.e. everything after the closing frontmatter
    /// delimiter.
    pub body: String,
    /// The 1-based line number, within the original file, that `body`
    /// starts at.
    pub body_line_offset: usize,
    /// The complete, unmodified source text of the file.
    pub source: String,
}
