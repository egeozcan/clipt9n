//! Settings editor — open, update loop, and the save transaction.
//! The view (`src/ui/settings.rs`) is pure; every effect lands here.

use std::path::Path;
use std::time::SystemTime;

use crate::error::TranslateError;
use crate::platform::Platform;
use crate::ui::prompt_default_inner_size;
use crate::ui::settings::{KeyStorage, SettingsModel, SettingsOutcome};

impl super::ClipApp {
    /// Open the settings window seeded from the live config. Re-entrant:
    /// if the editor is already up, this just refocuses instead of
    /// discarding whatever the user has typed.
    pub(super) fn dispatch_open_settings(&mut self, ctx: &egui::Context) {
        if matches!(self.app_state, super::AppState::Settings { .. }) {
            super::pure::reset_focus_loss_latch(&mut self.has_been_focused);
            self.set_window_visible(ctx, true);
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            crate::platform::current().activate_app();
            return;
        }

        // Refuse to replace states that own work we would destroy.
        //
        // A translation in flight has a worker whose outcome is matched
        // against the current state; clobbering it here means an inline
        // replacement silently never pastes and a normal translation
        // never reaches the clipboard or history. The wizard owns
        // in-flight verification checks whose results would land in a
        // later wizard session and advance it on the wrong key — and
        // with no working key there is nothing for a Save to rebuild a
        // provider from anyway.
        let busy = match &self.app_state {
            super::AppState::Translating { .. } | super::AppState::TranslatingInline { .. } => {
                Some("a translation is in flight")
            }
            super::AppState::SetupWizard { .. } => Some("the setup wizard is open"),
            _ => None,
        };
        if let Some(reason) = busy {
            tracing::info!(reason, "settings request ignored");
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            return;
        }

        let model = Box::new(self.build_settings_model());

        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(
            crate::ui::settings::SETTINGS_INNER_SIZE,
        ));
        super::pure::reset_focus_loss_latch(&mut self.has_been_focused);
        self.set_window_visible(ctx, true);
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        crate::platform::current().activate_app();
        self.app_state = super::AppState::Settings { model };
    }

    /// Seed a settings model from the live config. Shared by the tray
    /// menu and the `--settings` launch flag.
    pub(super) fn build_settings_model(&self) -> SettingsModel {
        // Probe the platform rather than asking `self.secrets`: an
        // EnvSecrets-backed resolver always answers `false` for
        // `keychain_available` regardless of the OS's actual state
        // (same reasoning as `dispatch_rerun_wizard`).
        let keychain_available = crate::secrets::keychain_probe(&self.cfg.provider.api_key.service);
        let has_stored_key = crate::secrets::resolve(&self.cfg.provider.api_key)
            .get_api_key()
            .is_ok();
        SettingsModel {
            cfg: self.cfg.clone(),
            original: self.cfg.clone(),
            key_storage: KeyStorage::from_source(&self.cfg.provider.api_key.source),
            keychain_available,
            has_stored_key,
            config_path_display: self.config_path().display().to_string(),
            config_mtime: config_mtime(self.config_path()),
            ..Default::default()
        }
    }

    pub(super) fn update_settings(&mut self, ctx: &egui::Context, mut model: Box<SettingsModel>) {
        let outcome = crate::ui::settings::draw(ctx, &mut model);
        match outcome {
            Some(SettingsOutcome::Cancel) => {
                tracing::info!("settings cancelled — no changes written");
                self.dismiss_settings_to_idle(ctx);
            }
            Some(SettingsOutcome::Save) => match self.apply_settings(&mut model) {
                Ok(()) => self.dismiss_settings_to_idle(ctx),
                Err(e) => {
                    tracing::warn!(error = %e, "settings save rejected");
                    model.err_msg = e.to_string();
                    self.app_state = super::AppState::Settings { model };
                }
            },
            Some(SettingsOutcome::OpenConfigFile) => {
                self.dispatch_open_config();
                self.app_state = super::AppState::Settings { model };
            }
            Some(SettingsOutcome::OpenProviderKeyUrl(url)) => {
                ctx.open_url(egui::OpenUrl {
                    url: url.to_string(),
                    new_tab: true,
                });
                self.app_state = super::AppState::Settings { model };
            }
            None => {
                self.app_state = super::AppState::Settings { model };
            }
        }
    }

    /// Commit the working copy.
    ///
    /// Ordering matters: everything that can fail — validation, key
    /// resolution, provider construction — runs against a *candidate*
    /// config before a single byte is written or a single field of
    /// `self` is touched. A rejected save therefore leaves the running
    /// app exactly as it was, with the user's edits still on screen.
    /// The setup wizard follows the same candidate/commit/publish ordering.
    fn apply_settings(&mut self, model: &mut SettingsModel) -> Result<(), TranslateError> {
        let cfg_path = self.config_path().to_path_buf();
        let config_dir = cfg_path
            .parent()
            .ok_or_else(|| TranslateError::Config("config path has no parent".into()))?
            .to_path_buf();

        let mut new_cfg = model.cfg.clone();
        new_cfg.provider.api_key.source = model.key_storage.as_source().into();
        if model.key_storage == KeyStorage::File && new_cfg.provider.api_key.path.trim().is_empty()
        {
            new_cfg.provider.api_key.path = crate::secrets::FileSecrets::keyfile_path(&config_dir)
                .to_string_lossy()
                .into_owned();
        }
        new_cfg.validate()?;

        // Resolve the key and credential write plan before constructing
        // the provider. Environment storage is read-only: a typed key is
        // rejected and the named variable must already resolve.
        let typed_key = (!model.api_key.is_empty()).then(|| model.api_key.clone());
        let (key, credential) = credential_plan(&new_cfg, typed_key)?;

        // Construct before committing. This is local work (URL parsing,
        // header assembly) — no network call, so it is cheap and safe to
        // use as a gate.
        let new_provider = crate::llm::factory::build_provider(&new_cfg, key, None)?;

        // Refuse to overwrite a file that changed underneath us. The
        // window offers an "Open config.toml" button, so editing the
        // file while this is open is an invited workflow — and this
        // model still holds the snapshot from when it opened, which
        // would silently revert those edits (including `[templates]`,
        // which the GUI doesn't show at all).
        if config_changed_since(&cfg_path, model.config_mtime) {
            return Err(TranslateError::Config(
                "config.toml changed on disk since this window opened; nothing was saved. \
                 Close and reopen Settings to pick up those edits."
                    .into(),
            ));
        }

        let committed = crate::config_commit::ConfigCommitter::new(
            crate::config_commit::DiskAtomicConfig::new(&cfg_path),
            crate::config_commit::SystemCredentialStore::new(&config_dir),
        )
        .commit(new_cfg, credential)?;
        let new_cfg = committed.config;

        // Point the glossary at its (possibly new) file and re-read it.
        // `reload_glossary` keeps the previous entries on a parse error,
        // so a typo'd filename degrades rather than emptying the glossary.
        let new_glossary_path = config_dir.join(&new_cfg.glossary.file);
        if new_glossary_path != self.glossary_path {
            self.glossary_path = new_glossary_path;
            self.reload_glossary();
        }

        self.cfg = new_cfg;
        self.provider = Some(new_provider);
        model.api_key.clear();
        model.original = self.cfg.clone();
        model.config_mtime = config_mtime(&cfg_path);
        // Re-probe rather than assuming: with env storage the typed key
        // was never written anywhere, so "a key is stored" may still be
        // false even though the save succeeded.
        model.has_stored_key = crate::secrets::resolve(&self.cfg.provider.api_key)
            .get_api_key()
            .is_ok();
        model.err_msg.clear();
        tracing::info!(
            provider = %self.cfg.provider.kind,
            model = %self.cfg.provider.model,
            path = %cfg_path.display(),
            "settings saved; provider rebuilt"
        );
        Ok(())
    }

    fn dismiss_settings_to_idle(&mut self, ctx: &egui::Context) {
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(prompt_default_inner_size(
            &self.cfg.ui,
        )));
        self.app_state = super::AppState::Idle;
        self.set_window_visible(ctx, false);
    }
}

fn credential_plan(
    cfg: &crate::config::Config,
    typed_key: Option<zeroize::Zeroizing<String>>,
) -> Result<(zeroize::Zeroizing<String>, crate::config_commit::Credential), TranslateError> {
    if cfg.provider.api_key.source == "env" || cfg.provider.api_key.source == "prompt" {
        if typed_key.is_some() {
            return Err(TranslateError::Config(format!(
                "cannot save a typed API key to environment storage; set {} and clear the key field",
                cfg.provider.api_key.env_var
            )));
        }
        let key = crate::secrets::resolve(&cfg.provider.api_key)
            .get_api_key()
            .map_err(|_| {
                TranslateError::Config(format!(
                    "environment storage requires {}; export it before Save",
                    cfg.provider.api_key.env_var
                ))
            })?;
        return Ok((key, crate::config_commit::Credential::Keep));
    }

    match typed_key {
        Some(key) => Ok((key.clone(), crate::config_commit::Credential::Store(key))),
        None => {
            let key = crate::secrets::resolve(&cfg.provider.api_key)
                .get_api_key()
                .map_err(|e| {
                    TranslateError::Config(format!(
                        "no API key available for the selected storage ({e}) — enter one above"
                    ))
                })?;
            Ok((key, crate::config_commit::Credential::Keep))
        }
    }
}

/// Last-modified time of the config file, or `None` if it doesn't exist
/// or the filesystem won't report one.
fn config_mtime(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// Whether the config file has been written since `opened_at`.
/// A file that has appeared or vanished counts as changed; an unreadable
/// mtime does not, since we then have nothing to compare — blocking
/// every save on a filesystem without mtimes would be worse than the
/// edit we are guarding against.
fn config_changed_since(path: &Path, opened_at: Option<SystemTime>) -> bool {
    match (config_mtime(path), opened_at) {
        (Some(now), Some(then)) => now != then,
        (None, None) => false,
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn environment_storage_rejects_typed_key_and_names_variable() {
        let mut cfg = crate::config::Config::default();
        cfg.provider.api_key.source = "env".into();
        cfg.provider.api_key.env_var = "CLIPT9N_TYPED_ENV_REJECTED".into();
        let err =
            credential_plan(&cfg, Some(zeroize::Zeroizing::new("sk-typed".into()))).unwrap_err();
        let message = err.to_string();
        assert!(message.contains("CLIPT9N_TYPED_ENV_REJECTED"), "{message}");
    }

    #[test]
    fn environment_storage_requires_resolved_variable() {
        let mut cfg = crate::config::Config::default();
        cfg.provider.api_key.source = "env".into();
        cfg.provider.api_key.env_var = "CLIPT9N_REQUIRED_ENV_MISSING".into();
        std::env::remove_var(&cfg.provider.api_key.env_var);
        let err = credential_plan(&cfg, None).unwrap_err();
        assert!(
            err.to_string().contains("CLIPT9N_REQUIRED_ENV_MISSING"),
            "{err}"
        );
    }

    #[test]
    fn unchanged_file_is_not_flagged() {
        let f = tempfile::NamedTempFile::new().unwrap();
        let at_open = config_mtime(f.path());
        assert!(at_open.is_some());
        assert!(!config_changed_since(f.path(), at_open));
    }

    #[test]
    fn external_write_is_flagged() {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        let at_open = config_mtime(f.path());
        // Coarse mtime granularity on some filesystems means a write in
        // the same tick can land on the identical timestamp; sleep past
        // it so the test asserts the comparison, not the clock.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        writeln!(f, "[ui]\ndensity = \"compact\"").unwrap();
        f.flush().unwrap();
        assert!(config_changed_since(f.path(), at_open));
    }

    #[test]
    fn appearing_or_vanishing_file_counts_as_changed() {
        let f = tempfile::NamedTempFile::new().unwrap();
        let path = f.path().to_path_buf();
        // Existed at open, gone now.
        let at_open = config_mtime(&path);
        drop(f);
        assert!(config_changed_since(&path, at_open));
        // Absent at open, absent now — nothing to clobber.
        assert!(!config_changed_since(&path, None));
    }
}
