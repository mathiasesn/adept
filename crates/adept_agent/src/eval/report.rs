//! [`EvalReport`]: the aggregate result of scoring one skill (optionally
//! within a wider [`adept::SkillSet`] for overlap detection), and its
//! human-readable renderer.

use serde::{Deserialize, Serialize};

use crate::eval::overlap::OverlapAdjudication;
use crate::eval::prompts::PROMPT_VERSION;
use crate::eval::tokens::TokenBloatReport;
use crate::eval::triggering::TriggeringReport;

/// The full result of `adept eval` for one skill.
///
/// Serializes to JSON directly via `serde` for `--format json`; use
/// [`EvalReport::render`] for the human-readable form.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvalReport {
    /// The [`PROMPT_VERSION`] active when this report was generated, so
    /// reports produced under different prompt revisions can be told apart
    /// (and not naively compared) later.
    pub prompt_version: String,
    /// The name of the skill this report is for.
    pub skill_name: String,
    /// Triggering-accuracy results, if that analysis was run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub triggering: Option<TriggeringReport>,
    /// Token-bloat analysis, if that analysis was run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_bloat: Option<TokenBloatReport>,
    /// Overlap/conflict adjudications against other skills in the same
    /// [`adept::SkillSet`], if that analysis was run. `None` when the
    /// analysis did not run at all; `Some(vec![])` when it ran and found no
    /// shortlisted overlaps — the two are rendered distinguishably.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overlaps: Option<Vec<OverlapAdjudication>>,
    /// Eval-dataset grading against `evals/evals.jsonl` run results, if that
    /// analysis was run. `None` when no `--results` were supplied, distinct
    /// from a report that ran and found nothing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evals: Option<adept::evals::EvalBenchmarkReport>,
}

impl EvalReport {
    /// Construct an empty report for `skill_name`, stamped with the current
    /// [`PROMPT_VERSION`]. Populate fields via struct-update syntax.
    #[must_use]
    pub fn new(skill_name: impl Into<String>) -> Self {
        Self {
            prompt_version: PROMPT_VERSION.to_string(),
            skill_name: skill_name.into(),
            triggering: None,
            token_bloat: None,
            overlaps: None,
            evals: None,
        }
    }

    /// Render this report as human-readable text, suitable for printing
    /// directly by `adept eval`.
    pub fn render(&self) -> String {
        let prompt_based_ran =
            self.triggering.is_some() || self.token_bloat.is_some() || self.overlaps.is_some();
        let mut out = String::new();
        out.push_str(&format!("Eval report for skill: {}\n", self.skill_name));
        if prompt_based_ran {
            out.push_str(&format!("(prompt set version: {})\n", self.prompt_version));
        }
        out.push('\n');

        if let Some(t) = &self.triggering {
            out.push_str("== Triggering accuracy ==\n");
            out.push_str(&format!(
                "precision: {:.2}  recall: {:.2}  f1: {:.2}  ({}/{} correct)\n",
                t.metrics.precision,
                t.metrics.recall,
                t.metrics.f1,
                t.metrics.correct,
                t.metrics.total
            ));
            for j in &t.judgements {
                let label = match j.prompt.label {
                    crate::eval::triggering::PromptLabel::Positive => "should-trigger",
                    crate::eval::triggering::PromptLabel::Negative => "should-not-trigger",
                };
                let status = if j.is_correct() { "OK" } else { "MISS" };
                out.push_str(&format!(
                    "  [{status}] ({label}, agreement {:.0}%) predicted={} :: {}\n",
                    j.agreement_rate * 100.0,
                    j.would_trigger,
                    j.prompt.text
                ));
            }
            out.push('\n');
        }

        if let Some(tb) = &self.token_bloat {
            out.push_str("== Token bloat ==\n");
            out.push_str(&format!(
                "description: {} tokens, body: {} tokens, companions: {} tokens, total: {} tokens\n",
                tb.description_tokens,
                tb.body_tokens,
                tb.companion_file_tokens.values().sum::<usize>(),
                tb.total_tokens
            ));
            if tb.suggestions.is_empty() {
                out.push_str("  no trimming suggestions\n");
            } else {
                for s in &tb.suggestions {
                    out.push_str(&format!("  - {s}\n"));
                }
            }
            out.push('\n');
        }

        if let Some(overlaps) = &self.overlaps {
            out.push_str("== Overlap/conflict detection ==\n");
            if overlaps.is_empty() {
                out.push_str("  no shortlisted overlaps\n");
            } else {
                for o in overlaps {
                    let kind = if o.conflicts {
                        "CONFLICT"
                    } else if o.overlaps {
                        "overlap"
                    } else {
                        "no issue"
                    };
                    out.push_str(&format!(
                        "  [{kind}] {} <-> {} (similarity {:.2}): {}\n",
                        o.skill_a, o.skill_b, o.similarity, o.explanation
                    ));
                    if !o.disambiguation.is_empty() {
                        out.push_str(&format!("      suggestion: {}\n", o.disambiguation));
                    }
                }
            }
            out.push('\n');
        }

        if let Some(evals) = &self.evals {
            out.push_str("== Eval-dataset grading ==\n");
            let skill_case_count = evals
                .cases
                .iter()
                .filter(|c| c.arm == adept::evals::Arm::Skill)
                .count();
            out.push_str(&format!(
                "pass rate: {:.0}% ({} cases)\n",
                evals.pass_rate * 100.0,
                skill_case_count
            ));
            if let (Some(baseline), Some(lift)) =
                (evals.baseline_pass_rate, evals.lift_percentage_points)
            {
                out.push_str(&format!(
                    "baseline pass rate: {:.0}%  lift: {:+.0}pp\n",
                    baseline * 100.0,
                    lift
                ));
            }
            out.push_str(&format!(
                "assertions: {}/{} met ({} skipped)\n",
                evals.assertions_met, evals.assertions_checked, evals.assertions_skipped
            ));
            if !evals.skipped_reasons.is_empty() {
                for (reason, count) in &evals.skipped_reasons {
                    out.push_str(&format!("  skipped: {reason} ({count})\n"));
                }
            }
            if !evals.unknown_result_ids.is_empty() {
                out.push_str(&format!(
                    "  results with unknown case id: {:?}\n",
                    evals.unknown_result_ids
                ));
            }
            if !evals.unmatched_cases.is_empty() {
                out.push_str(&format!(
                    "  dataset cases with no result: {:?}\n",
                    evals.unmatched_cases
                ));
            }
            if let (Some(tin), Some(tout)) = (evals.tokens_in, evals.tokens_out) {
                out.push_str(&format!("tokens: {tin} in / {tout} out\n"));
            }
        }

        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eval::overlap::OverlapAdjudication;
    use crate::eval::tokens::TokenBloatReport;
    use crate::eval::triggering::{CandidatePrompt, Metrics, PromptJudgement, PromptLabel};
    use std::collections::BTreeMap;

    fn sample_report() -> EvalReport {
        EvalReport {
            prompt_version: PROMPT_VERSION.to_string(),
            skill_name: "pdf-filler".to_string(),
            triggering: Some(TriggeringReport {
                metrics: Metrics {
                    precision: 1.0,
                    recall: 0.5,
                    f1: 0.6667,
                    correct: 3,
                    total: 4,
                },
                judgements: vec![
                    PromptJudgement {
                        prompt: CandidatePrompt {
                            text: "Fill out this W-9 PDF for me".to_string(),
                            label: PromptLabel::Positive,
                        },
                        would_trigger: true,
                        votes: vec![true],
                        agreement_rate: 1.0,
                    },
                    PromptJudgement {
                        prompt: CandidatePrompt {
                            text: "What's the weather today?".to_string(),
                            label: PromptLabel::Negative,
                        },
                        would_trigger: false,
                        votes: vec![false],
                        agreement_rate: 1.0,
                    },
                ],
            }),
            token_bloat: Some(TokenBloatReport {
                description_tokens: 12,
                body_tokens: 340,
                companion_file_tokens: BTreeMap::from([("reference.md".into(), 88)]),
                total_tokens: 440,
                suggestions: vec!["Move the reference table to a companion file".to_string()],
            }),
            overlaps: Some(vec![OverlapAdjudication {
                skill_a: "pdf-filler".to_string(),
                skill_b: "pdf-writer".to_string(),
                similarity: 0.42,
                overlaps: true,
                conflicts: false,
                explanation: "Both fill PDF forms".to_string(),
                disambiguation: "Narrow pdf-writer to non-form documents".to_string(),
            }]),
            evals: None,
        }
    }

    #[test]
    fn render_snapshot() {
        insta::assert_snapshot!(sample_report().render());
    }

    #[test]
    fn json_round_trips() {
        let report = sample_report();
        let json = serde_json::to_string(&report).unwrap();
        let parsed: EvalReport = serde_json::from_str(&json).unwrap();
        assert_eq!(report, parsed);
    }
}
