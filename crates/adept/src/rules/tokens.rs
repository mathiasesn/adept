//! `SL3xx` token budget rules.
//!
//! `SL301` (`DescriptionTokenBudget`) is the sole rule for an overlong
//! `description`: an earlier `SL202` duplicated it exactly (same condition,
//! same default threshold) and has been retired. See
//! `rules/description.rs` for that history.

use std::fs;

use crate::diagnostic::{Diagnostic, Severity};
use crate::skill::Skill;
use crate::token::TokenCounter;

use super::{impl_rule, FixKind, FixRegion, LintConfig, Rule, SkillRule};

/// `SL301` `description-tokens-over-budget`: the description exceeds
/// [`LintConfig::description_max_tokens`].
pub struct DescriptionTokenBudget;

impl_rule!(
    DescriptionTokenBudget,
    "SL301",
    "description-tokens-over-budget",
    Error,
    Llm,
    Description
);

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

impl_rule!(
    BodyTokenBudget,
    "SL302",
    "body-tokens-over-budget",
    Error,
    Llm,
    Body
);

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
///
/// Bundled license files (e.g. `LICENSE.txt`, `LICENSE-APACHE`) are exempt:
/// they are boilerplate legal text, not skill content, and commonly exceed
/// any reasonable token budget without being a documentation smell. Files
/// under a top-level `evals/` directory (e.g. `evals/evals.jsonl`, written
/// by `adept create`) are exempt for the same reason: a synthetic eval
/// dataset is not skill content either. Note that
/// [`crate::companion::discover_companion_files`] is non-recursive today, so
/// a nested `evals/` file is never discovered in the first place and this
/// exemption is not expected to fire in practice; it is defence-in-depth for
/// if discovery ever becomes recursive.
pub struct CompanionFileBloat;

impl_rule!(CompanionFileBloat, "SL303", "companion-file-bloat", Warning);

impl SkillRule for CompanionFileBloat {
    fn check(&self, skill: &Skill, config: &LintConfig, tokens: &TokenCounter) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();
        for path in crate::companion::discover_companion_files(skill) {
            if path
                .file_name()
                .is_some_and(|n| crate::companion::is_license_file(&n.to_string_lossy()))
                || crate::companion::is_eval_dataset(&path)
            {
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
