//! Shared cheap text-similarity primitives used by cross-skill heuristics.
//!
//! These are the tokenizer and Jaccard-similarity building blocks shared by
//! `adept`'s own `SL4xx` cross-skill rules (`rules::cross`) and by
//! `adept_score`'s offline overlap shortlist (`adept_score::overlap`).
//! Extracted here so the two crates can't silently drift apart on what
//! counts as a "word" or how similarity is computed; each caller still
//! chooses its own *input text* and *similarity threshold* independently
//! (see each caller's docs for why).

use std::collections::HashSet;

/// Tokenize `text` into a lowercased set of alphanumeric words.
///
/// Splits on any non-alphanumeric character, drops empty tokens, and
/// lowercases the rest. Used as the basis for Jaccard similarity between
/// two pieces of text (e.g. two skills' descriptions).
#[must_use]
pub fn word_bag(text: &str) -> HashSet<String> {
    words(text).collect()
}

/// Tokenize `text` into lowercased alphanumeric words, in order.
///
/// The ordered counterpart to [`word_bag`], for callers that need adjacency
/// (e.g. `SL403`'s shingles). Both share this definition of a "word" so the
/// rules can't drift apart on tokenization.
pub fn words(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
}

/// Jaccard similarity between two sets: `|intersection| / |union|`, or
/// `0.0` if both sets are empty (rather than dividing zero by zero).
#[must_use]
pub fn jaccard(a: &HashSet<String>, b: &HashSet<String>) -> f64 {
    // `|union| = |a| + |b| - |intersection|`, so one intersection pass is
    // enough — `union().count()` would walk both sets a second time.
    let intersection = a.intersection(b).count();
    let union = a.len() + b.len() - intersection;
    if union == 0 {
        0.0
    } else {
        intersection as f64 / union as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_text_is_fully_similar() {
        let a = word_bag("Fills PDF forms automatically");
        assert!(a.contains("fills"));
        assert_eq!(jaccard(&a, &a), 1.0);
    }

    #[test]
    fn disjoint_text_has_zero_similarity() {
        let a = word_bag("apples oranges");
        let b = word_bag("trucks planes");
        assert_eq!(jaccard(&a, &b), 0.0);
    }

    #[test]
    fn empty_sets_are_zero_not_nan() {
        let empty = word_bag("   ");
        assert_eq!(jaccard(&empty, &empty), 0.0);
    }
}
