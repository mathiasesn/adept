//! `SL00x` frontmatter/naming rules.
//!
//! `SL001`/`SL002` are dual-registered: they check the ordinary
//! [`SkillRule`] path (field present but empty) and the [`ParseErrorRule`]
//! path (field missing entirely, so parsing failed before a `Skill` could
//! be built). `SL003` (`malformed-frontmatter`) is exclusively a
//! [`ParseErrorRule`]: a skill with malformed frontmatter never produces a
//! `Skill` to run an ordinary rule against.

use crate::diagnostic::{Diagnostic, Severity};
use crate::error::AdeptError;
use crate::skill::Skill;

use super::{impl_rule, FixKind, LintConfig, ParseErrorRule, Rule, SkillRule};

/// `SL001` `missing-description`: the `description` frontmatter field is
/// present but empty (or whitespace-only), or absent entirely (parse-time).
pub struct MissingDescription;

impl_rule!(MissingDescription, "SL001", "missing-description", Error);

impl SkillRule for MissingDescription {
    fn check(
        &self,
        skill: &Skill,
        _config: &LintConfig,
        _tokens: &crate::token::TokenCounter,
    ) -> Vec<Diagnostic> {
        if skill.frontmatter.description.trim().is_empty() {
            vec![Diagnostic::new(
                self.code(),
                "the `description` frontmatter field is empty",
                self.default_severity(),
                &skill.path,
                skill.frontmatter.description_line,
                1,
            )
            .with_fix_suggestion(
                "write a description stating what the skill does and when to use it",
            )]
        } else {
            Vec::new()
        }
    }
}

/// Build the single diagnostic a [`ParseErrorRule`] reports for a parse
/// failure, always anchored at 1:1 — a file that failed to parse has no
/// meaningful line to point at.
fn parse_error_diagnostic(
    rule: &dyn Rule,
    path: &std::path::Path,
    message: impl Into<String>,
    suggestion: Option<&str>,
) -> Vec<Diagnostic> {
    let diagnostic = Diagnostic::new(rule.code(), message, rule.default_severity(), path, 1, 1);
    vec![match suggestion {
        Some(s) => diagnostic.with_fix_suggestion(s),
        None => diagnostic,
    }]
}

/// The parse-time half of `SL001`/`SL002`: the field is absent entirely, so
/// there is no [`Skill`] for the [`SkillRule`] impl to inspect.
fn missing_field_diagnostic(
    rule: &dyn Rule,
    path: &std::path::Path,
    err: &AdeptError,
    expected: &str,
    message: &str,
    suggestion: &str,
) -> Vec<Diagnostic> {
    match err {
        AdeptError::MissingField { field, .. } if *field == expected => {
            parse_error_diagnostic(rule, path, message, Some(suggestion))
        }
        _ => Vec::new(),
    }
}

impl ParseErrorRule for MissingDescription {
    fn check(&self, path: &std::path::Path, err: &AdeptError) -> Vec<Diagnostic> {
        missing_field_diagnostic(
            self,
            path,
            err,
            "description",
            "SKILL.md is missing the required `description` frontmatter field",
            "add a `description` field stating what the skill does and when to use it",
        )
    }
}

/// `SL002` `missing-name`: the `name` frontmatter field is present but empty
/// (or whitespace-only), or absent entirely (parse-time).
pub struct MissingName;

impl_rule!(MissingName, "SL002", "missing-name", Error);

impl SkillRule for MissingName {
    fn check(
        &self,
        skill: &Skill,
        _config: &LintConfig,
        _tokens: &crate::token::TokenCounter,
    ) -> Vec<Diagnostic> {
        if skill.frontmatter.name.trim().is_empty() {
            vec![Diagnostic::new(
                self.code(),
                "the `name` frontmatter field is empty",
                self.default_severity(),
                &skill.path,
                skill.frontmatter.name_line,
                1,
            )
            .with_fix_suggestion("set `name` to match the skill's directory name")]
        } else {
            Vec::new()
        }
    }
}

impl ParseErrorRule for MissingName {
    fn check(&self, path: &std::path::Path, err: &AdeptError) -> Vec<Diagnostic> {
        missing_field_diagnostic(
            self,
            path,
            err,
            "name",
            "SKILL.md is missing the required `name` frontmatter field",
            "add a `name` field matching the skill's directory name",
        )
    }
}

/// `SL003` `malformed-frontmatter`: the frontmatter block itself fails to
/// parse (missing or unterminated `---` fence, invalid YAML, non-mapping
/// frontmatter, or a known field with the wrong type). Exclusively a
/// [`ParseErrorRule`]: a skill in this state has no `Skill` to run an
/// ordinary rule against.
pub struct MalformedFrontmatter;

impl_rule!(
    MalformedFrontmatter,
    "SL003",
    "malformed-frontmatter",
    Error
);

impl ParseErrorRule for MalformedFrontmatter {
    fn check(&self, path: &std::path::Path, err: &AdeptError) -> Vec<Diagnostic> {
        let (message, suggestion): (String, Option<&str>) = match err {
            AdeptError::MissingFrontmatter { .. } => (
                "SKILL.md must start with a line containing only '---' to open the YAML \
                 frontmatter block"
                    .to_string(),
                Some("add an opening `---` line as the first line of the file"),
            ),
            AdeptError::UnterminatedFrontmatter { .. } => (
                "SKILL.md frontmatter is opened with '---' but never closed".to_string(),
                Some("add a closing `---` line after the frontmatter fields"),
            ),
            AdeptError::InvalidYaml { source, .. } => (
                format!("SKILL.md frontmatter is not valid YAML: {source}"),
                None,
            ),
            AdeptError::FrontmatterNotMapping { .. } => (
                "SKILL.md frontmatter must be a YAML mapping (key: value pairs)".to_string(),
                None,
            ),
            AdeptError::InvalidFieldType { field, .. } => (
                format!("SKILL.md frontmatter field `{field}` must be a string"),
                None,
            ),
            _ => return Vec::new(),
        };
        parse_error_diagnostic(self, path, message, suggestion)
    }
}

/// `SL004` `name-mismatch`: the frontmatter `name` does not match the name
/// of the directory containing SKILL.md.
pub struct NameMismatch;

impl_rule!(NameMismatch, "SL004", "name-mismatch", Warning);

impl SkillRule for NameMismatch {
    fn check(
        &self,
        skill: &Skill,
        _config: &LintConfig,
        _tokens: &crate::token::TokenCounter,
    ) -> Vec<Diagnostic> {
        let Some(dir_name) = skill
            .path
            .parent()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_string())
        else {
            return Vec::new();
        };

        if skill.frontmatter.name.trim().is_empty() || skill.frontmatter.name == dir_name {
            return Vec::new();
        }

        vec![Diagnostic::new(
            self.code(),
            format!(
                "frontmatter `name` (\"{}\") does not match the containing directory name (\"{dir_name}\")",
                skill.frontmatter.name
            ),
            self.default_severity(),
            &skill.path,
            skill.frontmatter.name_line,
            1,
        )
        .with_fix_suggestion(format!("rename the `name` field to \"{dir_name}\", or rename the directory to \"{}\"", skill.frontmatter.name))]
    }
}

/// `SL005` `invalid-name-format`: the frontmatter `name` is not kebab-case
/// (contains whitespace, uppercase letters, or characters other than
/// lowercase ASCII letters, digits, and hyphens).
pub struct InvalidNameFormat;

impl_rule!(InvalidNameFormat, "SL005", "invalid-name-format", Error);

impl SkillRule for InvalidNameFormat {
    fn check(
        &self,
        skill: &Skill,
        _config: &LintConfig,
        _tokens: &crate::token::TokenCounter,
    ) -> Vec<Diagnostic> {
        let name = &skill.frontmatter.name;
        if name.trim().is_empty() || is_kebab_case(name) {
            return Vec::new();
        }

        vec![Diagnostic::new(
            self.code(),
            format!("`name` (\"{name}\") is not kebab-case"),
            self.default_severity(),
            &skill.path,
            skill.frontmatter.name_line,
            1,
        )
        .with_fix_suggestion(format!(
            "use lowercase letters, digits, and hyphens only, e.g. \"{}\"",
            crate::text::to_kebab_case(name)
        ))]
    }
}

fn is_kebab_case(s: &str) -> bool {
    if s.is_empty() || s.starts_with('-') || s.ends_with('-') || s.contains("--") {
        return false;
    }
    s.chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}
