//! Unix (Linux + macOS) signal handling. Currently exposes a SIGHUP
//! listener that forwards reload requests to a sync channel. Lives in
//! `platform/` per the cross-platform discipline rule (no `cfg(unix)` in
//! `app.rs` or anywhere else).

use std::fs::{File, OpenOptions};
use std::io::{Error, ErrorKind, Write};
use std::os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::Path;

use crossbeam_channel::Sender;
use tokio::runtime::Runtime;

/// Install a SIGHUP handler against `rt`'s reactor and spawn a task that
/// forwards each delivery to `tx`. Caller drains `tx`'s receiver in its
/// event loop and triggers whatever reload it owns.
///
/// **Why register synchronously:** `tokio::signal::unix::signal(...)` is
/// what installs the OS-level `sigaction` handler — until that call
/// returns, the kernel still uses the default disposition, which for
/// SIGHUP is "terminate the process." If we delayed the call by putting
/// it inside the spawned future, an early `kill -HUP` (e.g. immediately
/// after launch) would race the worker thread and kill us before the
/// handler took effect. Registering synchronously while holding a
/// runtime-context guard closes that window: by the time `install`
/// returns, SIGHUP is intercepted.
///
/// `wake` runs after each successful send. The caller's event loop may be
/// asleep — a bare channel send would sit unread until something unrelated
/// woke it — so `wake` is what makes the reload land promptly. Kept as a
/// callback rather than an `egui::Context` so this module stays UI-agnostic.
///
/// The task runs until the runtime is dropped; if `tx` is dropped, sends
/// fail silently (logged at debug) and the task exits.
pub(crate) fn install(rt: &Runtime, tx: Sender<()>, wake: impl Fn() + Send + 'static) {
    let _enter = rt.enter();
    let mut sighup = match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::hangup()) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(error = %e, "failed to install SIGHUP listener");
            return;
        }
    };
    drop(_enter);
    tracing::info!("SIGHUP listener installed; pkill -HUP triggers glossary reload");
    rt.spawn(async move {
        loop {
            match sighup.recv().await {
                Some(()) => {
                    tracing::info!("SIGHUP received; forwarding reload signal");
                    if tx.send(()).is_err() {
                        tracing::debug!("reload channel closed; SIGHUP listener exiting");
                        return;
                    }
                    wake();
                }
                None => {
                    tracing::debug!("SIGHUP stream ended; listener exiting");
                    return;
                }
            }
        }
    });
}

/// Set the file at `path` to mode `0o600` (owner read/write only). Called
/// by `History` after writing the keyfile. On non-Unix platforms the
/// equivalent caller path no-ops via `cfg(not(unix))` dispatch in
/// `src/history/crypto.rs`.
pub(crate) fn sync_directory(path: &Path) -> std::io::Result<()> {
    File::open(path)?.sync_all()
}

fn validate_owner(actual_uid: u32, effective_uid: u32, path: &Path) -> std::io::Result<()> {
    if actual_uid == effective_uid {
        Ok(())
    } else {
        Err(Error::new(
            ErrorKind::PermissionDenied,
            format!(
                "secret file must be owned by the current user: {}",
                path.display()
            ),
        ))
    }
}

fn validate_metadata_owner(metadata: &std::fs::Metadata, path: &Path) -> std::io::Result<()> {
    // SAFETY: geteuid has no preconditions and does not retain pointers.
    let effective_uid = unsafe { libc::geteuid() };
    validate_owner(metadata.uid(), effective_uid, path)
}

pub(crate) fn set_owner_only_permissions(path: &Path) -> std::io::Result<()> {
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            format!(
                "refusing owner-only permissions on symlink {}",
                path.display()
            ),
        ));
    }
    if !metadata.file_type().is_file() {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            format!(
                "secret destination is not a regular file: {}",
                path.display()
            ),
        ));
    }
    validate_metadata_owner(&metadata, path)?;
    let perms = std::fs::Permissions::from_mode(0o600);
    std::fs::set_permissions(path, perms)
}

pub(crate) fn secure_read_file(path: &Path) -> std::io::Result<Vec<u8>> {
    reject_unsafe_destination(path)?;
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    let metadata = file.metadata()?;
    if !metadata.file_type().is_file() {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            format!("secret source is not a regular file: {}", path.display()),
        ));
    }
    validate_metadata_owner(&metadata, path)?;
    if metadata.permissions().mode() & 0o077 != 0 {
        return Err(Error::new(
            ErrorKind::PermissionDenied,
            format!("secret file must be owner-only (0600): {}", path.display()),
        ));
    }
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(&mut file, &mut bytes)?;
    Ok(bytes)
}

pub(crate) fn secure_atomic_write(
    path: &Path,
    contents: &[u8],
    failure: super::SecureWriteFailure,
) -> std::io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        Error::new(
            ErrorKind::InvalidInput,
            format!("secret destination has no parent: {}", path.display()),
        )
    })?;
    reject_unsafe_destination(path)?;

    let file_name = path.file_name().ok_or_else(|| {
        Error::new(
            ErrorKind::InvalidInput,
            format!("secret destination has no file name: {}", path.display()),
        )
    })?;
    let mut random = [0u8; 8];
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut random);
    let suffix = u64::from_ne_bytes(random);
    let temp_path = parent.join(format!(
        ".{}.tmp-{}-{suffix:016x}",
        file_name.to_string_lossy(),
        std::process::id()
    ));

    let result = (|| {
        let mut temp = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&temp_path)?;
        let metadata = temp.metadata()?;
        validate_metadata_owner(&metadata, &temp_path)?;
        if !metadata.file_type().is_file()
            || metadata.file_type().is_block_device()
            || metadata.file_type().is_char_device()
            || metadata.file_type().is_fifo()
            || metadata.file_type().is_socket()
        {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                format!(
                    "temporary secret destination is not a regular file: {}",
                    temp_path.display()
                ),
            ));
        }
        if matches!(failure, super::SecureWriteFailure::Permission) {
            return Err(Error::new(
                ErrorKind::PermissionDenied,
                "injected owner-only permission failure",
            ));
        }
        temp.write_all(contents)?;
        temp.flush()?;
        temp.sync_all()?;
        drop(temp);

        // A destination swapped after the first check must not be followed or
        // silently replaced. The containing directory is expected to be
        // private to the user; this second check narrows the remaining race.
        reject_unsafe_destination(path)?;
        if matches!(failure, super::SecureWriteFailure::Rename) {
            return Err(Error::other("injected atomic rename failure"));
        }
        std::fs::rename(&temp_path, path)?;
        File::open(parent)?.sync_all()?;
        Ok(())
    })();

    if result.is_err() {
        let _ = std::fs::remove_file(&temp_path);
    }
    result
}

pub(crate) fn rename_legacy_key_to_recovery(source: &Path, recovery: &Path) -> std::io::Result<()> {
    let source_meta = std::fs::symlink_metadata(source)?;
    if source_meta.file_type().is_symlink() || !source_meta.file_type().is_file() {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            format!("legacy key is not a regular file: {}", source.display()),
        ));
    }
    validate_metadata_owner(&source_meta, source)?;
    set_owner_only_permissions(source)?;
    match std::fs::symlink_metadata(recovery) {
        Err(e) if e.kind() == ErrorKind::NotFound => {}
        Ok(_) => {
            return Err(Error::new(
                ErrorKind::AlreadyExists,
                format!(
                    "recovery key destination already exists: {}",
                    recovery.display()
                ),
            ))
        }
        Err(e) => return Err(e),
    }
    std::fs::rename(source, recovery)?;
    let parent = recovery.parent().ok_or_else(|| {
        Error::new(
            ErrorKind::InvalidInput,
            format!("recovery destination has no parent: {}", recovery.display()),
        )
    })?;
    File::open(parent)?.sync_all()?;
    Ok(())
}

fn reject_unsafe_destination(path: &Path) -> std::io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(Error::new(
            ErrorKind::InvalidInput,
            format!("refusing symlink secret destination: {}", path.display()),
        )),
        Ok(metadata) if !metadata.file_type().is_file() => Err(Error::new(
            ErrorKind::InvalidInput,
            format!(
                "secret destination is not a regular file: {}",
                path.display()
            ),
        )),
        Ok(metadata) => validate_metadata_owner(&metadata, path),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;
    use tempfile::NamedTempFile;

    #[test]
    fn owner_validation_rejects_a_file_owned_by_another_user() {
        let error = validate_owner(501, 502, Path::new("secret")).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::PermissionDenied);
        assert!(error.to_string().contains("owned by the current user"));
    }

    #[test]
    fn owner_validation_accepts_the_effective_user() {
        validate_owner(501, 501, Path::new("secret")).unwrap();
    }

    #[test]
    fn set_owner_only_permissions_writes_0o600() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(f, "secret bytes").unwrap();
        let path = f.path().to_path_buf();
        // Pre-condition: tempfile defaults are 0o600 on most Unixes, but
        // we explicitly set 0o644 first to make the test meaningful.
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();

        set_owner_only_permissions(&path).expect("chmod 0o600 should succeed");

        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        // PermissionsExt::mode returns the full st_mode; mask off the
        // file-type bits.
        assert_eq!(
            mode & 0o777,
            0o600,
            "expected 0o600, got {:o}",
            mode & 0o777
        );
    }
}
