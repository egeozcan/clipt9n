//! Setup wizard — update loop, connectivity check, sample-translation
//! check, persist-to-disk, and dismissal. Extracted from `app/mod.rs`
//! Step 4 of the improvement plan.

use crate::error::TranslateError;
use crate::platform::Platform;
use crate::secrets::Secrets;
use crate::ui::prompt_default_inner_size;

impl super::ClipApp {
    pub(super) fn update_setup_wizard(
        &mut self,
        ctx: &egui::Context,
        mut model: crate::ui::setup::SetupWizardModel,
    ) {
        // First, drain any check results sitting on our channel.
        while let Ok((check, result)) = self.setup_check_rx.try_recv() {
            match (check, result) {
                (crate::ui::setup::SetupCheck::Connectivity, Ok(())) => {
                    model.check1 = crate::ui::setup::CheckStatus::Ok;
                    if !model.test_translation {
                        // Skip check2; advance to Done.
                        model.phase = crate::ui::setup::WizardPhase::Done;
                    } else {
                        // Kick off check2.
                        model.check2 = crate::ui::setup::CheckStatus::Running;
                        self.spawn_sample_translation_check(&model.provider, model.key.clone());
                    }
                }
                (crate::ui::setup::SetupCheck::Connectivity, Err(msg)) => {
                    model.check1 = crate::ui::setup::CheckStatus::Fail;
                    model.err_msg = msg;
                    model.phase = crate::ui::setup::WizardPhase::Error;
                }
                (crate::ui::setup::SetupCheck::SampleTranslation, Ok(())) => {
                    model.check2 = crate::ui::setup::CheckStatus::Ok;
                    model.phase = crate::ui::setup::WizardPhase::Done;
                }
                (crate::ui::setup::SetupCheck::SampleTranslation, Err(msg)) => {
                    model.check2 = crate::ui::setup::CheckStatus::Fail;
                    model.err_msg = msg;
                    model.phase = crate::ui::setup::WizardPhase::Error;
                }
            }
        }

        let outcome = crate::ui::setup::draw(ctx, &mut model);

        match outcome {
            Some(crate::ui::setup::SetupOutcome::Cancel) => {
                tracing::warn!("setup wizard cancelled — no API key persisted");
                self.dismiss_setup_to_idle(ctx);
            }
            Some(crate::ui::setup::SetupOutcome::Verify) => {
                model.phase = crate::ui::setup::WizardPhase::Verifying;
                model.check1 = crate::ui::setup::CheckStatus::Running;
                model.check2 = crate::ui::setup::CheckStatus::Idle;
                model.err_msg.clear();
                self.spawn_connectivity_check(&model.provider, model.key.clone());
                self.app_state = super::AppState::SetupWizard { model };
            }
            Some(crate::ui::setup::SetupOutcome::SaveAndStart) => {
                if let Err(e) = self.persist_setup_completion(&model) {
                    tracing::error!(error = %e, "setup wizard persist failed");
                    model.err_msg = format!("save failed: {e}");
                    model.phase = crate::ui::setup::WizardPhase::Error;
                    self.app_state = super::AppState::SetupWizard { model };
                    return;
                }
                self.dismiss_setup_to_idle(ctx);
            }
            Some(crate::ui::setup::SetupOutcome::OpenConfig) => {
                let cfg_path = self.state_path.parent().map(|p| p.join("config.toml"));
                if let Some(p) = cfg_path {
                    let plat = crate::platform::current();
                    if let Err(e) = plat.open_path(&p) {
                        tracing::warn!(error = %e, "open_path failed");
                    }
                }
                self.app_state = super::AppState::SetupWizard { model };
            }
            Some(crate::ui::setup::SetupOutcome::OpenProviderKeyUrl(url)) => {
                ctx.open_url(egui::OpenUrl {
                    url: url.to_string(),
                    new_tab: true,
                });
                self.app_state = super::AppState::SetupWizard { model };
            }
            None => {
                self.app_state = super::AppState::SetupWizard { model };
            }
        }
    }

    fn spawn_connectivity_check(&self, provider: &str, key: zeroize::Zeroizing<String>) {
        // Use the wizard-selected provider's default base URL — the
        // live cfg.provider.base_url may not match the wizard's
        // selection until Save-and-start rewrites the config.
        let provider = provider.to_string();
        let base_url = crate::ui::setup::default_base_url(&provider).to_string();
        let tx = self.setup_check_tx.clone();
        // Wake the sleeping event loop once the result lands. See
        // `ClipApp::repaint_ctx`.
        let ctx = self.repaint_ctx.clone();
        let runtime = self.runtime.handle().clone();
        runtime.spawn(async move {
            let result = run_connectivity_check(&provider, &base_url, &key).await;
            // One auto-retry per spec §13.
            let final_result = if result.is_err() {
                tokio::time::sleep(std::time::Duration::from_millis(400)).await;
                run_connectivity_check(&provider, &base_url, &key).await
            } else {
                result
            };
            let _ = tx.send((
                crate::ui::setup::SetupCheck::Connectivity,
                final_result.map_err(|e| e.to_string()),
            ));
            ctx.request_repaint();
        });
    }

    fn spawn_sample_translation_check(&self, provider_kind: &str, key: zeroize::Zeroizing<String>) {
        // The wizard's selected provider may differ from the running
        // self.provider (which was built from the cfg at startup, possibly
        // with a placeholder key). Build a fresh provider from the
        // wizard's typed key + the wizard's selected provider kind +
        // the kind-default base URL.
        let provider_kind = provider_kind.to_string();
        let cfg = self.cfg.clone();
        let templates = self.templates.clone();
        let glossary = self.glossary.clone();
        let tx = self.setup_check_tx.clone();
        // Wake the sleeping event loop once the result lands. See
        // `ClipApp::repaint_ctx`.
        let ctx = self.repaint_ctx.clone();
        let runtime = self.runtime.handle().clone();
        runtime.spawn(async move {
            let base_url_override = crate::ui::setup::default_base_url(&provider_kind);
            // Build a config with the wizard's selected provider kind so the
            // factory routes to the right provider type. The base URL override
            // ensures we use the per-provider default (from
            // setup::default_base_url), not cfg.provider.base_url, because the
            // user might be configuring a fresh provider whose base_url hasn't
            // been persisted yet.
            let mut check_cfg = cfg.clone();
            check_cfg.provider.kind = provider_kind.clone();
            let provider_result = crate::llm::factory::build_provider(
                &check_cfg,
                key.clone(),
                Some(base_url_override),
            );
            let provider = match provider_result {
                Ok(p) => p,
                Err(e) => {
                    let _ = tx.send((
                        crate::ui::setup::SetupCheck::SampleTranslation,
                        Err(e.to_string()),
                    ));
                    return;
                }
            };
            let action = crate::translator::Action::Translate { code: "de".into() };
            let attempt = || async {
                let g_snapshot = crate::glossary::Glossary::read_shared(&glossary).clone();
                let translator = crate::translator::Translator::new(
                    &cfg,
                    provider.as_ref(),
                    &templates,
                    &g_snapshot,
                );
                translator.execute(&action, "Hello, world.").await
            };
            let result = attempt().await;
            // One auto-retry per spec §13.
            let final_result = if result.is_err() {
                tokio::time::sleep(std::time::Duration::from_millis(400)).await;
                attempt().await
            } else {
                result
            };
            let _ = tx.send((
                crate::ui::setup::SetupCheck::SampleTranslation,
                final_result.map(|_| ()).map_err(|e| e.to_string()),
            ));
            ctx.request_repaint();
        });
    }

    fn dismiss_setup_to_idle(&mut self, ctx: &egui::Context) {
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(prompt_default_inner_size(
            &self.cfg.ui,
        )));
        self.app_state = super::AppState::Idle;
        self.set_window_visible(ctx, false);
    }

    fn persist_setup_completion(
        &mut self,
        model: &crate::ui::setup::SetupWizardModel,
    ) -> Result<(), TranslateError> {
        // Update in-memory config.
        let prev_kind = self.cfg.provider.kind.clone();
        self.cfg.provider.kind = model.provider.clone();
        // Auto-select sensible defaults when switching providers.
        if self.cfg.provider.kind != prev_kind {
            self.cfg.provider.model = crate::ui::setup::default_model(&model.provider).to_string();
            self.cfg.provider.base_url =
                crate::ui::setup::default_base_url(&model.provider).to_string();
        }
        let new_source = match model.storage {
            crate::ui::setup::Storage::Keychain => "keychain",
            crate::ui::setup::Storage::Env => "env",
        };
        self.cfg.provider.api_key.source = new_source.into();
        self.cfg.provider.api_key.account = model.provider.clone();
        // Update env-var name for the env case so the user knows what
        // to export.
        let (_, _, env_var) = crate::ui::setup::provider_meta(&model.provider);
        self.cfg.provider.api_key.env_var = env_var.into();

        // Persist config to disk.
        let cfg_path = self
            .state_path
            .parent()
            .map(|p| p.join("config.toml"))
            .ok_or_else(|| TranslateError::Config("state path has no parent".into()))?;
        self.cfg.persist(&cfg_path)?;

        // Persist the key to the chosen backend. Env-storage logs a
        // warning that the user must set the env var manually — the
        // wizard already showed them the variable name.
        match model.storage {
            crate::ui::setup::Storage::Keychain => {
                // Build a fresh KeychainSecrets from the just-updated
                // cfg so the key lands in the correct slot when the
                // user switches providers in the wizard. self.secrets
                // was constructed at startup from the OLD account and
                // would write to the stale slot.
                let fresh = crate::secrets::KeychainSecrets::new(
                    &self.cfg.provider.api_key.service,
                    &self.cfg.provider.api_key.account,
                );
                fresh.set_api_key(model.key.clone())?;
                // Read-back self-test: macOS silently fails to persist
                // keychain items written by unsigned / ad-hoc-signed
                // binaries (SecItemAdd reports success, the item is
                // never findable). Detect that here and auto-fallback
                // to a 0600 keyfile under the config dir so the user's
                // wizard run still produces a key that survives a
                // restart instead of throwing them back into the
                // wizard on every launch.
                let verify = crate::secrets::KeychainSecrets::new(
                    &self.cfg.provider.api_key.service,
                    &self.cfg.provider.api_key.account,
                );
                let readback_ok = matches!(
                    verify.get_api_key(),
                    Ok(read) if *read == *model.key
                );
                if !readback_ok {
                    let config_dir = self
                        .state_path
                        .parent()
                        .ok_or_else(|| TranslateError::Config("state path has no parent".into()))?;
                    let keyfile = crate::secrets::FileSecrets::keyfile_path(config_dir);
                    let file_secrets = crate::secrets::FileSecrets::new(keyfile.clone());
                    file_secrets.set_api_key(model.key.clone())?;
                    self.cfg.provider.api_key.source = "file".into();
                    self.cfg.provider.api_key.path = keyfile.to_string_lossy().into_owned();
                    self.cfg.persist(&cfg_path)?;
                    tracing::warn!(
                        path = %keyfile.display(),
                        "keychain write didn't persist; fell back to 0600 keyfile"
                    );
                }
            }
            crate::ui::setup::Storage::Env => {
                tracing::warn!(
                    env_var = %env_var,
                    "setup wizard: storage=env — user must set the env var manually before next launch"
                );
            }
        }

        // M7 Task 10 (Q6): live provider rebuild — replaces self.provider
        // so the next translation uses the just-saved key without
        // requiring a restart. The wizard's Verify gate already proved
        // the key works at the network level; if build_provider fails
        // here, the constraint is config-shape (e.g., a malformed URL).
        // Wipe self.provider so the next translation surfaces the
        // failure rather than using the stale provider.
        match crate::llm::factory::build_provider(&self.cfg, model.key.clone(), None) {
            Ok(new_provider) => {
                self.provider = Some(new_provider);
                tracing::info!("setup wizard: provider rebuilt with new key");
            }
            Err(e) => {
                self.provider = None;
                tracing::error!(error = %e, "setup wizard: provider rebuild failed; next translation will surface this");
                return Err(e);
            }
        }

        Ok(())
    }
}

// -----------------------------------------------------------------------
// Async helpers
// -----------------------------------------------------------------------

async fn run_connectivity_check(
    provider: &str,
    base_url: &str,
    key: &str,
) -> Result<(), TranslateError> {
    let (url, headers) = crate::ui::setup::connectivity_request(provider, base_url, key);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| TranslateError::Network(e.to_string()))?;
    let mut req = client.get(&url);
    for (k, v) in headers {
        req = req.header(&k, &v);
    }
    let resp = req
        .send()
        .await
        .map_err(|e| TranslateError::Network(e.to_string()))?;
    let status = resp.status();
    if status.is_success() {
        Ok(())
    } else if status.as_u16() == 401 {
        Err(TranslateError::SetupWizard(format!(
            "{} Invalid API key",
            status.as_u16()
        )))
    } else {
        Err(TranslateError::Provider {
            status: status.as_u16(),
            message: status.canonical_reason().unwrap_or("provider error").into(),
        })
    }
}
