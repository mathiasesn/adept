//! Asserts `release-plz.toml` cannot silently drift out of sync with the
//! root `Cargo.toml`'s `[workspace] members`: every member's package name
//! must appear exactly once as a `[[package]]` entry in `release-plz.toml`,
//! carrying the shared `version_group`, and `release-plz.toml` must not name
//! anything that isn't a real member.
//!
//! release-plz has no workspace-level default for `version_group`, so a
//! member added to `Cargo.toml` and forgotten in `release-plz.toml` would
//! otherwise silently start versioning on its own. Package names are not
//! derivable from directory names (`crates/adept` -> package `adept-core`,
//! `crates/adept_cli` -> package `adept`), so this reads each member's own
//! `Cargo.toml` for its `[package] name` rather than guessing from the path.
//!
//! The hyphen in the filename is load-bearing: release-plz only ever looks for
//! `release-plz.toml` or `.release-plz.toml`, so an underscored copy parses
//! fine here while release-plz runs with its defaults and versions the crates
//! apart.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// The one group every crate must share, so all four version in lockstep.
const VERSION_GROUP: &str = "adept";

#[derive(Deserialize)]
struct RootManifest {
    workspace: Workspace,
}

#[derive(Deserialize)]
struct Workspace {
    members: Vec<String>,
}

#[derive(Deserialize)]
struct MemberManifest {
    package: MemberPackage,
}

#[derive(Deserialize)]
struct MemberPackage {
    name: String,
}

#[derive(Deserialize)]
struct ReleasePlzConfig {
    /// `name` and `version_group` are optional here so a malformed entry is
    /// reported alongside every other problem instead of aborting the parse.
    #[serde(default)]
    package: Vec<ReleasePlzPackage>,
}

#[derive(Deserialize)]
struct ReleasePlzPackage {
    name: Option<String>,
    version_group: Option<String>,
}

/// The workspace root, resolved relative to this crate so the test does not
/// depend on the working directory it is invoked from.
fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap_or_else(|e| panic!("workspace root should exist: {e}"))
}

fn read_toml<T: serde::de::DeserializeOwned>(path: &Path) -> T {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("should read {}: {e}", path.display()));
    toml::from_str(&text).unwrap_or_else(|e| panic!("should parse {}: {e}", path.display()))
}

/// The real package name for each workspace member, read from that member's
/// own `Cargo.toml`, in the order `members` lists them.
fn member_package_names(root: &Path) -> Vec<String> {
    let root_manifest: RootManifest = read_toml(&root.join("Cargo.toml"));
    root_manifest
        .workspace
        .members
        .iter()
        .map(|dir| {
            let manifest: MemberManifest = read_toml(&root.join(dir).join("Cargo.toml"));
            manifest.package.name
        })
        .collect()
}

/// `dry_run: false` is a silent no-op, so it must never appear in the release
/// workflow. `release-plz/action` guards the flag on string emptiness rather
/// than truthiness (`if [[ -n "${{ inputs.dry_run }}" ]]`, `action.yml:115-119`)
/// and the input has no `default:` (`action.yml:40-47`), so `false` renders as
/// the non-empty string `"false"` and `--dry-run` is still passed. Going live
/// means deleting the line.
///
/// The failure mode is why this is mechanical rather than a comment: the "go
/// live" commit merges, the job goes green, nothing publishes, and there is no
/// signal until someone notices crates.io is empty. A substring check over the
/// non-comment lines is sufficient — the only spellings that matter are ones a
/// human would type, and the comments deliberately quote the bad value while
/// explaining why it is bad.
#[test]
fn release_workflow_never_disables_dry_run_by_value() {
    let path = workspace_root().join(".github/workflows/release.yml");
    let workflow = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("should read {}: {e}", path.display()));

    let offender = workflow
        .lines()
        .enumerate()
        .find(|(_, line)| !line.trim_start().starts_with('#') && line.contains("dry_run: false"));

    if let Some((idx, line)) = offender {
        panic!(
            "{}:{} sets `{}`, which does NOT disable the dry run — the action \
             tests the input for emptiness, so `--dry-run` is still passed and \
             the release silently publishes nothing. Delete the `dry_run` line \
             instead.",
            path.display(),
            idx + 1,
            line.trim()
        );
    }
}

#[test]
fn release_plz_toml_matches_workspace_members() {
    let root = workspace_root();
    let member_names = member_package_names(&root);
    let config: ReleasePlzConfig = read_toml(&root.join("release-plz.toml"));

    let mut errors: Vec<String> = Vec::new();

    // Per-entry checks: every entry needs a `name`, and every entry needs the
    // shared version_group.
    for (idx, entry) in config.package.iter().enumerate() {
        let label = match entry.name.as_deref() {
            Some(name) => format!("`{name}`"),
            None => {
                errors.push(format!(
                    "release-plz.toml [[package]] entry #{idx} has no `name` key"
                ));
                format!("#{idx}")
            }
        };
        match entry.version_group.as_deref() {
            Some(group) if group == VERSION_GROUP => {}
            Some(other) => errors.push(format!(
                "release-plz.toml entry {label} has version_group = \"{other}\", expected \
                 \"{VERSION_GROUP}\" so all crates version in lockstep"
            )),
            None => errors.push(format!(
                "release-plz.toml entry {label} is missing version_group = \"{VERSION_GROUP}\""
            )),
        }
    }

    // How many entries name each package — the one fact the bijection checks
    // below both read from.
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for name in config.package.iter().filter_map(|e| e.name.as_deref()) {
        *counts.entry(name).or_default() += 1;
    }

    for (&name, &count) in &counts {
        if count > 1 {
            errors.push(format!(
                "release-plz.toml has {count} [[package]] entries named `{name}`, expected one"
            ));
        }
        // No stale entries pointing at packages that no longer exist.
        if !member_names.iter().any(|m| m == name) {
            errors.push(format!(
                "release-plz.toml has a [[package]] entry named `{name}`, which is not a \
                 workspace member in Cargo.toml — remove it or fix the name"
            ));
        }
    }

    // Every workspace member must appear.
    for member in &member_names {
        if !counts.contains_key(member.as_str()) {
            errors.push(format!(
                "release-plz.toml is missing a [[package]] entry for workspace member \
                 `{member}` — add one with version_group = \"{VERSION_GROUP}\""
            ));
        }
    }

    assert!(
        errors.is_empty(),
        "release-plz.toml is out of sync with Cargo.toml's workspace members:\n{}",
        errors.join("\n")
    );
}
