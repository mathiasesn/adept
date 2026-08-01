//! `SL4xx` cross-skill rules: operate on a whole [`SkillSet`] rather than a
//! single [`Skill`], looking for conflicts between skills.

use std::collections::{HashMap, HashSet};

use crate::diagnostic::{Diagnostic, Severity};
use crate::skillset::SkillSet;
use crate::text::{jaccard, word_bag};
use crate::token::TokenCounter;

use super::{impl_rule, FixKind, LintConfig, Rule, SetRule};

/// `SL401` `duplicate-skill-name`: two or more skills share the same
/// frontmatter `name`.
pub struct DuplicateSkillName;

impl_rule!(DuplicateSkillName, "SL401", "duplicate-skill-name", Error);

impl SetRule for DuplicateSkillName {
    fn check(
        &self,
        set: &SkillSet,
        _config: &LintConfig,
        _tokens: &TokenCounter,
    ) -> Vec<Diagnostic> {
        let mut by_name: HashMap<&str, Vec<usize>> = HashMap::new();
        for (i, skill) in set.skills.iter().enumerate() {
            by_name
                .entry(skill.frontmatter.name.as_str())
                .or_default()
                .push(i);
        }

        let mut diagnostics = Vec::new();
        for (name, indices) in by_name {
            if indices.len() < 2 {
                continue;
            }
            for &i in &indices {
                let skill = &set.skills[i];
                let others: Vec<String> = indices
                    .iter()
                    .filter(|&&j| j != i)
                    .map(|&j| set.skills[j].path.display().to_string())
                    .collect();
                diagnostics.push(
                    Diagnostic::new(
                        self.code(),
                        format!(
                            "skill name \"{name}\" is also used by: {}",
                            others.join(", ")
                        ),
                        self.default_severity(),
                        &skill.path,
                        skill.frontmatter.name_line,
                        1,
                    )
                    .with_fix_suggestion("give each skill a unique `name`"),
                );
            }
        }
        diagnostics
    }
}

/// `SL402` `similar-description`: two skills' descriptions have a
/// word-level Jaccard similarity above
/// [`LintConfig::similar_description_threshold`].
pub struct SimilarDescription;

impl_rule!(SimilarDescription, "SL402", "similar-description", Warning);

impl SetRule for SimilarDescription {
    fn check(
        &self,
        set: &SkillSet,
        config: &LintConfig,
        _tokens: &TokenCounter,
    ) -> Vec<Diagnostic> {
        // Note: this rule's input is the description alone, at
        // `similar_description_threshold` (default 0.6) — distinct from
        // `adept_agent::eval::overlap`'s description_similarity, which uses name+description
        // at its own (lower, shortlisting) threshold. See that function's
        // docs.
        pairwise_similarity(
            self,
            set,
            |s| word_bag(&s.frontmatter.description),
            config.similar_description_threshold,
            |sim, other_name, other_path| {
                format!(
                    "description is {:.0}% similar to \"{other_name}\" ({})",
                    sim * 100.0,
                    other_path.display()
                )
            },
            "differentiate the descriptions so agents can tell the skills apart",
        )
    }
}

/// `SL403` `overlapping-trigger-phrasing`: two skills' descriptions share a
/// high proportion of bigram "shingles", suggesting they'll compete to
/// trigger on the same requests.
pub struct OverlappingTriggerPhrasing;

impl_rule!(
    OverlappingTriggerPhrasing,
    "SL403",
    "overlapping-trigger-phrasing",
    Warning
);

impl SetRule for OverlappingTriggerPhrasing {
    fn check(
        &self,
        set: &SkillSet,
        config: &LintConfig,
        _tokens: &TokenCounter,
    ) -> Vec<Diagnostic> {
        pairwise_similarity(
            self,
            set,
            |s| shingles(&s.frontmatter.description, 2),
            config.trigger_overlap_threshold,
            |sim, other_name, other_path| {
                format!(
                    "trigger phrasing overlaps {:.0}% with \"{other_name}\" ({})",
                    sim * 100.0,
                    other_path.display()
                )
            },
            "narrow the trigger conditions so the skills don't compete for the same requests",
        )
    }
}

fn shingles(text: &str, n: usize) -> HashSet<String> {
    let words: Vec<String> = crate::text::words(text).collect();
    if words.len() < n {
        return words.into_iter().collect();
    }
    words
        .windows(n)
        .map(|w| w.join(" "))
        .collect::<HashSet<_>>()
}

/// Shared O(n²) upper-triangle pairwise-similarity scan behind both
/// [`SimilarDescription`] (`SL402`) and [`OverlappingTriggerPhrasing`]
/// (`SL403`): build one similarity-input set per skill via `set_builder`,
/// skip any pair where either side's set is empty (nothing to compare), and
/// report both directions of every pair whose Jaccard similarity meets
/// `threshold` — one diagnostic on skill A naming skill B and vice versa,
/// so each skill's own file shows the finding regardless of which one a
/// reader opens first.
///
/// A free function (not a trait method) so it stays independent of `Rule:
/// Send + Sync` and can be called from either rule's `check`.
fn pairwise_similarity<R, F, M>(
    rule: &R,
    set: &SkillSet,
    set_builder: F,
    threshold: f64,
    message: M,
    suggestion: &str,
) -> Vec<Diagnostic>
where
    R: Rule,
    F: Fn(&crate::skill::Skill) -> HashSet<String>,
    M: Fn(f64, &str, &std::path::Path) -> String,
{
    let sets: Vec<HashSet<String>> = set.skills.iter().map(&set_builder).collect();

    let mut diagnostics = Vec::new();
    for i in 0..set.skills.len() {
        for j in (i + 1)..set.skills.len() {
            if sets[i].is_empty() || sets[j].is_empty() {
                continue;
            }
            let sim = jaccard(&sets[i], &sets[j]);
            if sim >= threshold {
                let a = &set.skills[i];
                let b = &set.skills[j];
                diagnostics.push(
                    Diagnostic::new(
                        rule.code(),
                        message(sim, &b.frontmatter.name, &b.path),
                        rule.default_severity(),
                        &a.path,
                        a.frontmatter.description_line,
                        1,
                    )
                    .with_fix_suggestion(suggestion),
                );
                diagnostics.push(
                    Diagnostic::new(
                        rule.code(),
                        message(sim, &a.frontmatter.name, &a.path),
                        rule.default_severity(),
                        &b.path,
                        b.frontmatter.description_line,
                        1,
                    )
                    .with_fix_suggestion(suggestion),
                );
            }
        }
    }
    diagnostics
}
