//! Tray icon — status computation, hide-modal lifecycle, and
//! dismissal-to-idle for tray-specific modals. Extracted from
//! `app/mod.rs` Step 6b of the improvement plan.

use egui::ViewportCommand;

use super::pure;
use super::AppState;

impl super::ClipApp {
    pub(super) fn dispatch_hide_tray_request(&mut self, ctx: &egui::Context) {
        // Get the active hotkey display string. Falls back to a
        // generic placeholder if the helper isn't available.
        let hotkey_display = self.cfg.hotkey_display();
        let model = crate::ui::tray_modal::TrayHideModel { hotkey_display };
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(
            crate::ui::tray_modal::TRAY_HIDE_MODAL_SIZE,
        ));
        pure::reset_focus_loss_latch(&mut self.has_been_focused);
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        self.app_state = AppState::ConfirmingTrayHide { model };
    }

    pub(super) fn update_confirming_tray_hide(
        &mut self,
        ctx: &egui::Context,
        model: crate::ui::tray_modal::TrayHideModel,
    ) {
        let outcome = crate::ui::tray_modal::draw(ctx, &model);
        match outcome {
            Some(crate::ui::tray_modal::TrayHideOutcome::Cancel) => {
                self.dismiss_tray_modal_to_idle(ctx);
            }
            Some(crate::ui::tray_modal::TrayHideOutcome::Confirm) => {
                // Persist state, drop the tray, dismiss to idle.
                self.state.tray.visible = false;
                if let Err(e) = self.state.save(&self.state_path) {
                    tracing::warn!(error = %e, "failed to persist tray.visible=false");
                }
                self.tray = None; // Drop = remove from menu bar
                tracing::info!(
                    "tray hidden via user confirmation; relaunch with --show-tray to restore"
                );
                self.dismiss_tray_modal_to_idle(ctx);
            }
            None => {
                // Modal still open — re-store the state for next frame.
                self.app_state = AppState::ConfirmingTrayHide { model };
            }
        }
    }

    /// Compute the appropriate `TrayStatus` for the current app state.
    /// Called every frame from `update()`; the per-frame cost is one
    /// match plus a cheap atomic load.
    ///
    /// Priority (highest first): NoApiKey > Warn(KeychainStaleKey) >
    /// Warn(AccessibilityPermissionRevoked) > Warn(HotkeyInUse) >
    /// Warn(GlossaryMalformed) > Ready.
    ///
    /// Note: Warn(KeychainStaleKey) is intentionally NOT returned here.
    /// The 401-stale-key surface lives in `handle_translation_done`'s
    /// 401 arm, which directly calls `tray.set_status(KeychainStaleKey)`
    /// then transitions to `AppState::SetupWizard`. The transition fires
    /// `refresh_tray_status` later in the same frame, which sees
    /// `SetupWizard` and overwrites the pill back to `NoApiKey` — so the
    /// user effectively goes straight from `Ready` to `NoApiKey` with the
    /// amber `KeychainStaleKey` flip overwritten before render. The wizard
    /// auto-opening IS the user-visible signal; the pill flip is a
    /// best-effort breadcrumb for the OS tray's event log.
    pub(super) fn compute_tray_status(&self) -> crate::tray::TrayStatus {
        // Highest priority: missing API key (the wizard would be open
        // anyway, but tray status mirrors the underlying state).
        if matches!(self.app_state, AppState::SetupWizard { .. }) {
            return crate::tray::TrayStatus::NoApiKey;
        }
        if self.accessibility_revoked {
            return crate::tray::TrayStatus::Warn(
                crate::tray::WarnReason::AccessibilityPermissionRevoked,
            );
        }
        if self.hotkey_in_use {
            return crate::tray::TrayStatus::Warn(crate::tray::WarnReason::HotkeyInUse);
        }
        if self.glossary_was_malformed_at_startup() {
            return crate::tray::TrayStatus::Warn(crate::tray::WarnReason::GlossaryMalformed);
        }
        crate::tray::TrayStatus::Ready
    }

    /// Whether the M4 startup glossary load fell back to empty due to
    /// a parse error. M4 logs a warn but doesn't surface to the UI;
    /// M7 bridges that into the tray status. Backed by a cached
    /// AtomicBool set once at startup (and cleared by a successful
    /// non-empty SIGHUP reload).
    fn glossary_was_malformed_at_startup(&self) -> bool {
        self.glossary_malformed
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Push the current status to the tray. No-op if no tray.
    pub(super) fn refresh_tray_status(&mut self) {
        let status = self.compute_tray_status();
        if let Some(tray) = self.tray.as_mut() {
            if let Err(e) = tray.set_status(status) {
                tracing::warn!(error = %e, "tray status refresh failed");
            }
        }
    }

    fn dismiss_tray_modal_to_idle(&mut self, ctx: &egui::Context) {
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(
            crate::ui::prompt_default_inner_size(&self.cfg.ui),
        ));
        self.app_state = AppState::Idle;
        ctx.send_viewport_cmd(ViewportCommand::Visible(false));
    }
}
