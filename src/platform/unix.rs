//! Unix (Linux + macOS) signal handling. Currently exposes a SIGHUP
//! listener that forwards reload requests to a sync channel. Lives in
//! `platform/` per the cross-platform discipline rule (no `cfg(unix)` in
//! `app.rs` or anywhere else).

use crossbeam_channel::Sender;
use tokio::runtime::Runtime;

/// Spawn a tokio task on `rt` that listens for SIGHUP and forwards a
/// `()` to `tx` each time the signal arrives. Caller drains `tx`'s
/// receiver in its event loop and triggers whatever reload it owns.
///
/// The task runs until the runtime is dropped; if `tx` is dropped, sends
/// fail silently (logged at debug) and the task exits. Install itself
/// is infallible — `signal()` errors are logged inside the spawned task
/// rather than propagated.
pub(crate) fn install(rt: &Runtime, tx: Sender<()>) {
    rt.spawn(async move {
        let mut sighup =
            match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::hangup()) {
                Ok(s) => s,
                Err(e) => {
                    tracing::error!(error = %e, "failed to install SIGHUP listener");
                    return;
                }
            };
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
