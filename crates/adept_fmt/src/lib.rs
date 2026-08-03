//! `adept_fmt`: a prettier-style formatter for SKILL.md (Agent Skills)
//! files.
//!
//! This crate implements `adept fmt`: canonical frontmatter (key order,
//! minimal-but-correct YAML quoting) plus a full Markdown body reflow built
//! on the shared [`adept::markdown`] AST and a custom deterministic printer.
//!
//! # Supported constructs
//!
//! Fully normalized: ATX headings, paragraphs (with optional prose reflow
//! to a configurable line width), bullet and ordered lists (including
//! nesting and GFM task-list markers), fenced code blocks (content
//! preserved verbatim), tables (column alignment), emphasis/strong/
//! strikethrough markers, links, images, footnote references/definitions.
//!
//! Preserved but not reformatted: raw HTML blocks and inline HTML (passed
//! through verbatim), hard line breaks, block quotes (recursively
//! reformatted, but quoting itself preserved), thematic breaks.
//!
//! Known limitations: Setext headings round-trip only when configured via
//! `HeadingStyle::Setext` (h1/h2 only; h3+ always print as ATX) — the
//! default `HeadingStyle::Atx` normalizes every heading to ATX regardless of
//! source form, same as before; list "tightness" is not
//! preserved distinctly from looseness (blank lines are always inserted
//! between an item's blocks when it contains more than one); reference-style
//! link *definitions* are not currently preserved as such by the printer —
//! they are inlined at each use site (semantically equivalent, textually
//! different).
//!
//! # Escaping scope
//!
//! Prose reflow keeps line-start hazards off of *wrapped* line starts (see
//! `print::marker_like`) for: blockquote `>`, ATX `#` (1-6 of them),
//! ordered-list markers, thematic-break/setext/bullet runs of
//! `- = + * _ ~`, GFM table rows (`|`), HTML block starts (`<div>`,
//! `</p>`, `<!--`, `<?...`), link-reference definitions (`[label]:`), and
//! 4-space-indented code. `escape_text` separately backslash-escapes
//! `\ ` `` ` `` `*` `_` `[` `]` in plain text unconditionally (not just at a
//! line start).
//!
//! Deliberately **out of scope**, left unescaped even at a line start:
//! `<` `>` `&` `!` `(` `)` `#` `:`, a digit followed by `)`, and `#` runs
//! longer than six. These are only ambiguous in positions the formatter
//! already controls — e.g. a lone `>` mid-line, or `#` runs beyond the ATX
//! max — and each additional escape is itself a visible diff, which is
//! exactly the defect this scope closes rather than reproduces. Widening
//! `crates/adept_fmt/tests/proptest_idempotency.rs`'s grammar to generate
//! this punctuation is tracked separately as a coverage gap, not attempted
//! here.
//!
//! # Example
//!
//! ```
//! use adept_fmt::{format_str, FmtConfig};
//!
//! let source = "---\ndescription: does a thing\nname: my-skill\n---\nHello   world.\n";
//! let formatted = format_str(source, &FmtConfig::default()).unwrap();
//! assert!(formatted.starts_with("---\nname: my-skill\n"));
//! ```

mod config;
mod diff;
mod error;
mod frontmatter;
mod print;

pub use config::{BulletMarker, EmphasisMarker, FenceChar, FmtConfig, HeadingStyle, StrongMarker};
pub use diff::CheckResult;
pub use error::FmtError;

use adept::{AdeptError, AnthropicSkillParser, Skill, SkillParser};

/// Format raw SKILL.md source text.
///
/// This parses `source` as a SKILL.md file (YAML frontmatter + Markdown
/// body, per [`adept::AnthropicSkillParser`]) and returns the canonically
/// formatted text: reordered/re-quoted frontmatter, exactly one blank line,
/// then the reflowed Markdown body.
///
/// # Errors
/// Returns a [`FmtError`] if `source` is not a well-formed SKILL.md file
/// (missing/unterminated frontmatter, invalid YAML, or a missing/invalid
/// `name`/`description` field).
pub fn format_str(source: &str, config: &FmtConfig) -> Result<String, FmtError> {
    let path = std::path::Path::new("SKILL.md");
    let skill = AnthropicSkillParser
        .parse_str(path, source)
        .map_err(map_adept_error)?;
    format_skill(&skill, config)
}

/// Format an already-parsed [`Skill`].
///
/// Prefer this over [`format_str`] when a [`Skill`] has already been parsed
/// (e.g. by `adept_cli` walking a directory with [`adept::SkillSet`]), to
/// avoid re-parsing.
///
/// # Errors
/// This currently never fails (a valid [`Skill`] always has a
/// canonicalizable frontmatter and a body that parses as Markdown), but
/// returns a `Result` to leave room for future validation without breaking
/// the API.
pub fn format_skill(skill: &Skill, config: &FmtConfig) -> Result<String, FmtError> {
    let mut out = frontmatter::render_frontmatter(&skill.frontmatter);
    // Exactly one blank line after the closing `---`.
    out.push('\n');

    let blocks = adept::markdown::parse_document(&skill.body);
    if !blocks.is_empty() {
        out.push_str(&print::print_document(&blocks, config));
    }
    Ok(out)
}

/// Check whether `source` is already in canonical formatted form, returning
/// a unified diff of what would change if it is not.
///
/// # Errors
/// Returns a [`FmtError`] under the same conditions as [`format_str`].
pub fn check_str(source: &str, config: &FmtConfig) -> Result<CheckResult, FmtError> {
    let formatted = format_str(source, config)?;
    Ok(diff::check(source, &formatted))
}

/// Like [`check_str`], but for an already-parsed [`Skill`] (compares
/// against [`Skill::source`]).
///
/// # Errors
/// Returns a [`FmtError`] under the same conditions as [`format_skill`].
pub fn check_skill(skill: &Skill, config: &FmtConfig) -> Result<CheckResult, FmtError> {
    let formatted = format_skill(skill, config)?;
    Ok(diff::check(&skill.source, &formatted))
}

fn map_adept_error(err: AdeptError) -> FmtError {
    match err {
        AdeptError::MissingFrontmatter { .. } => FmtError::MissingFrontmatter,
        AdeptError::UnterminatedFrontmatter { .. } => FmtError::UnterminatedFrontmatter,
        AdeptError::InvalidYaml { source, .. } => FmtError::InvalidYaml(source),
        AdeptError::FrontmatterNotMapping { .. } => FmtError::FrontmatterNotMapping,
        AdeptError::MissingField { field, .. } | AdeptError::InvalidFieldType { field, .. } => {
            FmtError::InvalidField(field)
        }
        // `parse_str` (used by `format_str`) never produces `Io`, `WalkDir`,
        // `NotFound`, or `Json` errors today, but keep this arm to stay
        // exhaustive against future `AdeptError` variants without
        // panicking. Preserve the real error message rather than fabricating
        // a field name, so an unexpected case here stays diagnosable.
        other => FmtError::Internal(other.to_string()),
    }
}
