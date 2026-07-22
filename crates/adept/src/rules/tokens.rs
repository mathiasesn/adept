//! `SL3xx` token budget rules.
//!
//! Where [`super::description::TooLong`] (`SL202`) flags an overlong
//! description as a *quality* concern (`Warning`), [`DescriptionTokenBudget`]
//! (`SL301`) flags the same condition as a hard budget breach (`Error`) so
//! CI can gate on it independently of the softer style rules.

use std::fs;

use crate::diagnostic::{Diagnostic, Severity};
use crate::skill::Skill;
use crate::token::TokenCounter;

use super::{LintConfig, Rule, SkillRule};

/// `SL301` `description-tokens-over-budget`: the description exceeds
/// [`LintConfig::description_max_tokens`]. See the module docs for how this
/// differs from `SL202`.
pub struct DescriptionTokenBudget;

impl Rule for DescriptionTokenBudget {
    fn code(&self) -> &'static str {
        "SL301"
    }
    fn name(&self) -> &'static str {
        "description-tokens-over-budget"
    }
    fn default_severity(&self) -> Severity {
        Severity::Error
    }
}

impl SkillRule for DescriptionTokenBudget {
    fn check(&self, skill: &Skill, config: &LintConfig, tokens: &TokenCounter) -> Vec<Diagnostic> {
        let count = tokens.count(&skill.frontmatter.description);
        if count > config.description_max_tokens {
            vec![Diagnostic::new(
                self.code(),
                format!(
                    "description token budget exceeded: {count} tokens (budget: {})",
                    config.description_max_tokens
                ),
                self.default_severity(),
                &skill.path,
                skill.frontmatter.description_line,
                1,
            )
            .with_fix_suggestion("shorten the description below the configured token budget")]
        } else {
            Vec::new()
        }
    }
}

/// `SL302` `body-tokens-over-budget`: the SKILL.md body exceeds
/// [`LintConfig::body_max_tokens`].
pub struct BodyTokenBudget;

impl Rule for BodyTokenBudget {
    fn code(&self) -> &'static str {
        "SL302"
    }
    fn name(&self) -> &'static str {
        "body-tokens-over-budget"
    }
    fn default_severity(&self) -> Severity {
        Severity::Error
    }
}

impl SkillRule for BodyTokenBudget {
    fn check(&self, skill: &Skill, config: &LintConfig, tokens: &TokenCounter) -> Vec<Diagnostic> {
        let count = tokens.count(&skill.body);
        if count > config.body_max_tokens {
            vec![Diagnostic::new(
                self.code(),
                format!(
                    "SKILL.md body is {count} tokens, over the budget of {}",
                    config.body_max_tokens
                ),
                self.default_severity(),
                &skill.path,
                skill.body_line_offset,
                1,
            )
            .with_fix_suggestion(
                "move detailed reference material into companion files loaded on demand",
            )]
        } else {
            Vec::new()
        }
    }
}

/// `SL303` `companion-file-bloat`: a companion file (any file other than
/// SKILL.md in the skill's directory) exceeds
/// [`LintConfig::companion_file_max_tokens`].
pub struct CompanionFileBloat;

impl Rule for CompanionFileBloat {
    fn code(&self) -> &'static str {
        "SL303"
    }
    fn name(&self) -> &'static str {
        "companion-file-bloat"
    }
    fn default_severity(&self) -> Severity {
        Severity::Warning
    }
}

impl SkillRule for CompanionFileBloat {
    fn check(&self, skill: &Skill, config: &LintConfig, tokens: &TokenCounter) -> Vec<Diagnostic> {
        let Some(dir) = skill.path.parent() else {
            return Vec::new();
        };
        let Ok(entries) = fs::read_dir(dir) else {
            return Vec::new();
        };

        let mut diagnostics = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() || path == skill.path {
                continue;
            }
            let Ok(contents) = fs::read_to_string(&path) else {
                continue; // binary or unreadable companion file; not a token-budget concern
            };
            let count = tokens.count(&contents);
            if count > config.companion_file_max_tokens {
                diagnostics.push(
                    Diagnostic::new(
                        self.code(),
                        format!(
                            "companion file \"{}\" is {count} tokens, over the budget of {}",
                            path.file_name()
                                .map(|n| n.to_string_lossy())
                                .unwrap_or_default(),
                            config.companion_file_max_tokens
                        ),
                        self.default_severity(),
                        &skill.path,
                        1,
                        1,
                    )
                    .with_fix_suggestion("split the companion file or trim it down"),
                );
            }
        }
        diagnostics
    }
}
