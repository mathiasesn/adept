//! Atomic single-file write, and a transactional multi-file write built on
//! top of it: either every file in a batch lands, or none of them do.

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};

/// The shared temp-file suffix used by both [`write_atomically`] and
/// [`write_all_transactionally`], so there is exactly one convention for
/// what an in-progress `adept` write looks like on disk.
const TMP_SUFFIX: &str = "adept-tmp";

/// Write `contents` to `path` atomically: write to a sibling temp file
/// (`.{filename}.adept-tmp`, in the same directory as `path`), `write_all`
/// then `sync_all`, then rename over `path`. Never leaves `path` clobbered
/// if the write fails partway through.
///
/// # Errors
/// Returns the first I/O error encountered creating, writing, syncing, or
/// renaming the temp file.
pub fn write_atomically(path: &Path, contents: &str) -> std::io::Result<()> {
    let tmp_path = tmp_path_for(path);
    {
        let mut tmp_file = std::fs::File::create(&tmp_path)?;
        tmp_file.write_all(contents.as_bytes())?;
        tmp_file.sync_all()?;
    }
    std::fs::rename(&tmp_path, path)
}

/// The sibling temp path a write to `path` stages through.
fn tmp_path_for(path: &Path) -> PathBuf {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    dir.join(format!(
        ".{}.{TMP_SUFFIX}",
        path.file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "SKILL.md".to_string())
    ))
}

/// Write every `(path, contents)` pair in `files`, transactionally:
///
/// 1. For each path whose parent directory does not exist yet, create it
///    (recursively, like `mkdir -p`), remembering the highest ancestor this
///    call itself created so a later rollback can remove exactly what it
///    added and nothing that pre-existed.
/// 2. Write each to its sibling temp file via the same staging step
///    [`write_atomically`] uses (create, `write_all`, `sync_all`), but
///    without yet renaming.
/// 3. If any directory creation or temp write fails, unlink every temp file
///    already created (best effort), remove every directory this call
///    created (best effort, deepest first), and return the error — no
///    original file is touched and no directory is left behind that wasn't
///    there before the call.
/// 4. Only once every temp write has succeeded, rename each temp file into
///    place over its target.
///
/// This is the writer's own precondition to satisfy, not the caller's: a
/// batch cannot be staged into a directory that doesn't exist, so creating
/// it is part of "staging", not a step a caller should have to perform
/// before calling this function.
///
/// # Atomicity caveat
/// Step 4 is atomic *per file* (each `rename` is atomic on the same
/// filesystem), but not atomic *across* files: a crash or power loss
/// between the first and last rename can leave the batch partially applied.
/// This is a known, accepted limitation (there is no cross-file transaction
/// log here) — the mitigation is that all fallible work (steps 1-3) happens
/// before any rename, so the only failure mode left is an external crash
/// during a short, all-renames window, not a normal I/O error partially
/// applying the batch. The same caveat extends to directories created in
/// step 1: they are not retroactively removed once any rename has occurred.
///
/// # Errors
/// Returns the first I/O error encountered while creating a directory,
/// creating/writing/syncing a temp file. Errors during rename (step 4) are
/// also propagated, though by that point some files in the batch may already
/// have been renamed.
pub fn write_all_transactionally(files: &BTreeMap<PathBuf, String>) -> std::io::Result<()> {
    let mut created_dirs: Vec<PathBuf> = Vec::new();
    for path in files.keys() {
        if let Some(parent) = path.parent() {
            if let Err(err) = create_dir_all_tracked(parent, &mut created_dirs) {
                for dir in created_dirs.iter().rev() {
                    let _ = std::fs::remove_dir(dir);
                }
                return Err(err);
            }
        }
    }

    let mut tmp_paths: Vec<(PathBuf, PathBuf)> = Vec::with_capacity(files.len());

    for (path, contents) in files {
        let tmp_path = tmp_path_for(path);

        let write_result = (|| -> std::io::Result<()> {
            let mut tmp_file = std::fs::File::create(&tmp_path)?;
            tmp_file.write_all(contents.as_bytes())?;
            tmp_file.sync_all()
        })();

        if let Err(err) = write_result {
            for (_, tmp) in &tmp_paths {
                let _ = std::fs::remove_file(tmp);
            }
            let _ = std::fs::remove_file(&tmp_path);
            for dir in created_dirs.iter().rev() {
                let _ = std::fs::remove_dir(dir);
            }
            return Err(err);
        }

        tmp_paths.push((path.clone(), tmp_path));
    }

    for (path, tmp_path) in &tmp_paths {
        std::fs::rename(tmp_path, path)?;
    }

    Ok(())
}

/// Like `std::fs::create_dir_all(dir)`, but records every ancestor directory
/// this call itself creates (deepest first is not required; callers remove
/// in reverse order) into `created`, so a caller can roll back exactly what
/// was added on failure without touching directories that already existed.
/// Does nothing (and records nothing) if `dir` already exists.
fn create_dir_all_tracked(dir: &Path, created: &mut Vec<PathBuf>) -> std::io::Result<()> {
    if dir.is_dir() {
        return Ok(());
    }
    // Find the deepest already-existing ancestor, then create every missing
    // component from there down, recording each one.
    let mut to_create: Vec<&Path> = Vec::new();
    let mut cursor = Some(dir);
    while let Some(p) = cursor {
        if p.is_dir() || p.as_os_str().is_empty() {
            break;
        }
        to_create.push(p);
        cursor = p.parent();
    }
    for p in to_create.into_iter().rev() {
        std::fs::create_dir(p)?;
        created.push(p.to_path_buf());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_all_files_on_success() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let a = dir.join("SKILL.md");
        let b = dir.join("REFERENCE.md");
        let files = BTreeMap::from([
            (a.clone(), "new skill content".to_string()),
            (b.clone(), "new reference content".to_string()),
        ]);

        write_all_transactionally(&files).unwrap();

        assert_eq!(std::fs::read_to_string(&a).unwrap(), "new skill content");
        assert_eq!(
            std::fs::read_to_string(&b).unwrap(),
            "new reference content"
        );
        // No leftover temp files.
        let leftovers: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().contains("adept-tmp"))
            .collect();
        assert!(leftovers.is_empty());
    }

    #[test]
    fn failure_leaves_originals_untouched_and_no_temp_files() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let a = dir.join("SKILL.md");
        std::fs::write(&a, "original content").unwrap();

        // A target whose parent path is occupied by a regular file, not a
        // directory: `write_all_transactionally` now creates missing parent
        // directories itself, but it still cannot create one where a file
        // already sits, so this still simulates a failure partway through
        // the batch.
        let blocker = dir.join("blocker");
        std::fs::write(&blocker, "not a directory").unwrap();
        let bad_path = blocker.join("REFERENCE.md");

        let files = BTreeMap::from([
            (a.clone(), "new skill content".to_string()),
            (bad_path.clone(), "new reference content".to_string()),
        ]);

        let result = write_all_transactionally(&files);
        assert!(result.is_err());

        // Original SKILL.md is untouched.
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "original content");
        // No leftover temp files anywhere in the directory tree.
        let leftovers: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().contains("adept-tmp"))
            .collect();
        assert!(leftovers.is_empty());
    }

    #[test]
    fn creates_missing_parent_directories() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let nested = dir.join("new-skill").join("evals").join("evals.jsonl");
        let files = BTreeMap::from([(nested.clone(), "{}".to_string())]);

        write_all_transactionally(&files).unwrap();

        assert_eq!(std::fs::read_to_string(&nested).unwrap(), "{}");
    }

    #[test]
    fn rolls_back_directories_it_created_on_failure() {
        // One brand-new directory that would succeed to create, alongside a
        // second path whose parent creation fails (a file sits where a
        // directory needs to go). The whole batch fails, so the first
        // directory this call created must be rolled back rather than left
        // behind.
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        // Named so `files.keys()` (a `BTreeMap`, sorted) processes this
        // directory's creation before the failing one below.
        let new_dir = dir.join("aaa-brand-new-dir");
        let ok_path = new_dir.join("SKILL.md");
        let blocker = dir.join("zzz-blocker");
        std::fs::write(&blocker, "not a directory").unwrap();
        let bad_path = blocker.join("REFERENCE.md");

        let files = BTreeMap::from([
            (ok_path.clone(), "content".to_string()),
            (bad_path.clone(), "content".to_string()),
        ]);
        let result = write_all_transactionally(&files);
        assert!(result.is_err());
        assert!(
            !new_dir.exists(),
            "directory created for this failed batch must be rolled back"
        );
    }
}
