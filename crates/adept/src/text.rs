//! Shared cheap text primitives used across the crate stack.
//!
//! These are the tokenizer, Jaccard-similarity, and kebab-casing building
//! blocks shared by `adept`'s own rules (`rules::cross`'s `SL4xx`,
//! `rules::frontmatter`'s name suggestions) and by `adept_agent`
//! (`adept_agent::eval::overlap`'s offline shortlist,
//! `adept_agent::create`'s eval-case ids). Extracted here so the crates
//! can't silently drift apart on what counts as a "word", how similarity is
//! computed, or what a kebab-case rewrite produces; each caller still
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

/// Rewrite `s` into kebab-case: ASCII-lowercased alphanumerics, every other
/// character run collapsed to a single `-`, with no leading or trailing
/// hyphen.
///
/// Non-ASCII characters are dropped rather than transliterated, so the result
/// is always pure ASCII — which is what makes it safe for callers to slice on
/// byte offsets. A string with no ASCII alphanumerics at all (punctuation
/// only, or e.g. CJK text) kebab-cases to the empty string; callers decide
/// what to substitute (`SL005` suggests it verbatim, `adept_agent::create`
/// falls back to a positional id).
#[must_use]
pub fn to_kebab_case(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_dash = false;
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash && !out.is_empty() {
            out.push('-');
            prev_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kebab_case_collapses_runs_and_trims_hyphens() {
        assert_eq!(to_kebab_case("Fills PDF Forms!"), "fills-pdf-forms");
        assert_eq!(
            to_kebab_case("  __leading & trailing __ "),
            "leading-trailing"
        );
        assert_eq!(to_kebab_case("already-kebab-2"), "already-kebab-2");
    }

    #[test]
    fn kebab_case_of_text_with_no_ascii_alphanumerics_is_empty() {
        assert_eq!(to_kebab_case("!!! ??? ..."), "");
        assert_eq!(to_kebab_case("日本語"), "");
    }

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
