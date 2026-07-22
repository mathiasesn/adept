//! Integration tests for parsing SKILL.md files.

use std::path::Path;

use adept::{parse_skill, AdeptError, SkillParser};

fn fixture(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
        .join("SKILL.md")
}

#[test]
fn parses_valid_skill() {
    let skill = parse_skill(fixture("valid_skill")).expect("should parse");
    assert_eq!(skill.frontmatter.name, "pdf-extractor");
    assert_eq!(skill.frontmatter.name_line, 2);
    assert!(skill.frontmatter.description.contains("Extract text"));
    assert_eq!(skill.frontmatter.description_line, 3);
    assert_eq!(skill.frontmatter.license.as_deref(), Some("MIT"));
    assert_eq!(skill.frontmatter.license_line, Some(4));
    assert!(skill.frontmatter.extra.is_empty());
    assert!(skill.body.contains("# PDF Extractor"));
    // Line 1: ---, 2: name, 3: description, 4: license, 5: ---, 6: body starts.
    assert_eq!(skill.body_line_offset, 6);
    assert!(skill.body.trim_start().starts_with("# PDF Extractor"));
}

#[test]
fn rejects_missing_frontmatter() {
    let err = parse_skill(fixture("missing_frontmatter")).unwrap_err();
    assert!(matches!(err, AdeptError::MissingFrontmatter { .. }));
}

#[test]
fn rejects_missing_required_key() {
    let err = parse_skill(fixture("missing_required_key")).unwrap_err();
    match err {
        AdeptError::MissingField { field, .. } => assert_eq!(field, "description"),
        other => panic!("expected MissingField, got {other:?}"),
    }
}

#[test]
fn preserves_extra_keys() {
    let skill = parse_skill(fixture("extra_keys")).expect("should parse");
    assert_eq!(skill.frontmatter.extra.len(), 3);
    assert!(skill.frontmatter.extra.contains_key("author"));
    assert!(skill.frontmatter.extra.contains_key("version"));
    assert!(skill.frontmatter.extra.contains_key("tags"));
    let author = &skill.frontmatter.extra["author"];
    assert_eq!(
        author.value.as_str(),
        Some("Jane Doe"),
        "extra field value should round-trip"
    );
    assert_eq!(author.line, 5);
}

#[test]
fn parses_crlf_skill() {
    let skill = parse_skill(fixture("crlf_skill")).expect("should parse CRLF file");
    assert_eq!(skill.frontmatter.name, "crlf-skill");
    assert!(skill.body.contains("Body line one."));
    assert!(skill.body.contains("Body line two."));
}

#[test]
fn parses_file_without_trailing_newline() {
    let skill =
        parse_skill(fixture("no_trailing_newline")).expect("should parse missing trailing \\n");
    assert_eq!(skill.frontmatter.name, "no-newline-skill");
    assert_eq!(skill.body, "Body without trailing newline.");
}

#[test]
fn rejects_unterminated_frontmatter() {
    let parser = adept::AnthropicSkillParser;
    let source = "---\nname: x\ndescription: y\n";
    let err = parser.parse_str(Path::new("SKILL.md"), source).unwrap_err();
    assert!(matches!(err, AdeptError::UnterminatedFrontmatter { .. }));
}

#[test]
fn rejects_non_mapping_frontmatter() {
    let parser = adept::AnthropicSkillParser;
    let source = "---\n- just\n- a\n- list\n---\nbody\n";
    let err = parser.parse_str(Path::new("SKILL.md"), source).unwrap_err();
    assert!(matches!(err, AdeptError::FrontmatterNotMapping { .. }));
}

#[test]
fn nonexistent_path_is_reported() {
    let err = parse_skill("tests/fixtures/does_not_exist/SKILL.md").unwrap_err();
    assert!(matches!(err, AdeptError::Io { .. }));
}
