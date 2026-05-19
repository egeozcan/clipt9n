//! Tray icon integration. `tray-icon` crate is the unified
//! cross-platform abstraction (macOS NSStatusItem / Windows shell tray
//! / Linux StatusNotifierItem). Per the cross-cutting decision, this
//! file contains no target-OS cfg attributes — all OS-specific bits
//! live inside the crate.
//!
//! Construction site is the eframe creator closure (main thread, after
//! the run loop has started). Constructed-but-failed is OK: `main.rs`
//! logs warn and runs without a tray.
//!
//! Menu drain happens in `ClipApp::update()` via a `try_recv()` on the
//! `MenuEvent::receiver()` static. The 150 ms repaint cadence
//! (`app.rs:1326`) gives sub-frame dispatch latency on user clicks.

use std::sync::OnceLock;

use crossbeam_channel::{unbounded, Receiver, Sender};
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

use crate::error::TranslateError;

/// Process-wide channel used by the installed `MenuEvent` handler to
/// hand IDs over to the eframe update loop, plus the `egui::Context`
/// used to wake eframe out of its idle sleep when a menu event fires.
///
/// Why we replace `MenuEvent::receiver()` with our own handler: when
/// the app sits in `Idle` with no visible viewport AND
/// `NSApplicationActivationPolicyAccessory`, AppKit's run loop sleeps
/// until an external event arrives. The default channel-based receiver
/// posts events to a static channel but does not wake the egui ctx, so
/// the next `update()` may not fire for many seconds. Calling
/// `ctx.request_repaint()` from the menu handler guarantees same-frame
/// drain.
static MENU_QUEUE: OnceLock<(Sender<String>, Receiver<String>)> = OnceLock::new();
static REPAINT_CTX: OnceLock<egui::Context> = OnceLock::new();
static HANDLER_INSTALLED: OnceLock<()> = OnceLock::new();

fn install_menu_handler(ctx: &egui::Context) {
    let _ = REPAINT_CTX.set(ctx.clone());
    let _ = MENU_QUEUE.get_or_init(unbounded);
    HANDLER_INSTALLED.get_or_init(|| {
        MenuEvent::set_event_handler(Some(|ev: MenuEvent| {
            if let Some((tx, _)) = MENU_QUEUE.get() {
                let _ = tx.send(ev.id.0);
            }
            if let Some(ctx) = REPAINT_CTX.get() {
                ctx.request_repaint();
            }
        }));
    });
}

/// Stable IDs used in the tray menu. Constants because the drain match
/// in `app.rs::drain_tray_events` references them too — no string
/// literal duplication.
pub const ID_TRANSLATE: &str = "clipt9n.translate";
pub const ID_HISTORY: &str = "clipt9n.history";
pub const ID_GLOSSARY_OPEN: &str = "clipt9n.glossary.open";
pub const ID_GLOSSARY_RELOAD: &str = "clipt9n.glossary.reload";
pub const ID_ACCESSIBILITY_SETTINGS: &str = "clipt9n.accessibility.settings";
pub const ID_RERUN_WIZARD: &str = "clipt9n.wizard";
pub const ID_OPEN_CONFIG: &str = "clipt9n.config.open";
pub const ID_HIDE: &str = "clipt9n.hide";
pub const ID_QUIT: &str = "clipt9n.quit";

/// Status-pill state. Drives both the icon image (dot color) and the
/// tooltip text. Mapping per spec §8 lives in `app.rs::compute_tray_status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TrayStatus {
    /// Default healthy state — green dot, tooltip "clipt9n — ready".
    #[default]
    Ready,
    /// No API key resolves. Red dot, tooltip "clipt9n — no API key".
    NoApiKey,
    /// One of: hotkey-already-in-use, glossary-malformed,
    /// accessibility-permission-revoked, keychain-stale-key.
    /// Yellow dot, tooltip carries the reason.
    Warn(WarnReason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WarnReason {
    HotkeyInUse,
    GlossaryMalformed,
    AccessibilityPermissionRevoked,
    KeychainStaleKey,
    KeychainUnavailable,
}

impl WarnReason {
    pub fn tooltip(self) -> &'static str {
        match self {
            Self::HotkeyInUse => "clipt9n — hotkey unavailable; another app is using it",
            Self::GlossaryMalformed => "clipt9n — glossary malformed; running without it",
            Self::AccessibilityPermissionRevoked => {
                "clipt9n — accessibility permission needed for hotkeys"
            }
            Self::KeychainStaleKey => "clipt9n — API key invalid; re-run setup wizard",
            Self::KeychainUnavailable => "clipt9n — keychain unavailable; using env var",
        }
    }
}

impl TrayStatus {
    pub fn tooltip(&self) -> &'static str {
        match self {
            Self::Ready => "clipt9n — ready",
            Self::NoApiKey => "clipt9n — no API key; click to run setup wizard",
            Self::Warn(r) => r.tooltip(),
        }
    }
}

/// Owns the live `TrayIcon`. Drop = remove the icon from the tray.
/// Not `Clone` (TrayIcon is `Rc<RefCell<…>>`-backed). If shared
/// ownership is ever needed, wrap in `Arc<Mutex<TrayHandle>>`. For
/// now `ClipApp` owns the only instance.
pub struct TrayHandle {
    /// Underlying tray-icon handle. Held to keep the icon alive; on
    /// drop the icon disappears from the menu bar.
    icon: TrayIcon,
    /// Last-rendered status — short-circuits a no-op `set_status` call.
    last_status: TrayStatus,
}

impl TrayHandle {
    /// Build the tray icon, attaching the menu and the initial status
    /// dot. Constructor failure is non-fatal — `main.rs` logs warn and
    /// runs without a tray.
    pub fn build(initial_status: TrayStatus, ctx: &egui::Context) -> Result<Self, TranslateError> {
        // Install the wakeup-aware MenuEvent handler before
        // TrayIconBuilder runs — set_event_handler replaces the default
        // channel receiver, and we want all events from this point
        // forward to flow through our queue.
        install_menu_handler(ctx);
        let menu = Self::build_menu()?;
        let icon = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_icon(build_icon(initial_status))
            .with_tooltip(initial_status.tooltip())
            .with_icon_as_template(true) // macOS: render with menu-bar tint (no-op on Windows/Linux; tray-icon handles the cfg internally — no platform branch needed here)
            .build()
            .map_err(|e| TranslateError::Internal(format!("tray icon build failed: {e}")))?;
        Ok(Self {
            icon,
            last_status: initial_status,
        })
    }

    /// Like `build`, but wraps construction in `catch_unwind`. On
    /// panic, returns `Err(Internal("tray construction panicked"))`.
    /// Use this from main.rs so a tray-side panic doesn't kill the
    /// app — covers spec §8 "Tray crashed" row.
    pub fn build_with_panic_isolation(
        initial_status: TrayStatus,
        ctx: &egui::Context,
    ) -> Result<Self, TranslateError> {
        use std::panic::{catch_unwind, AssertUnwindSafe};
        match catch_unwind(AssertUnwindSafe(|| Self::build(initial_status, ctx))) {
            Ok(result) => result,
            Err(_) => Err(TranslateError::Internal(
                "tray construction panicked; running without tray".into(),
            )),
        }
    }

    fn build_menu() -> Result<Menu, TranslateError> {
        let menu = Menu::new();
        menu.append(&MenuItem::with_id(
            ID_TRANSLATE,
            "Translate clipboard",
            true,
            None,
        ))
        .map_err(menu_err)?;
        menu.append(&MenuItem::with_id(ID_HISTORY, "Open history", true, None))
            .map_err(menu_err)?;
        menu.append(&PredefinedMenuItem::separator())
            .map_err(menu_err)?;
        menu.append(&MenuItem::with_id(
            ID_GLOSSARY_OPEN,
            "Open glossary",
            true,
            None,
        ))
        .map_err(menu_err)?;
        menu.append(&MenuItem::with_id(
            ID_GLOSSARY_RELOAD,
            "Reload glossary",
            true,
            None,
        ))
        .map_err(menu_err)?;
        menu.append(&PredefinedMenuItem::separator())
            .map_err(menu_err)?;
        menu.append(&MenuItem::with_id(
            ID_OPEN_CONFIG,
            "Open config",
            true,
            None,
        ))
        .map_err(menu_err)?;
        menu.append(&PredefinedMenuItem::separator())
            .map_err(menu_err)?;
        menu.append(&MenuItem::with_id(
            ID_ACCESSIBILITY_SETTINGS,
            "Open Accessibility settings",
            true,
            None,
        ))
        .map_err(menu_err)?;
        menu.append(&MenuItem::with_id(
            ID_RERUN_WIZARD,
            "Re-run setup wizard",
            true,
            None,
        ))
        .map_err(menu_err)?;
        menu.append(&MenuItem::with_id(ID_HIDE, "Hide icon", true, None))
            .map_err(menu_err)?;
        menu.append(&PredefinedMenuItem::separator())
            .map_err(menu_err)?;
        menu.append(&MenuItem::with_id(ID_QUIT, "Quit clipt9n", true, None))
            .map_err(menu_err)?;
        Ok(menu)
    }

    /// Update the status pill. No-op if the new status equals the last.
    /// Failure is best-effort — log warn at the call site.
    pub fn set_status(&mut self, status: TrayStatus) -> Result<(), TranslateError> {
        if status == self.last_status {
            return Ok(());
        }
        // Use `set_icon_with_as_template` instead of `set_icon` because
        // tray-icon 0.22.x hardcodes `icon_is_template: false` inside
        // `set_icon`, which silently undoes the constructor's
        // `with_icon_as_template(true)` flag and causes the menu-bar
        // glyph to render as opaque black on every status change.
        self.icon
            .set_icon_with_as_template(Some(build_icon(status)), true)
            .map_err(|e| TranslateError::Internal(format!("tray set_icon failed: {e}")))?;
        self.icon
            .set_tooltip(Some(status.tooltip()))
            .map_err(|e| TranslateError::Internal(format!("tray set_tooltip failed: {e}")))?;
        self.last_status = status;
        Ok(())
    }

    /// Drain ALL pending MenuEvents. Returns the *most recent* event's ID
    /// string (any earlier events in the queue are discarded — fine for
    /// our usage where at most one menu item is clicked per user
    /// interaction). `None` if the queue was empty. Caller dispatches
    /// on the ID. Static-channel global state means the receiver is
    /// process-wide; we drain from this thread (the eframe update
    /// loop's main thread) only.
    pub fn try_drain_menu_event() -> Option<String> {
        let mut last: Option<String> = None;
        if let Some((_, rx)) = MENU_QUEUE.get() {
            while let Ok(id) = rx.try_recv() {
                last = Some(id);
            }
        }
        last
    }
}

fn menu_err(e: tray_icon::menu::Error) -> TranslateError {
    TranslateError::Internal(format!("tray menu construction: {e}"))
}

/// Build the raw RGBA pixel buffer for the icon at the given size. The
/// glyph is a simple "T" stencil (clipt9n's "T") in ink color with a
/// status dot in the bottom-right corner whose color encodes the
/// status. Glyph and dot extents are scaled from a 22-px reference so
/// the helper supports future bundled icon sizes without losing the
/// existing 22-px tray contract.
fn build_icon_buffer(status: TrayStatus, size: u32) -> Vec<u8> {
    let mut buf = vec![0u8; (size * size * 4) as usize];
    // Reference geometry from the 22-px tray icon. Round to integers
    // so 22-px output is bit-for-bit unchanged from the M7 stencil.
    let scale = |v: u32| (v * size).div_ceil(22);
    let bar_y = scale(4)..scale(7);
    let bar_x = scale(4)..scale(18);
    let str_y = scale(7)..scale(18);
    let str_x = scale(9)..scale(13);
    let dot_size = scale(4).max(3);
    let dot_start = size.saturating_sub(dot_size + 1);
    let dot_y = dot_start..dot_start + dot_size;
    let dot_x = dot_start..dot_start + dot_size;

    let (dr, dg, db) = match status {
        TrayStatus::Ready => (0xC8, 0xFF, 0x5E),    // accent green
        TrayStatus::NoApiKey => (0xFF, 0x76, 0x76), // soft red
        TrayStatus::Warn(_) => (0xFF, 0xC4, 0x5E),  // amber
    };

    for y in 0..size {
        for x in 0..size {
            let i = ((y * size + x) * 4) as usize;
            let in_bar = bar_y.contains(&y) && bar_x.contains(&x);
            let in_stroke = str_y.contains(&y) && str_x.contains(&x);
            let in_dot = dot_y.contains(&y) && dot_x.contains(&x);
            if in_dot {
                buf[i] = dr;
                buf[i + 1] = dg;
                buf[i + 2] = db;
                buf[i + 3] = 255;
            } else if in_bar || in_stroke {
                // Glyph: black-with-transparent stencil. macOS template
                // rendering tints this to match the menu bar appearance.
                buf[i + 3] = 255;
            }
        }
    }
    buf
}

/// Build the 22×22 RGBA icon for the given status. Procedural — no
/// asset bundling.
pub(crate) fn build_icon(status: TrayStatus) -> Icon {
    const SIZE: u32 = 22;
    let buf = build_icon_buffer(status, SIZE);
    Icon::from_rgba(buf, SIZE, SIZE).expect("22x22 RGBA buffer is always valid")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_buffer_validates_for_each_status() {
        // Constructor doesn't panic for any status. (Real-OS tray
        // construction only happens via TrayHandle::build, which we
        // don't call here — that requires the tray-icon main-thread
        // runtime.)
        let _ = build_icon(TrayStatus::Ready);
        let _ = build_icon(TrayStatus::NoApiKey);
        let _ = build_icon(TrayStatus::Warn(WarnReason::HotkeyInUse));
        let _ = build_icon(TrayStatus::Warn(WarnReason::GlossaryMalformed));
        let _ = build_icon(TrayStatus::Warn(WarnReason::AccessibilityPermissionRevoked));
        let _ = build_icon(TrayStatus::Warn(WarnReason::KeychainStaleKey));
        let _ = build_icon(TrayStatus::Warn(WarnReason::KeychainUnavailable));
    }

    #[test]
    fn icon_buffer_dot_color_matches_status() {
        let cases = [
            (TrayStatus::Ready, [0xC8, 0xFF, 0x5E]),
            (TrayStatus::NoApiKey, [0xFF, 0x76, 0x76]),
            (
                TrayStatus::Warn(WarnReason::HotkeyInUse),
                [0xFF, 0xC4, 0x5E],
            ),
        ];
        for (status, expected_rgb) in cases {
            let buf = build_icon_buffer(status, 22);
            // pixel at (y=18, x=18) — inside the 4x4 dot at rows 17..21, cols 17..21
            let i = (18 * 22 + 18) * 4;
            assert_eq!(
                &buf[i..i + 3],
                &expected_rgb,
                "dot color for {status:?} should match expected RGB"
            );
            assert_eq!(buf[i + 3], 255, "dot alpha should be 255");
        }
    }

    #[test]
    fn warn_tooltips_are_distinct() {
        let reasons = [
            WarnReason::HotkeyInUse,
            WarnReason::GlossaryMalformed,
            WarnReason::AccessibilityPermissionRevoked,
            WarnReason::KeychainStaleKey,
            WarnReason::KeychainUnavailable,
        ];
        let tips: Vec<&'static str> = reasons.iter().map(|r| r.tooltip()).collect();
        let mut sorted = tips.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(
            sorted.len(),
            tips.len(),
            "every WarnReason should have a distinct tooltip"
        );
    }

    #[test]
    fn status_tooltip_dispatches_to_warn_reason() {
        assert_eq!(TrayStatus::Ready.tooltip(), "clipt9n — ready");
        assert_eq!(
            TrayStatus::NoApiKey.tooltip(),
            "clipt9n — no API key; click to run setup wizard"
        );
        let warn = TrayStatus::Warn(WarnReason::KeychainStaleKey);
        assert_eq!(warn.tooltip(), WarnReason::KeychainStaleKey.tooltip());
    }

    #[test]
    fn menu_ids_are_unique() {
        let ids = [
            ID_TRANSLATE,
            ID_HISTORY,
            ID_GLOSSARY_OPEN,
            ID_GLOSSARY_RELOAD,
            ID_OPEN_CONFIG,
            ID_ACCESSIBILITY_SETTINGS,
            ID_RERUN_WIZARD,
            ID_HIDE,
            ID_QUIT,
        ];
        let mut sorted: Vec<&str> = ids.into_iter().collect();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), ids.len(), "menu IDs must be unique");
    }
}
