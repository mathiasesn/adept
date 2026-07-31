//! Core data model, parser, and diagnostics for `adept`, a linter,
//! formatter, and evaluator for Agent Skills.
//!
//! This crate provides the shared foundation that `adept_fmt` (formatting),
//! `adept_agent` (LLM-assisted evaluation, fixing, and creation), and
//! `adept_cli` (the `adept` binary) build on:
//!
//! - [`Skill`] / [`Frontmatter`]: the parsed data model for a SKILL.md file.
//! - [`SkillParser`] / [`AnthropicSkillParser`]: pluggable parsing, so other
//!   Agent Skill ecosystems can be supported later without changing the
//!   rest of the pipeline.
//! - [`SkillSet`]: discovering all skills under a path.
//! - [`Diagnostic`] / [`Severity`] / [`reporting`]: the shared lint finding
//!   type and its human/JSON renderers (rule implementations live in a
//!   sibling crate, but this type is what they produce).
//! - [`markdown`]: the shared `pulldown-cmark`-backed Markdown lexer, used
//!   both by the `SL1xx` lint rules and by `adept_fmt`'s printer.
//! - [`TokenCounter`]: token counting via `tiktoken-rs`.
//! - [`AdeptError`]: the shared error type for hard failures (I/O, malformed
//!   input) as opposed to lint findings.

mod companion;
mod diagnostic;
mod error;
pub mod evals;
mod frontmatter;
pub mod markdown;
mod parser;
pub mod reporting;
mod rules;
mod skill;
mod skillset;
pub mod text;
mod token;

pub use companion::{discover_companion_files, is_eval_dataset};
pub use diagnostic::{Diagnostic, Severity};
pub use error::AdeptError;
pub use frontmatter::{ExtraField, Frontmatter};
pub use parser::{AnthropicSkillParser, SkillParser};
pub use rules::{
    sort_diagnostics, FixKind, FixRegion, LintConfig, Linter, Registry, Rule, RuleMeta, SetRule,
    SkillRule,
};
pub use skill::Skill;
pub use skillset::{sibling_root, skill_directory, SkillSet};
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
