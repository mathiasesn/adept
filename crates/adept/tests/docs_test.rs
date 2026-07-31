//! Asserts `docs/RULES.md` documents every rule in the registry, and that
//! `docs/EVALS.md` documents every eval-dataset assertion kind, so neither
//! doc can silently drift from the code it describes.

use std::path::Path;

use adept::Registry;

#[test]
fn every_registered_rule_is_documented() {
    let docs_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/RULES.md")
        .canonicalize()
        .expect("docs/RULES.md should exist");
    let docs = std::fs::read_to_string(&docs_path).expect("should read docs/RULES.md");

    let registry = Registry::new();
    for meta in registry.all_meta() {
        let code_heading = format!("### {}", meta.code);
        assert!(
            docs.contains(&code_heading),
            "docs/RULES.md is missing an entry for {} ({})",
            meta.code,
            meta.name
        );
        assert!(
            docs.contains(meta.name),
            "docs/RULES.md does not mention rule name `{}` for {}",
            meta.name,
            meta.code
        );
    }
}

/// The eval-dataset assertion vocabulary, as adept's code defines it
/// (`adept::evals::Assertion`'s `kind` values). Kept as a literal list here
/// (rather than derived via reflection, which serde does not expose) so
/// this test has to be hand-updated whenever the enum gains or loses a
/// variant — the same manual-sync tripwire `every_registered_rule_is_documented`
/// gets for free from the registry.
const ASSERTION_KINDS: &[&str] = &["contains", "file_exists", "file_contains", "command"];

#[test]
fn every_assertion_kind_is_documented_in_evals_md() {
    let docs_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/EVALS.md")
        .canonicalize()
        .expect("docs/EVALS.md should exist");
    let docs = std::fs::read_to_string(&docs_path).expect("should read docs/EVALS.md");

    for kind in ASSERTION_KINDS {
        let heading = format!("### `{kind}`");
        assert!(
            docs.contains(&heading),
            "docs/EVALS.md is missing a `{heading}` section for assertion kind `{kind}`"
        );
    }

    // And the reverse: every `### `kind`` heading actually present in the
    // doc corresponds to a real assertion kind, so a doc-only kind (one
    // that would deserialize as "unknown variant") can't hide undetected.
    // This parses the doc rather than repeating a hardcoded literal, so a
    // kind documented but removed from the code (or vice versa) is caught
    // by an actual disagreement between the two sources, not by two copies
    // of the same list agreeing with themselves.
    let documented_kinds: Vec<&str> = docs
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            let rest = line.strip_prefix("### `")?;
            rest.strip_suffix('`')
        })
        .collect();
    assert!(
        !documented_kinds.is_empty(),
        "found no `### `kind`` headings in docs/EVALS.md; the parser above may be broken"
    );
    for heading_kind in &documented_kinds {
        assert!(
            ASSERTION_KINDS.contains(heading_kind),
            "docs/EVALS.md documents `{heading_kind}`, which is not in ASSERTION_KINDS"
        );
    }
    for kind in ASSERTION_KINDS {
        assert!(
            documented_kinds.contains(kind),
            "ASSERTION_KINDS lists `{kind}`, but docs/EVALS.md has no `### `{kind}`` heading for it"
        );
    }

    // Prove the round-trip: every documented kind must actually deserialize
    // as a valid `Assertion` with a minimal plausible payload, so the doc
    // and the real serde tag values can't drift on spelling either.
    let samples = [
        (r#"{"kind":"contains","value":"x"}"#, "contains"),
        (r#"{"kind":"file_exists","path":"x"}"#, "file_exists"),
        (
            r#"{"kind":"file_contains","path":"x","value":"y"}"#,
            "file_contains",
        ),
        (r#"{"kind":"command","command":"true"}"#, "command"),
    ];
    for (json, kind) in samples {
        assert!(
            ASSERTION_KINDS.contains(&kind),
            "sample kind `{kind}` missing from ASSERTION_KINDS"
        );
        serde_json::from_str::<adept::evals::Assertion>(json)
            .unwrap_or_else(|e| panic!("sample for `{kind}` should deserialize: {e}"));
    }
}
