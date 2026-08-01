//! Asserts `release_plz.toml` cannot silently drift out of sync with the
//! root `Cargo.toml`'s `[workspace] members`: every member's package name
//! must appear exactly once as a `[[package]]` entry in `release_plz.toml`,
//! carrying `version_group = "adept"`, and `release_plz.toml` must not name
//! anything that isn't a real member.
//!
//! release-plz has no workspace-level default for `version_group`, so a
//! member added to `Cargo.toml` and forgotten in `release_plz.toml` would
//! otherwise silently start versioning on its own. Package names are not
//! derivable from directory names (`crates/adept` -> package `adept-core`,
//! `crates/adept_cli` -> package `adept`), so this reads each member's own
//! `Cargo.toml` for its `[package] name` rather than guessing from the path.

use std::path::{Path, PathBuf};

/// The workspace root, resolved relative to this crate so the test does not
/// depend on the working directory it is invoked from.
fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap_or_else(|e| panic!("workspace root should exist: {e}"))
}

fn read_toml(path: &Path) -> toml::Value {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("should read {}: {e}", path.display()));
    text.parse::<toml::Value>()
        .unwrap_or_else(|e| panic!("should parse {} as TOML: {e}", path.display()))
}

/// The real package name for each workspace member, read from that member's
/// own `Cargo.toml`, in the order `members` lists them.
fn member_package_names(root: &Path) -> Vec<String> {
    let root_manifest = read_toml(&root.join("Cargo.toml"));
    let members = root_manifest
        .get("workspace")
        .and_then(|w| w.get("members"))
        .and_then(|m| m.as_array())
        .unwrap_or_else(|| panic!("root Cargo.toml should have [workspace] members"));

    members
        .iter()
        .map(|m| {
            let dir = m
                .as_str()
                .unwrap_or_else(|| panic!("workspace member entry should be a string: {m:?}"));
            let manifest = read_toml(&root.join(dir).join("Cargo.toml"));
            manifest
                .get("package")
                .and_then(|p| p.get("name"))
                .and_then(|n| n.as_str())
                .unwrap_or_else(|| panic!("{dir}/Cargo.toml should have [package] name"))
                .to_string()
        })
        .collect()
}

/// The `[[package]]` entries in `release_plz.toml`, as `(name, version_group)`
/// pairs, in file order.
fn release_plz_entries(root: &Path) -> Vec<(Option<String>, Option<String>)> {
    let doc = read_toml(&root.join("release_plz.toml"));
    let packages = doc
        .get("package")
        .and_then(|p| p.as_array())
        .unwrap_or_else(|| panic!("release_plz.toml should have [[package]] entries"));

    packages
        .iter()
        .map(|entry| {
            let name = entry
                .get("name")
                .and_then(|n| n.as_str())
                .map(str::to_string);
            let version_group = entry
                .get("version_group")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            (name, version_group)
        })
        .collect()
}

#[test]
fn release_plz_toml_matches_workspace_members() {
    let root = workspace_root();
    let member_names = member_package_names(&root);
    let entries = release_plz_entries(&root);

    let mut errors: Vec<String> = Vec::new();

    // Duplicate `[[package]]` entries for the same name.
    let mut seen: Vec<&str> = Vec::new();
    for (name, _) in &entries {
        let Some(name) = name.as_deref() else {
            continue;
        };
        if seen.contains(&name) {
            errors.push(format!(
                "release_plz.toml has more than one [[package]] entry named `{name}`"
            ));
        } else {
            seen.push(name);
        }
    }

    // Entries missing a `name` key entirely.
    for (idx, (name, _)) in entries.iter().enumerate() {
        if name.is_none() {
            errors.push(format!(
                "release_plz.toml [[package]] entry #{idx} has no `name` key"
            ));
        }
    }

    let entry_names: Vec<&str> = entries.iter().filter_map(|(n, _)| n.as_deref()).collect();

    // Every workspace member must appear exactly once.
    for member in &member_names {
        let count = entry_names.iter().filter(|n| **n == member).count();
        if count == 0 {
            errors.push(format!(
                "release_plz.toml is missing a [[package]] entry for workspace member \
                 `{member}` — add one with version_group = \"adept\""
            ));
        }
    }

    // Every release_plz.toml entry must name a real member (no stale/extra
    // entries pointing at packages that no longer exist).
    for name in &entry_names {
        if !member_names.iter().any(|m| m == name) {
            errors.push(format!(
                "release_plz.toml has a [[package]] entry named `{name}`, which is not a \
                 workspace member in Cargo.toml — remove it or fix the name"
            ));
        }
    }

    // Every entry must carry version_group = "adept".
    for (name, version_group) in &entries {
        let name = name.as_deref().unwrap_or("<unnamed>");
        match version_group.as_deref() {
            Some("adept") => {}
            Some(other) => errors.push(format!(
                "release_plz.toml entry `{name}` has version_group = \"{other}\", expected \
                 \"adept\" so all crates version in lockstep"
            )),
            None => errors.push(format!(
                "release_plz.toml entry `{name}` is missing version_group = \"adept\""
            )),
        }
    }

    assert!(
        errors.is_empty(),
        "release_plz.toml is out of sync with Cargo.toml's workspace members:\n{}",
        errors.join("\n")
    );
}
