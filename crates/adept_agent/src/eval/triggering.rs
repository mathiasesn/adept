//! Triggering-accuracy scoring: generate candidate prompts, judge each one
//! against only the skill's name+description, and report precision/recall/F1.

use adept::Skill;
use serde::{Deserialize, Serialize};

use crate::eval::prompts::{
    render, GENERATE_TRIGGER_PROMPTS_SYSTEM, GENERATE_TRIGGER_PROMPTS_USER_TEMPLATE,
    JUDGE_TRIGGER_SYSTEM, JUDGE_TRIGGER_USER_TEMPLATE,
};
use crate::eval::EvalError;
use crate::llm::{ChatMessage, ChatRequest, LlmClient};

/// The default number of candidate prompts to generate (half positive,
/// half negative) when [`TriggeringOptions::num_prompts`] is not
/// overridden. Kept even so the positive/negative split is exact.
pub const DEFAULT_NUM_PROMPTS: usize = 10;

/// Whether a candidate prompt is intended to trigger the skill or not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PromptLabel {
    /// A well-calibrated agent should invoke the skill for this prompt.
    Positive,
    /// A well-calibrated agent should NOT invoke the skill for this prompt.
    Negative,
}

/// One candidate user prompt generated for the triggering eval.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CandidatePrompt {
    /// The prompt text.
    pub text: String,
    /// The intended label.
    pub label: PromptLabel,
}

/// Options controlling a triggering-accuracy run.
#[derive(Debug, Clone)]
pub struct TriggeringOptions {
    /// The model to use for both prompt generation and judging.
    pub model: String,
    /// How many candidate prompts to generate if `fixed_prompts` is not
    /// given. Must be even; odd values are rounded down.
    pub num_prompts: usize,
    /// A seed passed through to the backend for reproducible sampling,
    /// where supported.
    pub seed: Option<u64>,
    /// How many independent judge samples to take per prompt and combine
    /// by majority vote. `1` (the default) disables majority voting. Values
    /// greater than 1 report an `agreement_rate` per prompt as a variance/
    /// confidence signal.
    pub judge_samples: usize,
    /// If set, skip LLM-based prompt generation entirely and judge exactly
    /// this fixed, reusable prompt set instead. This is how callers get a
    /// deterministic, reusable eval across runs per the product doc's
    /// "fixed default prompt set" requirement.
    pub fixed_prompts: Option<Vec<CandidatePrompt>>,
}

impl Default for TriggeringOptions {
    fn default() -> Self {
        Self {
            model: String::new(),
            num_prompts: DEFAULT_NUM_PROMPTS,
            seed: Some(0),
            judge_samples: 1,
            fixed_prompts: None,
        }
    }
}

/// The judge's verdict for one [`CandidatePrompt`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PromptJudgement {
    /// The prompt that was judged.
    pub prompt: CandidatePrompt,
    /// The majority-vote (or single-sample) verdict: would the skill be
    /// triggered?
    pub would_trigger: bool,
    /// The individual votes making up `would_trigger`, in order.
    pub votes: Vec<bool>,
    /// The fraction of `votes` agreeing with the majority verdict, in
    /// `[0.0, 1.0]`. `1.0` when `judge_samples <= 1`. A confidence signal:
    /// low agreement means the judge itself is unstable on this prompt.
    pub agreement_rate: f64,
}

impl PromptJudgement {
    /// Whether the majority verdict matches the intended label (a "correct"
    /// judgement).
    pub fn is_correct(&self) -> bool {
        (self.prompt.label == PromptLabel::Positive) == self.would_trigger
    }
}

/// Precision/recall/F1 over a set of triggering judgements.
///
/// Positives are prompts labeled [`PromptLabel::Positive`]; a "predicted
/// positive" is a judgement where `would_trigger` is `true`.
///
/// Edge cases (documented rather than left to chance, since they're part of
/// the scoring contract):
/// - No prompts at all: all metrics are `0.0`.
/// - No actual positives (all negatives): `recall` is `0.0` (there is
///   nothing to recall; this is the standard convention rather than
///   treating it as vacuously perfect).
/// - No predicted positives: `precision` is `0.0`.
/// - `precision + recall == 0.0`: `f1` is `0.0`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Metrics {
    /// True positives / (true positives + false positives).
    pub precision: f64,
    /// True positives / (true positives + false negatives).
    pub recall: f64,
    /// The harmonic mean of precision and recall.
    pub f1: f64,
    /// Count of prompts where the judge's verdict matched the intended
    /// label.
    pub correct: usize,
    /// Total prompts judged.
    pub total: usize,
}

/// Compute [`Metrics`] from a set of `(label, predicted_would_trigger)`
/// pairs. Pure and offline so the scoring math can be unit-tested directly.
pub fn precision_recall_f1(results: &[(PromptLabel, bool)]) -> Metrics {
    let mut true_positive = 0usize;
    let mut false_positive = 0usize;
    let mut false_negative = 0usize;
    let mut correct = 0usize;

    for (label, predicted) in results {
        let is_positive_label = *label == PromptLabel::Positive;
        if is_positive_label == *predicted {
            correct += 1;
        }
        match (is_positive_label, predicted) {
            (true, true) => true_positive += 1,
            (false, true) => false_positive += 1,
            (true, false) => false_negative += 1,
            (false, false) => {}
        }
    }

    let predicted_positive = true_positive + false_positive;
    let actual_positive = true_positive + false_negative;

    let precision = if predicted_positive == 0 {
        0.0
    } else {
        true_positive as f64 / predicted_positive as f64
    };
    let recall = if actual_positive == 0 {
        0.0
    } else {
        true_positive as f64 / actual_positive as f64
    };
    let f1 = if precision + recall == 0.0 {
        0.0
    } else {
        2.0 * precision * recall / (precision + recall)
    };

    Metrics {
        precision,
        recall,
        f1,
        correct,
        total: results.len(),
    }
}

/// The full result of a triggering-accuracy run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TriggeringReport {
    /// Aggregate precision/recall/F1 over all judged prompts.
    pub metrics: Metrics,
    /// Per-prompt judgements, in the order they were generated/supplied.
    pub judgements: Vec<PromptJudgement>,
}

#[derive(Debug, Deserialize)]
struct RawPromptList {
    prompts: Vec<RawPrompt>,
}

#[derive(Debug, Deserialize)]
struct RawPrompt {
    text: String,
    label: RawLabel,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum RawLabel {
    Positive,
    Negative,
}

#[derive(Debug, Deserialize)]
struct RawJudgement {
    would_trigger: bool,
}

/// Run a full triggering-accuracy evaluation for `skill`.
///
/// If `options.fixed_prompts` is `Some`, that fixed prompt set is judged
/// directly (no generation call). Otherwise, one generation call produces
/// `options.num_prompts` candidate prompts, then each is judged
/// (`options.judge_samples` times, majority-voted) using ONLY the skill's
/// name and description.
///
/// # Errors
/// Returns [`EvalError`] if the LLM client errors, or if a response cannot
/// be parsed as the expected JSON shape.
pub async fn score_triggering(
    client: &dyn LlmClient,
    skill: &Skill,
    options: &TriggeringOptions,
) -> Result<TriggeringReport, EvalError> {
    let prompts = match &options.fixed_prompts {
        Some(fixed) => fixed.clone(),
        None => generate_prompts(client, skill, options).await?,
    };

    let mut judgements = Vec::with_capacity(prompts.len());
    for prompt in prompts {
        let judgement = judge_prompt(client, skill, &prompt, options).await?;
        judgements.push(judgement);
    }

    let results: Vec<(PromptLabel, bool)> = judgements
        .iter()
        .map(|j| (j.prompt.label, j.would_trigger))
        .collect();
    let metrics = precision_recall_f1(&results);

    Ok(TriggeringReport {
        metrics,
        judgements,
    })
}

async fn generate_prompts(
    client: &dyn LlmClient,
    skill: &Skill,
    options: &TriggeringOptions,
) -> Result<Vec<CandidatePrompt>, EvalError> {
    let count = options.num_prompts - (options.num_prompts % 2);
    let user = render(
        GENERATE_TRIGGER_PROMPTS_USER_TEMPLATE,
        &[
            ("skill_name", &skill.frontmatter.name),
            ("skill_description", &skill.frontmatter.description),
            ("count", &count.to_string()),
        ],
    );
    let request = ChatRequest::new(
        options.model.clone(),
        vec![
            ChatMessage::system(GENERATE_TRIGGER_PROMPTS_SYSTEM),
            ChatMessage::user(user),
        ],
    )
    .with_temperature(0.7)
    .with_seed(options.seed)
    .with_json_response(true);

    let response = client.chat(request).await?;
    let parsed: RawPromptList = serde_json::from_str(&response.content)
        .map_err(|e| EvalError::MalformedLlmJson(format!("prompt generation: {e}")))?;

    Ok(parsed
        .prompts
        .into_iter()
        .map(|p| CandidatePrompt {
            text: p.text,
            label: match p.label {
                RawLabel::Positive => PromptLabel::Positive,
                RawLabel::Negative => PromptLabel::Negative,
            },
        })
        .collect())
}

async fn judge_prompt(
    client: &dyn LlmClient,
    skill: &Skill,
    prompt: &CandidatePrompt,
    options: &TriggeringOptions,
) -> Result<PromptJudgement, EvalError> {
    let samples = options.judge_samples.max(1);
    let user = render(
        JUDGE_TRIGGER_USER_TEMPLATE,
        &[
            ("skill_name", &skill.frontmatter.name),
            ("skill_description", &skill.frontmatter.description),
            ("user_prompt", &prompt.text),
        ],
    );

    let mut votes = Vec::with_capacity(samples);
    for _ in 0..samples {
        let request = ChatRequest::new(
            options.model.clone(),
            vec![
                ChatMessage::system(JUDGE_TRIGGER_SYSTEM),
                ChatMessage::user(user.clone()),
            ],
        )
        // Always temperature 0 for judging, per the product doc's variance
        // mitigation: fixed temperature, fixed prompt set, seedable.
        .with_temperature(0.0)
        .with_seed(options.seed)
        .with_json_response(true);

        let response = client.chat(request).await?;
        let parsed: RawJudgement = serde_json::from_str(&response.content)
            .map_err(|e| EvalError::MalformedLlmJson(format!("trigger judge: {e}")))?;
        votes.push(parsed.would_trigger);
    }

    let true_votes = votes.iter().filter(|v| **v).count();
    let false_votes = votes.len() - true_votes;
    let would_trigger = true_votes >= false_votes;
    let agreement = true_votes.max(false_votes) as f64 / votes.len() as f64;

    Ok(PromptJudgement {
        prompt: prompt.clone(),
        would_trigger,
        votes,
        agreement_rate: agreement,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn perfect_metrics() {
        let results = vec![
            (PromptLabel::Positive, true),
            (PromptLabel::Positive, true),
            (PromptLabel::Negative, false),
            (PromptLabel::Negative, false),
        ];
        let m = precision_recall_f1(&results);
        assert_eq!(m.precision, 1.0);
        assert_eq!(m.recall, 1.0);
        assert_eq!(m.f1, 1.0);
        assert_eq!(m.correct, 4);
    }

    #[test]
    fn all_wrong_metrics() {
        let results = vec![
            (PromptLabel::Positive, false),
            (PromptLabel::Negative, true),
        ];
        let m = precision_recall_f1(&results);
        assert_eq!(m.precision, 0.0);
        assert_eq!(m.recall, 0.0);
        assert_eq!(m.f1, 0.0);
        assert_eq!(m.correct, 0);
    }

    #[test]
    fn zero_positives_edge_case() {
        let results = vec![
            (PromptLabel::Negative, false),
            (PromptLabel::Negative, false),
        ];
        let m = precision_recall_f1(&results);
        // No actual positives and no predicted positives: both defined as 0.
        assert_eq!(m.precision, 0.0);
        assert_eq!(m.recall, 0.0);
        assert_eq!(m.f1, 0.0);
        assert_eq!(m.correct, 2);
    }

    #[test]
    fn empty_results() {
        let m = precision_recall_f1(&[]);
        assert_eq!(m.precision, 0.0);
        assert_eq!(m.recall, 0.0);
        assert_eq!(m.f1, 0.0);
        assert_eq!(m.total, 0);
    }

    #[test]
    fn partial_metrics() {
        // 2 actual positives, judge predicts positive for 1 of them plus
        // 1 false positive on a negative.
        let results = vec![
            (PromptLabel::Positive, true),
            (PromptLabel::Positive, false),
            (PromptLabel::Negative, true),
            (PromptLabel::Negative, false),
        ];
        let m = precision_recall_f1(&results);
        assert_eq!(m.precision, 0.5); // 1 tp / (1 tp + 1 fp)
        assert_eq!(m.recall, 0.5); // 1 tp / (1 tp + 1 fn)
        assert!((m.f1 - 0.5).abs() < 1e-9);
    }
}
