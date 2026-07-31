//! Fixture-based round-trip and validation tests for the eval-dataset schema
//! (`adept::evals`). See `docs/EVALS.md` for the published contract this
//! pins.

use std::path::{Path, PathBuf};

use adept::evals::{parse_jsonl, to_jsonl, validate, Assertion, EvalError, SCHEMA_VERSION};

fn fixture_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/evals")
        .join(name)
}

/// The round-trip fixture: parses line by line, re-serializes, and must be
/// byte-identical to the file on disk. Every assertion kind named in
/// `docs/EVALS.md` (`contains`, `file_exists`, `file_contains`, `command`)
/// appears at least once across its lines.
#[test]
fn roundtrip_fixture_is_byte_identical() {
    let path = fixture_path("roundtrip.jsonl");
    let original = std::fs::read_to_string(&path).expect("fixture should exist");

    let cases = parse_jsonl(&original).expect("fixture should parse");
    let reserialized = to_jsonl(&cases);
    assert_eq!(
        reserialized, original,
        "re-serializing the fixture should reproduce it byte-for-byte"
    );

    validate(&original).expect("fixture should validate");

    let mut kinds: Vec<&'static str> = cases
        .iter()
        .flat_map(|c| &c.assertions)
        .map(|a| match a {
            Assertion::Contains { .. } => "contains",
            Assertion::FileExists { .. } => "file_exists",
            Assertion::FileContains { .. } => "file_contains",
            Assertion::Command { .. } => "command",
        })
        .collect();
    kinds.sort_unstable();
    kinds.dedup();
    assert_eq!(
        kinds,
        vec!["command", "contains", "file_contains", "file_exists"],
        "fixture should exercise every assertion kind"
    );

    for case in &cases {
        assert_eq!(case.schema_version, SCHEMA_VERSION);
    }
}

#[test]
fn blank_lines_are_skipped_on_parse() {
    let text = "\n\n{\"schema_version\":1,\"prompt\":\"p\",\"assertions\":[]}\n\n";
    let cases = parse_jsonl(text).unwrap();
    assert_eq!(cases.len(), 1);
}

#[test]
fn line_number_is_reported_for_bad_json() {
    let text = "{\"schema_version\":1,\"prompt\":\"p\",\"assertions\":[]}\nnope\n";
    match parse_jsonl(text) {
        Err(EvalError::Parse { line, .. }) => assert_eq!(line, 2),
        other => panic!("expected Parse error at line 2, got {other:?}"),
    }
}
