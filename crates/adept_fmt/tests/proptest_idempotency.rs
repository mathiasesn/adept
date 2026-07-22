//! A property test asserting idempotency (`fmt(fmt(x)) == fmt(x)`) over
//! generated Markdown bodies.
//!
//! Fully arbitrary Markdown is prone to generating inputs whose meaning is
//! ambiguous or ill-defined (e.g. pathological emphasis-delimiter runs),
//! which made a fully-unconstrained generator too flaky to be a useful
//! signal. Instead this generates bodies from a constrained grammar of
//! headings, paragraphs, and list items built from safe word tokens, which
//! stays representative while remaining stable; the fixture corpus in
//! `format_tests.rs` covers the constructs (tables, code fences, HTML,
//! links, etc.) that are riskier to generate arbitrarily.

use adept_fmt::{format_str, FmtConfig};
use proptest::prelude::*;

fn word_strategy() -> impl Strategy<Value = String> {
    prop::sample::select(vec![
        "hello",
        "world",
        "adept",
        "skill",
        "formatter",
        "prose",
        "reflow",
        "line",
        "width",
        "test",
        "example",
        "markdown",
        "body",
        "content",
    ])
    .prop_map(String::from)
}

fn paragraph_strategy() -> impl Strategy<Value = String> {
    prop::collection::vec(word_strategy(), 1..20).prop_map(|words| words.join(" "))
}

fn block_strategy() -> impl Strategy<Value = String> {
    prop_oneof![
        paragraph_strategy(),
        (1u8..=6, paragraph_strategy()).prop_map(|(level, text)| format!(
            "{} {}",
            "#".repeat(level as usize),
            text
        )),
        prop::collection::vec(paragraph_strategy(), 1..6).prop_map(|items| items
            .iter()
            .map(|i| format!("- {i}"))
            .collect::<Vec<_>>()
            .join("\n")),
    ]
}

fn document_strategy() -> impl Strategy<Value = String> {
    prop::collection::vec(block_strategy(), 1..8).prop_map(|blocks| blocks.join("\n\n"))
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    #[test]
    fn format_is_idempotent(body in document_strategy()) {
        let source = format!(
            "---\nname: prop-fixture\ndescription: property-generated body for idempotency testing.\n---\n{body}\n"
        );
        let cfg = FmtConfig::default();
        let once = format_str(&source, &cfg).expect("generated document should format");
        let twice = format_str(&once, &cfg).expect("formatted output should re-format");
        prop_assert_eq!(once, twice);
    }
}
