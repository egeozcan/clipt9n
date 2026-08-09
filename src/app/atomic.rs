//! Atomic text-file replacement, shared by the editors that write
//! their own files (glossary, templates).
//!
//! Lives here rather than in either editor because both need the same
//! guarantee — an interrupted save must not leave a truncated file
//! behind — and duplicating a temp-file dance is how the two copies
//! drift.

use std::path::Path;

use crate::error::TranslateError;
use crate::platform::Platform;

/// Write `contents` to `path` via a same-directory temp file and an
/// atomic replace. `wrap` builds the error variant that fits the
/// caller's domain, so a failed glossary save reads as a glossary error
/// and a failed template save as a template error.
///
/// Mirrors `DiskAtomicConfig::replace` minus its previous-contents
/// rollback: that exists to protect a config the app is mid-way through
/// adopting, whereas here the bytes are already final by the time the
/// rename runs.
pub(super) fn write_text_atomically(
    path: &Path,
    contents: &str,
    wrap: fn(String) -> TranslateError,
) -> Result<(), TranslateError> {
    use std::io::Write;

    let parent = path
        .parent()
        .ok_or_else(|| wrap(format!("{} has no parent directory", path.display())))?;
    std::fs::create_dir_all(parent)
        .map_err(|e| wrap(format!("creating {}: {e}", parent.display())))?;

    let temp_path = parent.join(format!(
        ".{}.{}.tmp",
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("clipt9n-write"),
        std::process::id()
    ));
    let staged = (|| {
        let mut temp = std::fs::File::create(&temp_path)
            .map_err(|e| wrap(format!("creating {}: {e}", temp_path.display())))?;
        temp.write_all(contents.as_bytes())
            .map_err(|e| wrap(format!("writing {}: {e}", temp_path.display())))?;
        temp.flush()
            .map_err(|e| wrap(format!("flushing {}: {e}", temp_path.display())))?;
        temp.sync_all()
            .map_err(|e| wrap(format!("syncing {}: {e}", temp_path.display())))?;
        crate::platform::current()
            .replace_file(&temp_path, path)
            .map_err(|e| {
                wrap(format!(
                    "replacing {} with {}: {e}",
                    path.display(),
                    temp_path.display()
                ))
            })
    })();
    if staged.is_err() {
        let _ = std::fs::remove_file(&temp_path);
    }
    staged
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wrap(m: String) -> TranslateError {
        TranslateError::Internal(m)
    }

    #[test]
    fn atomic_write_creates_the_file_and_leaves_no_temp_behind() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.txt");
        write_text_atomically(&path, "hello", wrap).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "hello");
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "temp file should be renamed away");
    }

    #[test]
    fn atomic_write_replaces_existing_contents() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.txt");
        std::fs::write(&path, "old").unwrap();
        write_text_atomically(&path, "new", wrap).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new");
    }

    #[test]
    fn atomic_write_creates_missing_parent_directories() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/deeper/out.txt");
        write_text_atomically(&path, "hi", wrap).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "hi");
    }

    #[test]
    fn the_wrap_fn_decides_the_error_variant() {
        let path = std::path::Path::new("/");
        let err = write_text_atomically(path, "x", TranslateError::Template).unwrap_err();
        assert!(
            matches!(err, TranslateError::Template(_)),
            "expected a template error, got {err:?}"
        );
    }
}
