//! The `SL302` (`body-tokens-over-budget`) token-conservation guard.
//!
//! `SL302`'s fix suggestion is to *relocate* detailed material into
//! companion files, not to delete it. A candidate that merely deletes body
//! content to get under budget "fixes" the diagnostic while destroying
//! documentation, which is worse than not fixing it at all — so any
//! candidate produced for an `SL302` violation must be checked here before
//! it can be accepted.

use std::collections::BTreeMap;
use std::path::PathBuf;

use adept::{Skill, TokenCounter};

use crate::candidate::FixCandidate;

/// Allowed fractional shrinkage (2%) in total token count between the
/// original skill's content and a candidate's, before the candidate is
/// rejected as having lost content rather than relocated it.
///
/// Rationale: relocation is a move, so total content across SKILL.md plus
/// all companion files should not meaningfully shrink. A small tolerance
/// (rather than requiring `>=` exactly) absorbs two sources of harmless
/// noise: `adept_fmt` canonicalization (whitespace/heading normalization
/// can shave a handful of tokens) and the fact that the model's pointer
/// text in the body ("see REFERENCE.md for...") is shorter than the
/// material it points at. Growth is always fine — the guard only rejects
/// shrinkage beyond this tolerance.
pub const CONTENT_TOLERANCE: f64 = 0.02;

/// The candidate's total token count fell below the original's by more than
/// [`CONTENT_TOLERANCE`].
#[derive(Debug, thiserror::Error)]
#[error(
    "candidate lost content: {candidate_tokens} tokens vs {original_tokens} original \
     (minimum allowed: {min_allowed_tokens}, tolerance {tolerance_pct:.1}%)"
)]
pub struct ConservationError {
    /// Total tokens across the original SKILL.md and its companion files.
    pub original_tokens: usize,
    /// Total tokens across the candidate SKILL.md and its companion files.
    pub candidate_tokens: usize,
    /// The minimum candidate token count that would have passed.
    pub min_allowed_tokens: usize,
    /// [`CONTENT_TOLERANCE`], as a percentage, for display.
    pub tolerance_pct: f64,
}

/// Check that `candidate` conserves `original`'s content: the sum of tokens
/// across (candidate SKILL.md + all candidate companion files) must be at
/// least `original_total * (1.0 - CONTENT_TOLERANCE)`, where `original_total`
/// is computed over `original`'s SKILL.md source plus every pre-existing
/// companion file in `original_companions`.
///
/// `original_companions` is the full set of the original skill's companion
/// files (path -> contents), read once by the caller — this function never
/// touches disk, so it can be called every round without re-reading or
/// re-tokenizing files that haven't changed.
///
/// # Errors
/// Returns [`ConservationError`] if the candidate's total falls below the
/// tolerated minimum.
pub fn conserves_content(
    original: &Skill,
    candidate: &FixCandidate,
    tokens: &TokenCounter,
    original_companions: &BTreeMap<PathBuf, String>,
) -> Result<(), ConservationError> {
    let original_companion_tokens: usize = original_companions
        .values()
        .map(|contents| tokens.count(contents))
        .sum();
    let original_tokens = tokens.count(&original.source) + original_companion_tokens;

    // The candidate's companion total must cover the same file set as the
    // original's: every pre-existing companion the candidate didn't touch
    // still carries its original content over unchanged, so it must be
    // counted too — otherwise a large untouched companion makes the
    // candidate look like it lost content it never touched.
    let mut candidate_companion_tokens = 0usize;
    for (path, original_contents) in original_companions {
        let contents = candidate.companions.get(path).unwrap_or(original_contents);
        candidate_companion_tokens += tokens.count(contents);
    }
    for (path, contents) in &candidate.companions {
        if !original_companions.contains_key(path) {
            candidate_companion_tokens += tokens.count(contents);
        }
    }
    let candidate_tokens = tokens.count(&candidate.skill_source) + candidate_companion_tokens;

    let min_allowed_tokens = (original_tokens as f64 * (1.0 - CONTENT_TOLERANCE)).floor() as usize;

    if candidate_tokens < min_allowed_tokens {
        return Err(ConservationError {
            original_tokens,
            candidate_tokens,
            min_allowed_tokens,
            tolerance_pct: CONTENT_TOLERANCE * 100.0,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    fn skill(body: &str) -> Skill {
        let source = format!("---\nname: demo\ndescription: A demo skill for tests\n---\n{body}");
        Skill {
            path: PathBuf::from("/nonexistent/adept_agent_relocate_test/SKILL.md"),
            frontmatter: adept::Frontmatter {
                name: "demo".into(),
                name_line: 2,
                description: "A demo skill for tests".into(),
                description_line: 3,
                license: None,
                license_line: None,
                extra: Default::default(),
            },
            body: body.to_string(),
            body_line_offset: 5,
            source,
        }
    }

    #[test]
    fn accepts_pure_relocation() {
        let counter = TokenCounter::default();
        let original = skill(&"word ".repeat(1000));
        let candidate = FixCandidate {
            skill_source: format!(
                "---\nname: demo\ndescription: A demo skill for tests\n---\n{}",
                "word ".repeat(200)
            ),
            companions: BTreeMap::from([(
                PathBuf::from("/nonexistent/adept_agent_relocate_test/REFERENCE.md"),
                "word ".repeat(800),
            )]),
        };
        let original_companions = BTreeMap::new();
        assert!(conserves_content(&original, &candidate, &counter, &original_companions).is_ok());
    }

    /// Regression test for the conservation guard false-rejecting when a
    /// large pre-existing companion the candidate never touched: the
    /// candidate's companion total must still count that file's original
    /// content, not just what's in `candidate.companions`.
    #[test]
    fn accepts_candidate_that_leaves_a_large_pre_existing_companion_untouched() {
        let counter = TokenCounter::default();

        let body = "word ".repeat(50);
        let source = format!("---\nname: demo\ndescription: A demo skill for tests\n---\n{body}");
        let untouched_path = PathBuf::from("/nonexistent/adept_agent_relocate_test/UNTOUCHED.md");
        // A large pre-existing companion the model never edits.
        let untouched_content = "word ".repeat(5000);

        let original = Skill {
            path: PathBuf::from("/nonexistent/adept_agent_relocate_test/SKILL.md"),
            frontmatter: adept::Frontmatter {
                name: "demo".into(),
                name_line: 2,
                description: "A demo skill for tests".into(),
                description_line: 3,
                license: None,
                license_line: None,
                extra: Default::default(),
            },
            body: body.clone(),
            body_line_offset: 5,
            source: source.clone(),
        };
        let original_companions = BTreeMap::from([(untouched_path, untouched_content)]);

        // Candidate leaves the body (and thus total content) unchanged and
        // touches no companions at all — a pure no-op, which must be
        // accepted since nothing was lost.
        let candidate = FixCandidate {
            skill_source: source,
            companions: BTreeMap::new(),
        };

        assert!(
            conserves_content(&original, &candidate, &counter, &original_companions).is_ok(),
            "guard must not penalize an untouched pre-existing companion"
        );
    }

    #[test]
    fn rejects_deletion_without_relocation() {
        let counter = TokenCounter::default();
        let original = skill(&"word ".repeat(1000));
        let candidate = FixCandidate {
            skill_source: format!(
                "---\nname: demo\ndescription: A demo skill for tests\n---\n{}",
                "word ".repeat(100)
            ),
            companions: BTreeMap::new(),
        };
        let original_companions = BTreeMap::new();
        let err =
            conserves_content(&original, &candidate, &counter, &original_companions).unwrap_err();
        assert!(err.candidate_tokens < err.min_allowed_tokens);
    }
}
