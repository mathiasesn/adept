//! Integration tests for `SkillSet` directory discovery.

use std::path::Path;

use adept::SkillSet;

fn fixture(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

#[test]
fn discovers_single_skill_file() {
    let set = SkillSet::discover(fixture("valid_skill").join("SKILL.md")).expect("discover");
    assert_eq!(set.skills.len(), 1);
    assert_eq!(set.skills[0].frontmatter.name, "pdf-extractor");
    assert!(set.errors.is_empty());
}

#[test]
fn discovers_single_skill_directory() {
    let set = SkillSet::discover(fixture("valid_skill")).expect("discover");
    assert_eq!(set.skills.len(), 1);
}

#[test]
fn discovers_tree_skipping_hidden_and_excluded_dirs() {
    let set = SkillSet::discover(fixture("tree")).expect("discover");
    let names: Vec<&str> = set
        .skills
        .iter()
        .map(|s| s.frontmatter.name.as_str())
        .collect();
    assert_eq!(set.errors.len(), 0);
    assert_eq!(
        names.len(),
        2,
        "expected exactly skill-a and skill-b: {names:?}"
    );
    assert!(names.contains(&"skill-a"));
    assert!(names.contains(&"skill-b"));
    assert!(!names.contains(&"hidden-skill"));
    assert!(!names.contains(&"target-skill"));
}

#[test]
fn nonexistent_root_errors() {
    let err = SkillSet::discover(fixture("does_not_exist")).unwrap_err();
    assert!(matches!(err, adept::AdeptError::NotFound(_)));
}
