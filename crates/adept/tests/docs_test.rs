//! Asserts `docs/rules.md` documents every rule in the registry, so the two
//! can't silently drift apart.

use std::path::Path;

use adept::Registry;

#[test]
fn every_registered_rule_is_documented() {
    let docs_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/rules.md")
        .canonicalize()
        .expect("docs/rules.md should exist");
    let docs = std::fs::read_to_string(&docs_path).expect("should read docs/rules.md");

    let registry = Registry::new();
    for meta in registry.all_meta() {
        let code_heading = format!("### {}", meta.code);
        assert!(
            docs.contains(&code_heading),
            "docs/rules.md is missing an entry for {} ({})",
            meta.code,
            meta.name
        );
        assert!(
            docs.contains(meta.name),
            "docs/rules.md does not mention rule name `{}` for {}",
            meta.name,
            meta.code
        );
    }
}
