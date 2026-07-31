//! Overlap/conflict detection across a [`adept::SkillSet`]: a cheap offline
//! pairwise description-similarity shortlist, followed by LLM adjudication
//! only on the shortlisted pairs.
//!
//! The offline similarity heuristic here is implemented locally and is
//! deliberately separate from any similarity rule a sibling agent may add
//! to the `adept` core crate (e.g. an `SL4xx` rule) — this module does
//! not depend on that.

use adept::Skill;
use serde::{Deserialize, Serialize};

use crate::eval::prompts::{
    render, OVERLAP_ADJUDICATION_SYSTEM, OVERLAP_ADJUDICATION_USER_TEMPLATE,
};
use crate::eval::EvalError;
use crate::llm::{ChatMessage, ChatRequest, LlmClient};

/// The default Jaccard-similarity threshold above which a pair of skills is
/// shortlisted for LLM adjudication.
pub const DEFAULT_SIMILARITY_THRESHOLD: f64 = 0.25;

/// A pair of skills shortlisted by offline similarity, with their score.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OverlapCandidate {
    /// Index of the first skill within the input slice.
    pub index_a: usize,
    /// Index of the second skill within the input slice.
    pub index_b: usize,
    /// Jaccard similarity of the two skills' name+description word sets, in
    /// `[0.0, 1.0]`.
    pub similarity: f64,
}

/// The LLM's adjudication of one shortlisted [`OverlapCandidate`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OverlapAdjudication {
    /// The name of the first skill.
    pub skill_a: String,
    /// The name of the second skill.
    pub skill_b: String,
    /// The offline similarity score that shortlisted this pair.
    pub similarity: f64,
    /// Whether a reasonable request could trigger both skills.
    pub overlaps: bool,
    /// Whether the two skills' purposes actively conflict/duplicate.
    pub conflicts: bool,
    /// The judge's explanation.
    pub explanation: String,
    /// A concrete suggestion for disambiguating the two skills, if
    /// `overlaps` or `conflicts` is true. Empty otherwise.
    pub disambiguation: String,
}

/// Jaccard similarity between the name+description word sets of two skills.
///
/// Uses the shared [`adept::text::word_bag`]/[`adept::text::jaccard`]
/// tokenizer, but deliberately different *input* and *threshold* than
/// `adept`'s own `SL402` (`similar-description`) rule: this shortlists
/// candidate pairs for (expensive) LLM adjudication, so it combines
/// name+description and uses a low threshold
/// ([`DEFAULT_SIMILARITY_THRESHOLD`], 0.25) to cast a wide net; `SL402` is a
/// standalone static lint, so it uses description alone at a higher
/// threshold (0.6) tuned to flag near-duplicates without an LLM pass to
/// filter out false positives.
pub fn description_similarity(a: &Skill, b: &Skill) -> f64 {
    let text_a = format!("{} {}", a.frontmatter.name, a.frontmatter.description);
    let text_b = format!("{} {}", b.frontmatter.name, b.frontmatter.description);
    let set_a = adept::text::word_bag(&text_a);
    let set_b = adept::text::word_bag(&text_b);

    if set_a.is_empty() && set_b.is_empty() {
        return 0.0;
    }
    adept::text::jaccard(&set_a, &set_b)
}

/// Shortlist all pairs of `skills` whose [`description_similarity`] meets or
/// exceeds `threshold`, sorted by descending similarity.
pub fn shortlist_candidates(skills: &[Skill], threshold: f64) -> Vec<OverlapCandidate> {
    let mut candidates = Vec::new();
    for i in 0..skills.len() {
        for j in (i + 1)..skills.len() {
            let similarity = description_similarity(&skills[i], &skills[j]);
            if similarity >= threshold {
                candidates.push(OverlapCandidate {
                    index_a: i,
                    index_b: j,
                    similarity,
                });
            }
        }
    }
    candidates.sort_by(|a, b| {
        b.similarity
            .partial_cmp(&a.similarity)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    candidates
}

#[derive(Debug, Deserialize)]
struct RawAdjudication {
    overlaps: bool,
    conflicts: bool,
    explanation: String,
    #[serde(default)]
    disambiguation: String,
}

/// Detect overlaps/conflicts across `skills`: shortlist candidate pairs
/// offline via [`shortlist_candidates`], then adjudicate each shortlisted
/// pair with the LLM using only each skill's name and description.
///
/// # Errors
/// Returns [`EvalError`] if the LLM client errors or a response cannot be
/// parsed as the expected JSON shape.
pub async fn detect_overlaps(
    client: &dyn LlmClient,
    skills: &[Skill],
    model: &str,
    threshold: f64,
) -> Result<Vec<OverlapAdjudication>, EvalError> {
    let candidates = shortlist_candidates(skills, threshold);
    let mut adjudications = Vec::with_capacity(candidates.len());

    for candidate in candidates {
        let a = &skills[candidate.index_a];
        let b = &skills[candidate.index_b];
        let user = render(
            OVERLAP_ADJUDICATION_USER_TEMPLATE,
            &[
                ("skill_a_name", &a.frontmatter.name),
                ("skill_a_description", &a.frontmatter.description),
                ("skill_b_name", &b.frontmatter.name),
                ("skill_b_description", &b.frontmatter.description),
            ],
        );
        let request = ChatRequest::new(
            model.to_string(),
            vec![
                ChatMessage::system(OVERLAP_ADJUDICATION_SYSTEM),
                ChatMessage::user(user),
            ],
        )
        .with_temperature(0.0)
        .with_json_response(true);

        let response = client.chat(request).await?;
        let parsed: RawAdjudication = serde_json::from_str(&response.content)
            .map_err(|e| EvalError::MalformedLlmJson(format!("overlap adjudication: {e}")))?;

        adjudications.push(OverlapAdjudication {
            skill_a: a.frontmatter.name.clone(),
            skill_b: b.frontmatter.name.clone(),
            similarity: candidate.similarity,
            overlaps: parsed.overlaps,
            conflicts: parsed.conflicts,
            explanation: parsed.explanation,
            disambiguation: parsed.disambiguation,
        });
    }

    Ok(adjudications)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_descriptions_are_fully_similar() {
        let set = adept::text::word_bag("Fills PDF forms automatically");
        assert!(set.contains("fills"));
        let sim = adept::text::jaccard(&set, &set);
        assert_eq!(sim, 1.0);
    }

    #[test]
    fn unrelated_descriptions_have_low_similarity() {
        let a = adept::text::word_bag("Fills PDF forms automatically for tax filing");
        let b = adept::text::word_bag("Generates weather forecasts from satellite data");
        assert!(adept::text::jaccard(&a, &b) < 0.2);
    }

    #[tokio::test]
    async fn shortlist_and_adjudicate_end_to_end() {
        use crate::llm::MockLlmClient;
        use adept::parse_skill;
        use std::io::Write;

        let dir = std::env::temp_dir().join(format!(
            "adept_agent_eval_overlap_test_{}_{}",
            std::process::id(),
            {
                use std::time::{SystemTime, UNIX_EPOCH};
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            }
        ));

        let write = |name: &str, description: &str| -> std::path::PathBuf {
            let skill_dir = dir.join(name);
            std::fs::create_dir_all(&skill_dir).unwrap();
            let path = skill_dir.join("SKILL.md");
            let mut file = std::fs::File::create(&path).unwrap();
            writeln!(
                file,
                "---\nname: {name}\ndescription: {description}\n---\nBody."
            )
            .unwrap();
            path
        };

        let a = parse_skill(write("pdf-filler", "Fills PDF forms with user data")).unwrap();
        let b = parse_skill(write("pdf-writer", "Fills PDF forms and writes documents")).unwrap();
        let c = parse_skill(write("weather", "Generates weather forecasts")).unwrap();
        let skills = vec![a, b, c];

        let candidates = shortlist_candidates(&skills, 0.2);
        assert_eq!(candidates.len(), 1);
        assert_eq!((candidates[0].index_a, candidates[0].index_b), (0, 1));

        let mock = MockLlmClient::with_texts(vec![
            r#"{"overlaps": true, "conflicts": false, "explanation": "both fill PDFs", "disambiguation": "merge them"}"#,
        ]);
        let adjudications = detect_overlaps(&mock, &skills, "test-model", 0.2)
            .await
            .unwrap();
        assert_eq!(adjudications.len(), 1);
        assert!(adjudications[0].overlaps);
        assert!(!adjudications[0].conflicts);

        std::fs::remove_dir_all(&dir).ok();
    }
}
