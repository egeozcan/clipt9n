//! Unix (Linux + macOS) signal handling. Currently exposes a SIGHUP
//! listener that forwards reload requests to a sync channel. Lives in
//! `platform/` per the cross-platform discipline rule (no `cfg(unix)` in
//! `app.rs` or anywhere else).

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
/// The task runs until the runtime is dropped; if `tx` is dropped, sends
/// fail silently (logged at debug) and the task exits.
pub(crate) fn install(rt: &Runtime, tx: Sender<()>) {
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
#[allow(dead_code)]
pub(crate) fn set_owner_only_permissions(path: &std::path::Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let perms = std::fs::Permissions::from_mode(0o600);
    std::fs::set_permissions(path, perms)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;
    use tempfile::NamedTempFile;

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
