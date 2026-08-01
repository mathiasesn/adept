//! Asserts `docs/RULES.md` documents every rule in the registry, that
//! `docs/EVALS.md` documents every eval-dataset assertion kind, and that
//! `docs/BACKLOG.md` follows its own formatting conventions (80-column
//! wrap, parenthesized `#N` issue citations), so none of these docs can
//! silently drift from the code or convention they describe.

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

/// `docs/BACKLOG.md` wraps prose at 80 columns. Counted in characters, not
/// bytes: the file uses multi-byte characters (`—`, `≤`, `²`, `×`) whose
/// UTF-8 byte length would over-count relative to what a reader (or an
/// 80-column terminal) actually sees.
#[test]
fn backlog_lines_fit_eighty_columns() {
    let docs_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/BACKLOG.md")
        .canonicalize()
        .expect("docs/BACKLOG.md should exist");
    let docs = std::fs::read_to_string(&docs_path).expect("should read docs/BACKLOG.md");

    for (idx, line) in docs.lines().enumerate() {
        let len = line.chars().count();
        assert!(
            len <= 80,
            "docs/BACKLOG.md:{} is {} characters (max 80): {:?}",
            idx + 1,
            len,
            line
        );
    }
}

/// `docs/BACKLOG.md` cites GitHub issues as `(#N)`. This only checks the
/// mechanical form — that every `#<digits>` run is immediately preceded by
/// `(` — which is all a text scan can verify. It does NOT check that the
/// referenced issue exists, is open, or is the correct issue; it rejects
/// bare `#28` or `**#7**` forms while accepting both item citations and
/// ordinary parenthesized cross-references, which are indistinguishable
/// mechanically.
#[test]
fn backlog_citations_use_canonical_form() {
    let docs_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/BACKLOG.md")
        .canonicalize()
        .expect("docs/BACKLOG.md should exist");
    let docs = std::fs::read_to_string(&docs_path).expect("should read docs/BACKLOG.md");

    for (idx, line) in docs.lines().enumerate() {
        let bytes = line.as_bytes();
        for (i, &b) in bytes.iter().enumerate() {
            if b != b'#' {
                continue;
            }
            // Only consider `#` runs followed by at least one ASCII digit;
            // other `#`s (headings, anchors) aren't issue citations.
            if !bytes.get(i + 1).is_some_and(u8::is_ascii_digit) {
                continue;
            }
            let preceded_by_paren = i > 0 && bytes[i - 1] == b'(';
            assert!(
                preceded_by_paren,
                "docs/BACKLOG.md:{} has a `#<digits>` citation not preceded by `(`: {:?}",
                idx + 1,
                line
            );
        }
    }
}
