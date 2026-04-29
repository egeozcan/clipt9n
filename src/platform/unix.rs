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
