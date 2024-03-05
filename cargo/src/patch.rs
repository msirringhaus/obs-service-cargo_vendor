use crate::errors::OBSCargoError;
use crate::errors::OBSCargoErrorKind;
use patch::{Line, Patch};
use std::path::Path;
use std::path::PathBuf;

#[allow(unused_imports)]
use tracing::{debug, error, info, trace, warn, Level};

// No fuzzy apply for now.
fn apply_patch_to_string(diff: &Patch, old: &str) -> Result<String, OBSCargoError> {
    let old_lines = old.lines().collect::<Vec<&str>>();
    let mut out: Vec<&str> = vec![];
    let mut old_line = 0usize;
    for hunk in &diff.hunks {
        // First add all non-affected lines in front of this hunk to the new file
        while old_line < hunk.old_range.start as usize - 1 {
            out.push(old_lines[old_line]);
            old_line += 1;
        }
        // Then deal with the hunk
        for line in &hunk.lines {
            match line {
                Line::Context(s) => {
                    // Verify the context line is correct
                    if old_lines[old_line] != *s {
                        let err_str = format!(
                            "Failed to apply hunk:\n{}.\n\nContext mismatch in line {}: '{}' vs. '{}'",
                            hunk, old_line, old_lines[old_line], s
                        );
                        return Err(OBSCargoError::new(OBSCargoErrorKind::PatchError, err_str));
                    }
                    out.push(s);
                    old_line += 1;
                }
                Line::Add(s) => out.push(s),
                Line::Remove(s) => {
                    // Verify the line to be removed is correct
                    if old_lines[old_line] != *s {
                        let err_str = format!(
                            "Failed to apply hunk:\n{}.\n\nLine to be removed not found at {}: '{}' vs. '{}'",
                            hunk, old_line, old_lines[old_line], s
                        );
                        return Err(OBSCargoError::new(OBSCargoErrorKind::PatchError, err_str));
                    }
                    old_line += 1;
                }
            }
        }
    }
    Ok(out.join("\n"))
}

fn make_patch_path_absolute(prjdir: impl AsRef<Path>, patch: impl AsRef<Path>) -> PathBuf {
    // hardcoding `-p1` for now, since that is the most common
    let path: &Path = patch.as_ref();
    // "root" is an separate path-section, so if we are absolute
    // (which we should pretty much always be), then we have to
    // skip the root and the actual first path.
    let to_skip = if path.is_absolute() { 2 } else { 1 };
    let stripped: PathBuf = path.iter().skip(to_skip).collect();
    prjdir.as_ref().join(stripped)
}

pub fn apply_patch(prjdir: impl AsRef<Path>, patch: impl AsRef<Path>) -> Result<(), OBSCargoError> {
    // Read the patch to memory
    let absolute_patch_path = prjdir.as_ref().join(patch.as_ref());
    let patch_str = std::fs::read_to_string(absolute_patch_path).map_err(|err| {
        error!(?err, "Failed to access patch");
        OBSCargoError::new(
            OBSCargoErrorKind::PatchError,
            "failed to access patch".to_string(),
        )
    })?;
    // Parse the patches
    let patches = Patch::from_multiple(&patch_str).map_err(|err| {
        error!(?err, "Failed to parse patch");
        OBSCargoError::new(
            OBSCargoErrorKind::PatchError,
            "failed to parse patch".to_string(),
        )
    })?;

    // Start applying patches
    for p in &patches {
        let absolute_old_path = make_patch_path_absolute(&prjdir, p.old.path.as_ref());
        // Read in the old file to memory
        let old = std::fs::read_to_string(&absolute_old_path).map_err(|err| {
            error!(
                ?err,
                "Failed to read previous version of patched file: {}",
                &absolute_old_path.to_string_lossy()
            );
            OBSCargoError::new(
                OBSCargoErrorKind::PatchError,
                "failed to read previous version of patched file".to_string(),
            )
        })?;
        // Apply the patch to the string we now have in memory
        let new = apply_patch_to_string(p, &old)?;
        // Write the newly patched String back to the new destination
        let absolute_new_path = make_patch_path_absolute(&prjdir, p.new.path.as_ref());
        std::fs::write(absolute_new_path, new).map_err(|err| {
            error!(?err, "Failed to write new, patched version of file");
            OBSCargoError::new(
                OBSCargoErrorKind::PatchError,
                "failed to write new, patched version of file".to_string(),
            )
        })?;
    }
    Ok(())
}
