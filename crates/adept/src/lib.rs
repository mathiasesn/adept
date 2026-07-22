//! Core data model, parser, and diagnostics for `adept`, a linter,
//! formatter, and scorer for Agent Skills.
//!
//! This crate provides the shared foundation that `adept_fmt` (formatting),
//! `adept_score` (LLM-assisted scoring), and `adept_cli` (the `adept`
//! binary) build on:
//!
//! - [`Skill`] / [`Frontmatter`]: the parsed data model for a SKILL.md file.
//! - [`SkillParser`] / [`AnthropicSkillParser`]: pluggable parsing, so other
//!   Agent Skill ecosystems can be supported later without changing the
//!   rest of the pipeline.
//! - [`SkillSet`]: discovering all skills under a path.
//! - [`Diagnostic`] / [`Severity`] / [`reporting`]: the shared lint finding
//!   type and its human/JSON renderers (rule implementations live in a
//!   sibling crate, but this type is what they produce).
//! - [`TokenCounter`]: token counting via `tiktoken-rs`.
//! - [`AdeptError`]: the shared error type for hard failures (I/O, malformed
//!   input) as opposed to lint findings.

mod companion;
mod diagnostic;
mod error;
mod frontmatter;
mod parser;
pub mod reporting;
mod rules;
mod skill;
mod skillset;
pub mod text;
mod token;

pub use companion::discover_companion_files;
pub use diagnostic::{Diagnostic, Severity};
pub use error::AdeptError;
pub use frontmatter::{ExtraField, Frontmatter};
pub use parser::{AnthropicSkillParser, SkillParser};
pub use rules::{LintConfig, Linter, Registry, Rule, RuleMeta, SetRule, SkillRule};
pub use skill::Skill;
pub use skillset::SkillSet;
pub use token::{TokenCounter, Tokenizer};

use std::path::Path;

/// Parse a single SKILL.md file using the default [`AnthropicSkillParser`].
///
/// This is a convenience wrapper around
/// `AnthropicSkillParser.parse(path.as_ref())` for the common case; use
/// [`SkillParser`] directly for other formats or [`SkillSet::discover`] to
/// parse a whole directory tree.
///
/// # Errors
/// Returns an [`AdeptError`] if the file cannot be read or does not parse as
/// a valid SKILL.md.
pub fn parse_skill(path: impl AsRef<Path>) -> Result<Skill, AdeptError> {
    AnthropicSkillParser.parse(path.as_ref())
}
