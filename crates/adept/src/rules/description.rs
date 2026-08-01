//! `SL2xx` description/triggering heuristic rules.

use crate::diagnostic::{Diagnostic, Severity};
use crate::skill::Skill;
use crate::text::{word_bag, words};
use crate::token::TokenCounter;

use super::{impl_rule, FixKind, FixRegion, LintConfig, Rule, SkillRule};

const TRIGGER_PHRASES: &[&str] = &[
    "use when",
    "use this when",
    "use this skill when",
    "when the user",
    "trigger on",
    "triggers on",
    "when asked",
    "when you need",
    "when working with",
    "for use when",
];

const NEGATIVE_PHRASES: &[&str] = &[
    "do not use",
    "don't use",
    "not for",
    "avoid using",
    "should not be used",
];

/// `SL201` `description-too-short`: the description is below
/// [`LintConfig::description_min_tokens`].
pub struct TooShort;

impl_rule!(TooShort, "SL201", "description-too-short", Warning);

impl SkillRule for TooShort {
    fn check(&self, skill: &Skill, config: &LintConfig, tokens: &TokenCounter) -> Vec<Diagnostic> {
        let desc = &skill.frontmatter.description;
        if desc.trim().is_empty() {
            return Vec::new(); // reported by SL001
        }
        let count = tokens.count(desc);
        if count < config.description_min_tokens {
            vec![Diagnostic::new(
                self.code(),
                format!(
                    "description is only {count} tokens, below the minimum of {}",
                    config.description_min_tokens
                ),
                self.default_severity(),
                &skill.path,
                skill.frontmatter.description_line,
                1,
            )
            .with_fix_suggestion(
                "expand the description to state both what the skill does and when to use it",
            )]
        } else {
            Vec::new()
        }
    }
}

// `SL202` (`description-too-long`) is retired: it duplicated `SL301`
// (`description-tokens-over-budget`) exactly, both firing on
// `description_max_tokens` for the same input with no distinct meaning.
// `SL3xx` is the token-budget family per the spec, so `SL301` is the
// surviving rule; see `docs/RULES.md` for the rationale. The code `SL202`
// is retired, not reused, so historical configs referencing it don't
// silently start meaning something else.

/// `SL203` `missing-trigger-phrase`: the description does not state *when*
/// to use the skill (no recognizable trigger phrasing such as "use when",
/// "when the user", "triggers on").
pub struct MissingTriggerPhrase;

impl_rule!(
    MissingTriggerPhrase,
    "SL203",
    "missing-trigger-phrase",
    Warning
);

impl SkillRule for MissingTriggerPhrase {
    fn check(
        &self,
        skill: &Skill,
        _config: &LintConfig,
        _tokens: &TokenCounter,
    ) -> Vec<Diagnostic> {
        let desc = skill.frontmatter.description.to_lowercase();
        if desc.trim().is_empty() {
            return Vec::new();
        }
        let has_trigger = TRIGGER_PHRASES.iter().any(|p| desc.contains(p));
        if has_trigger {
            Vec::new()
        } else {
            vec![Diagnostic::new(
                self.code(),
                "description does not state when the skill should be used",
                self.default_severity(),
                &skill.path,
                skill.frontmatter.description_line,
                1,
            )
            .with_fix_suggestion("add trigger phrasing, e.g. \"Use when the user asks to...\"")]
        }
    }
}

/// `SL204` `first-person-description`: the description is written in first
/// person ("I will...", "I can...") instead of third person.
pub struct FirstPerson;

impl_rule!(FirstPerson, "SL204", "first-person-description", Warning);

impl SkillRule for FirstPerson {
    fn check(
        &self,
        skill: &Skill,
        _config: &LintConfig,
        _tokens: &TokenCounter,
    ) -> Vec<Diagnostic> {
        let desc = &skill.frontmatter.description;
        if desc.trim().is_empty() {
            return Vec::new(); // reported by SL001
        }
        let lower = desc.to_lowercase();
        let first_person = ["i will", "i can", "i am able to", "i'll", "i'm able to"]
            .iter()
            .any(|p| lower.contains(p))
            || lower
                .split_whitespace()
                .next()
                .is_some_and(|w| w.trim_matches(|c: char| !c.is_alphanumeric()) == "i");
        if first_person {
            vec![Diagnostic::new(
                self.code(),
                "description is written in first person; descriptions should be third person",
                self.default_severity(),
                &skill.path,
                skill.frontmatter.description_line,
                1,
            )
            .with_fix_suggestion(
                "rewrite in third person, e.g. \"Extracts...\" instead of \"I extract...\"",
            )]
        } else {
            Vec::new()
        }
    }
}

/// Minimum description word count `SL205` requires before judging overlap
/// with the name at all.
///
/// Rationale: below two words there's nothing meaningful to compare against
/// the name, so treating it as "restates the name" would be a false
/// positive on descriptions that are simply too short (already `SL201`'s
/// job to flag).
const RESTATES_NAME_MIN_DESC_WORDS: usize = 2;

/// Fraction (0.0-1.0) of description words that must also appear in the name
/// for `SL205` to fire.
///
/// Rationale: 0.8 tolerates a couple of extra words (e.g. "when", "for")
/// while still catching descriptions that are little more than the name
/// reworded.
const RESTATES_NAME_OVERLAP_RATIO: f64 = 0.8;

/// `SL205` `description-restates-name`: the description is just the name
/// reworded, with no additional information about behavior or triggering.
pub struct RestatesName;

impl_rule!(RestatesName, "SL205", "description-restates-name", Warning);

impl SkillRule for RestatesName {
    fn check(
        &self,
        skill: &Skill,
        _config: &LintConfig,
        _tokens: &TokenCounter,
    ) -> Vec<Diagnostic> {
        let name_words = word_bag(&skill.frontmatter.name);
        if name_words.is_empty() {
            return Vec::new();
        }
        let desc_words: Vec<String> = words(&skill.frontmatter.description).collect();
        if desc_words.len() < RESTATES_NAME_MIN_DESC_WORDS {
            return Vec::new();
        }
        let overlap = desc_words
            .iter()
            .filter(|w| name_words.contains(*w))
            .count();
        let ratio = overlap as f64 / desc_words.len() as f64;
        if ratio >= RESTATES_NAME_OVERLAP_RATIO {
            vec![Diagnostic::new(
                self.code(),
                "description is little more than the skill name reworded",
                self.default_severity(),
                &skill.path,
                skill.frontmatter.description_line,
                1,
            )
            .with_fix_suggestion(
                "describe what the skill does and when to use it, not just its name",
            )]
        } else {
            Vec::new()
        }
    }
}

/// `SL206` `no-negative-guidance`: the description gives no guidance on when
/// *not* to use the skill (e.g. "do not use for..."). Informational only,
/// since not every skill needs negative guidance.
pub struct NoNegativeGuidance;

impl_rule!(
    NoNegativeGuidance,
    "SL206",
    "no-negative-guidance",
    Info,
    Llm,
    Description
);

impl SkillRule for NoNegativeGuidance {
    fn check(
        &self,
        skill: &Skill,
        _config: &LintConfig,
        _tokens: &TokenCounter,
    ) -> Vec<Diagnostic> {
        let desc = skill.frontmatter.description.to_lowercase();
        if desc.trim().is_empty() {
            return Vec::new();
        }
        let has_negative = NEGATIVE_PHRASES.iter().any(|p| desc.contains(p));
        if has_negative {
            Vec::new()
        } else {
            vec![Diagnostic::new(
                self.code(),
                "description gives no guidance on when not to use the skill",
                self.default_severity(),
                &skill.path,
                skill.frontmatter.description_line,
                1,
            )
            .with_fix_suggestion(
                "consider adding \"Do not use for...\" guidance to reduce over-triggering",
            )]
        }
    }
}
