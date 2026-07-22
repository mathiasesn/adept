//! `SL4xx` cross-skill rules: operate on a whole [`SkillSet`] rather than a
//! single [`Skill`], looking for conflicts between skills.

use std::collections::{HashMap, HashSet};

use crate::diagnostic::{Diagnostic, Severity};
use crate::skillset::SkillSet;
use crate::token::TokenCounter;

use super::{LintConfig, Rule, SetRule};

/// `SL401` `duplicate-skill-name`: two or more skills share the same
/// frontmatter `name`.
pub struct DuplicateSkillName;

impl Rule for DuplicateSkillName {
    fn code(&self) -> &'static str {
        "SL401"
    }
    fn name(&self) -> &'static str {
        "duplicate-skill-name"
    }
    fn default_severity(&self) -> Severity {
        Severity::Error
    }
}

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

impl Rule for SimilarDescription {
    fn code(&self) -> &'static str {
        "SL402"
    }
    fn name(&self) -> &'static str {
        "similar-description"
    }
    fn default_severity(&self) -> Severity {
        Severity::Warning
    }
}

impl SetRule for SimilarDescription {
    fn check(
        &self,
        set: &SkillSet,
        config: &LintConfig,
        _tokens: &TokenCounter,
    ) -> Vec<Diagnostic> {
        let word_sets: Vec<HashSet<String>> = set
            .skills
            .iter()
            .map(|s| word_bag(&s.frontmatter.description))
            .collect();

        let mut diagnostics = Vec::new();
        for i in 0..set.skills.len() {
            for j in (i + 1)..set.skills.len() {
                if word_sets[i].is_empty() || word_sets[j].is_empty() {
                    continue;
                }
                let sim = jaccard(&word_sets[i], &word_sets[j]);
                if sim >= config.similar_description_threshold {
                    let a = &set.skills[i];
                    let b = &set.skills[j];
                    diagnostics.push(
                        Diagnostic::new(
                            self.code(),
                            format!(
                                "description is {:.0}% similar to \"{}\" ({})",
                                sim * 100.0,
                                b.frontmatter.name,
                                b.path.display()
                            ),
                            self.default_severity(),
                            &a.path,
                            a.frontmatter.description_line,
                            1,
                        )
                        .with_fix_suggestion(
                            "differentiate the descriptions so agents can tell the skills apart",
                        ),
                    );
                    diagnostics.push(
                        Diagnostic::new(
                            self.code(),
                            format!(
                                "description is {:.0}% similar to \"{}\" ({})",
                                sim * 100.0,
                                a.frontmatter.name,
                                a.path.display()
                            ),
                            self.default_severity(),
                            &b.path,
                            b.frontmatter.description_line,
                            1,
                        )
                        .with_fix_suggestion(
                            "differentiate the descriptions so agents can tell the skills apart",
                        ),
                    );
                }
            }
        }
        diagnostics
    }
}

/// `SL403` `overlapping-trigger-phrasing`: two skills' descriptions share a
/// high proportion of bigram "shingles", suggesting they'll compete to
/// trigger on the same requests.
pub struct OverlappingTriggerPhrasing;

impl Rule for OverlappingTriggerPhrasing {
    fn code(&self) -> &'static str {
        "SL403"
    }
    fn name(&self) -> &'static str {
        "overlapping-trigger-phrasing"
    }
    fn default_severity(&self) -> Severity {
        Severity::Warning
    }
}

impl SetRule for OverlappingTriggerPhrasing {
    fn check(
        &self,
        set: &SkillSet,
        config: &LintConfig,
        _tokens: &TokenCounter,
    ) -> Vec<Diagnostic> {
        let shingle_sets: Vec<HashSet<String>> = set
            .skills
            .iter()
            .map(|s| shingles(&s.frontmatter.description, 2))
            .collect();

        let mut diagnostics = Vec::new();
        for i in 0..set.skills.len() {
            for j in (i + 1)..set.skills.len() {
                if shingle_sets[i].is_empty() || shingle_sets[j].is_empty() {
                    continue;
                }
                let sim = jaccard(&shingle_sets[i], &shingle_sets[j]);
                if sim >= config.trigger_overlap_threshold {
                    let a = &set.skills[i];
                    let b = &set.skills[j];
                    diagnostics.push(
                        Diagnostic::new(
                            self.code(),
                            format!(
                                "trigger phrasing overlaps {:.0}% with \"{}\" ({})",
                                sim * 100.0,
                                b.frontmatter.name,
                                b.path.display()
                            ),
                            self.default_severity(),
                            &a.path,
                            a.frontmatter.description_line,
                            1,
                        )
                        .with_fix_suggestion(
                            "narrow the trigger conditions so the skills don't compete for the same requests",
                        ),
                    );
                    diagnostics.push(
                        Diagnostic::new(
                            self.code(),
                            format!(
                                "trigger phrasing overlaps {:.0}% with \"{}\" ({})",
                                sim * 100.0,
                                a.frontmatter.name,
                                a.path.display()
                            ),
                            self.default_severity(),
                            &b.path,
                            b.frontmatter.description_line,
                            1,
                        )
                        .with_fix_suggestion(
                            "narrow the trigger conditions so the skills don't compete for the same requests",
                        ),
                    );
                }
            }
        }
        diagnostics
    }
}

fn word_bag(text: &str) -> HashSet<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| w.to_lowercase())
        .collect()
}

fn shingles(text: &str, n: usize) -> HashSet<String> {
    let words: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| w.to_lowercase())
        .collect();
    if words.len() < n {
        return words.into_iter().collect();
    }
    words
        .windows(n)
        .map(|w| w.join(" "))
        .collect::<HashSet<_>>()
}

fn jaccard(a: &HashSet<String>, b: &HashSet<String>) -> f64 {
    let intersection = a.intersection(b).count();
    let union = a.union(b).count();
    if union == 0 {
        0.0
    } else {
        intersection as f64 / union as f64
    }
}
