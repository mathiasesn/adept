//! `SL00x` frontmatter/naming rules.
//!
//! `SL003` (`malformed-frontmatter`) has no [`SkillRule`] here: a skill with
//! malformed frontmatter fails to parse entirely, so it is synthesized from
//! [`crate::skillset::SkillSet::errors`] by `Linter::lint_set` instead. See
//! [`super::parse_error_diagnostic`].

use crate::diagnostic::{Diagnostic, Severity};
use crate::skill::Skill;

use super::{impl_rule, FixKind, LintConfig, Rule, SkillRule};

/// `SL001` `missing-description`: the `description` frontmatter field is
/// present but empty (or whitespace-only).
///
/// A genuinely absent `description` key is instead reported as `SL001` from
/// the parse error path, since parsing requires the field to be present.
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

/// `SL002` `missing-name`: the `name` frontmatter field is present but empty
/// (or whitespace-only).
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
            to_kebab_case(name)
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

fn to_kebab_case(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_dash = false;
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash && !out.is_empty() {
            out.push('-');
            prev_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}
