//! Setup wizard — update loop, connectivity check, sample-translation
//! check, persist-to-disk, and dismissal. Extracted from `app/mod.rs`
//! Step 4 of the improvement plan.

use crate::error::TranslateError;
use crate::platform::Platform;
use crate::ui::prompt_default_inner_size;

impl super::ClipApp {
    pub(super) fn update_setup_wizard(
        &mut self,
        ctx: &egui::Context,
        mut model: crate::ui::setup::SetupWizardModel,
    ) {
        // Reject edits made between frames before draining results. This
        // prevents a result for the prior immutable scope from touching the
        // newly-edited model.
        invalidate_changed_verification(&mut self.setup_verification_gen, &mut model);

        // First, drain any check results sitting on our channel.
        while let Ok((verification_id, check, result)) = self.setup_check_rx.try_recv() {
            let start_sample = verification_id == model.verification_id
                && check == crate::ui::setup::SetupCheck::Connectivity
                && result.is_ok()
                && model.test_translation;
            if !apply_setup_result(&mut model, verification_id, check, result) {
                tracing::debug!(
                    ?verification_id,
                    "discarding stale setup verification result"
                );
                continue;
            }
            if start_sample {
                self.spawn_sample_translation_check(
                    verification_id,
                    &model.provider,
                    model.verification_key.clone(),
                );
            }
        }

        let mut outcome = crate::ui::setup::draw(ctx, &mut model);
        // Edits can occur inside draw, including in the same frame as a Save
        // click. Invalidate and suppress that outcome before any persistence.
        if invalidate_changed_verification(&mut self.setup_verification_gen, &mut model) {
            outcome = None;
        }

        match outcome {
            Some(crate::ui::setup::SetupOutcome::Cancel) => {
                tracing::warn!("setup wizard cancelled — no API key persisted");
                invalidate_setup_verification(&mut self.setup_verification_gen, &mut model);
                self.dismiss_setup_to_idle(ctx);
            }
            Some(crate::ui::setup::SetupOutcome::Verify) => {
                let key = match verification_key(&model) {
                    Ok(key) => key,
                    Err(e) => {
                        model.err_msg = e.to_string();
                        model.phase = crate::ui::setup::WizardPhase::Error;
                        self.app_state = super::AppState::SetupWizard { model };
                        return;
                    }
                };
                seed_setup_verification(&mut self.setup_verification_gen, &mut model);
                model.verification_key = key.clone();
                model.verification_scope = Some(capture_setup_scope(&model));
                model.phase = crate::ui::setup::WizardPhase::Verifying;
                model.check1 = crate::ui::setup::CheckStatus::Running;
                model.check2 = crate::ui::setup::CheckStatus::Idle;
                model.err_msg.clear();
                self.spawn_connectivity_check(model.verification_id, &model.provider, key);
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
                let plat = crate::platform::current();
                if let Err(e) = plat.open_path(self.config_path()) {
                    tracing::warn!(error = %e, "open_path failed");
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

    fn spawn_connectivity_check(
        &self,
        verification_id: crate::ui::setup::VerificationId,
        provider: &str,
        key: zeroize::Zeroizing<String>,
    ) {
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
                verification_id,
                crate::ui::setup::SetupCheck::Connectivity,
                final_result.map_err(|e| e.to_string()),
            ));
            ctx.request_repaint();
        });
    }

    fn spawn_sample_translation_check(
        &self,
        verification_id: crate::ui::setup::VerificationId,
        provider_kind: &str,
        key: zeroize::Zeroizing<String>,
    ) {
        // The wizard's selected provider may differ from the running
        // self.provider (which was built from the cfg at startup, possibly
        // with a placeholder key). Build a fresh provider from the
        // verification key + the wizard's selected provider kind +
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
            let check_cfg = match sample_check_config(
                &cfg,
                &provider_kind,
                crate::ui::setup::default_base_url(&provider_kind),
            ) {
                Ok(check_cfg) => check_cfg,
                Err(e) => {
                    let _ = tx.send((
                        verification_id,
                        crate::ui::setup::SetupCheck::SampleTranslation,
                        Err(e.to_string()),
                    ));
                    return;
                }
            };
            let provider_result =
                crate::llm::factory::build_provider(&check_cfg, key.clone(), None);
            let provider = match provider_result {
                Ok(p) => p,
                Err(e) => {
                    let _ = tx.send((
                        verification_id,
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
                verification_id,
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
        // Defense in depth: Save is rejected unless the current model still
        // matches the immutable scope that reached Done.
        let key = verified_key_for_save(model)?;
        let profile = crate::llm::profiles::provider_profile(&model.provider)?;
        let mut candidate = self.cfg.clone();
        let provider_changed = candidate.provider.kind != profile.id;
        candidate.provider.kind = profile.id.to_string();
        if provider_changed {
            candidate.provider.model = profile.default_model.to_string();
            candidate.provider.base_url = profile.default_base_url.to_string();
        }
        candidate.provider.api_key.source = match model.storage {
            crate::ui::setup::Storage::Keychain => "keychain",
            crate::ui::setup::Storage::Env => "env",
        }
        .into();
        candidate.provider.api_key.account = profile.account.to_string();
        candidate.provider.api_key.env_var = profile.env_var.to_string();
        candidate.validate()?;

        let credential = match model.storage {
            crate::ui::setup::Storage::Keychain => {
                crate::config_commit::Credential::Store(key.clone())
            }
            crate::ui::setup::Storage::Env => crate::config_commit::Credential::Keep,
        };

        // Provider construction is the final non-I/O gate. Neither live
        // state nor config.toml has changed if it rejects the candidate.
        let new_provider = crate::llm::factory::build_provider(&candidate, key, None)?;
        let cfg_path = self.config_path().to_path_buf();
        let config_dir = cfg_path
            .parent()
            .ok_or_else(|| TranslateError::Config("config path has no parent".into()))?;
        let committed = crate::config_commit::ConfigCommitter::new(
            crate::config_commit::DiskAtomicConfig::new(&cfg_path),
            crate::config_commit::SystemCredentialStore::new(config_dir),
        )
        .commit(candidate, credential)?;

        self.cfg = committed.config;
        self.provider = Some(new_provider);
        tracing::info!("setup wizard: configuration committed and provider rebuilt");
        Ok(())
    }
}

// -----------------------------------------------------------------------
// Pure verification helpers and async checks
// -----------------------------------------------------------------------

fn verification_key(
    model: &crate::ui::setup::SetupWizardModel,
) -> Result<zeroize::Zeroizing<String>, TranslateError> {
    if !model.key.is_empty() {
        return Ok(model.key.clone());
    }
    let profile = crate::llm::profiles::provider_profile(&model.provider)?;
    if model.storage != crate::ui::setup::Storage::Env {
        return Err(TranslateError::Config(
            "enter an API key before Verify".into(),
        ));
    }
    std::env::var(profile.env_var)
        .map(zeroize::Zeroizing::new)
        .map_err(|_| {
            TranslateError::Config(format!(
                "environment verification requires {}; export it before Verify",
                profile.env_var
            ))
        })
}

fn capture_setup_scope(
    model: &crate::ui::setup::SetupWizardModel,
) -> crate::ui::setup::SetupVerificationScope {
    crate::ui::setup::SetupVerificationScope::capture(model)
}

pub(super) fn seed_setup_verification(
    generation: &mut u64,
    model: &mut crate::ui::setup::SetupWizardModel,
) {
    *generation = generation.wrapping_add(1);
    model.verification_id = crate::ui::setup::VerificationId(*generation);
    model.verification_scope = None;
    model.verification_key.clear();
}

fn reset_setup_verification(model: &mut crate::ui::setup::SetupWizardModel) {
    model.verification_scope = None;
    model.verification_key.clear();
    model.phase = crate::ui::setup::WizardPhase::Entry;
    model.check1 = crate::ui::setup::CheckStatus::Idle;
    model.check2 = crate::ui::setup::CheckStatus::Idle;
    model.err_msg.clear();
}

fn invalidate_setup_verification(
    generation: &mut u64,
    model: &mut crate::ui::setup::SetupWizardModel,
) {
    seed_setup_verification(generation, model);
    reset_setup_verification(model);
}

fn invalidate_changed_verification(
    generation: &mut u64,
    model: &mut crate::ui::setup::SetupWizardModel,
) -> bool {
    let changed = model
        .verification_scope
        .as_ref()
        .is_some_and(|scope| !scope.matches(model));
    if changed {
        invalidate_setup_verification(generation, model);
    }
    changed
}

fn verified_key_for_save(
    model: &crate::ui::setup::SetupWizardModel,
) -> Result<zeroize::Zeroizing<String>, TranslateError> {
    if !crate::ui::setup::save_enabled(model) {
        return Err(TranslateError::Config(
            "setup inputs changed after verification; verify again before Save".into(),
        ));
    }

    if model.storage == crate::ui::setup::Storage::Env {
        if !model.key.is_empty() {
            let profile = crate::llm::profiles::provider_profile(&model.provider)?;
            return Err(TranslateError::Config(format!(
                "cannot save a typed API key to environment storage; set {} and clear the key field",
                profile.env_var
            )));
        }
        let current = verification_key(model)?;
        if current != model.verification_key {
            return Err(TranslateError::Config(
                "environment API key changed after verification; verify again before Save".into(),
            ));
        }
        return Ok(current);
    }

    if model.key != model.verification_key {
        return Err(TranslateError::Config(
            "API key changed after verification; verify again before Save".into(),
        ));
    }
    Ok(model.verification_key.clone())
}

fn apply_setup_result(
    model: &mut crate::ui::setup::SetupWizardModel,
    verification_id: crate::ui::setup::VerificationId,
    check: crate::ui::setup::SetupCheck,
    result: Result<(), String>,
) -> bool {
    if verification_id != model.verification_id
        || !model
            .verification_scope
            .as_ref()
            .is_some_and(|scope| scope.matches(model))
    {
        return false;
    }

    match (check, result) {
        (crate::ui::setup::SetupCheck::Connectivity, Ok(())) => {
            model.check1 = crate::ui::setup::CheckStatus::Ok;
            if model.test_translation {
                model.check2 = crate::ui::setup::CheckStatus::Running;
            } else {
                model.phase = crate::ui::setup::WizardPhase::Done;
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
    true
}

fn sample_check_config(
    cfg: &crate::config::Config,
    provider_kind: &str,
    base_url: &str,
) -> Result<crate::config::Config, TranslateError> {
    let profile = crate::llm::profiles::provider_profile(provider_kind)?;
    let mut check_cfg = cfg.clone();
    check_cfg.provider.kind = profile.id.to_string();
    check_cfg.provider.model = profile.default_model.to_string();
    check_cfg.provider.base_url = base_url.to_string();
    check_cfg.provider.api_key.account = profile.account.to_string();
    check_cfg.provider.api_key.env_var = profile.env_var.to_string();
    Ok(check_cfg)
}

async fn run_connectivity_check(
    provider: &str,
    base_url: &str,
    key: &str,
) -> Result<(), TranslateError> {
    let (url, headers) = crate::ui::setup::connectivity_request(provider, base_url, key);
    let client = crate::llm::client::provider_http_client(std::time::Duration::from_secs(10))?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::llm::LlmProvider;
    use wiremock::matchers::{body_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn stale_verification_result_cannot_advance_current_wizard() {
        let mut model = crate::ui::setup::SetupWizardModel {
            provider: "anthropic".into(),
            key: zeroize::Zeroizing::new("sk-current".into()),
            verification_id: crate::ui::setup::VerificationId(2),
            verification_key: zeroize::Zeroizing::new("sk-current".into()),
            phase: crate::ui::setup::WizardPhase::Verifying,
            check1: crate::ui::setup::CheckStatus::Running,
            ..Default::default()
        };
        model.verification_scope = Some(capture_setup_scope(&model));

        assert!(!apply_setup_result(
            &mut model,
            crate::ui::setup::VerificationId(1),
            crate::ui::setup::SetupCheck::Connectivity,
            Ok(()),
        ));
        assert_eq!(model.phase, crate::ui::setup::WizardPhase::Verifying);
        assert_eq!(model.check1, crate::ui::setup::CheckStatus::Running);

        assert!(apply_setup_result(
            &mut model,
            crate::ui::setup::VerificationId(2),
            crate::ui::setup::SetupCheck::Connectivity,
            Ok(()),
        ));
        assert_eq!(model.check1, crate::ui::setup::CheckStatus::Ok);
    }

    #[test]
    fn cancel_and_reopen_invalidate_prior_verification_results() {
        let mut generation = 0;
        let mut cancelled = crate::ui::setup::SetupWizardModel::default();
        seed_setup_verification(&mut generation, &mut cancelled);
        let cancelled_id = cancelled.verification_id;
        invalidate_setup_verification(&mut generation, &mut cancelled);
        assert_ne!(cancelled.verification_id, cancelled_id);

        let mut reopened = crate::ui::setup::SetupWizardModel::default();
        seed_setup_verification(&mut generation, &mut reopened);
        reopened.provider = "anthropic".into();
        reopened.key = zeroize::Zeroizing::new("sk-reopened".into());
        reopened.verification_key = reopened.key.clone();
        reopened.verification_scope = Some(capture_setup_scope(&reopened));
        reopened.phase = crate::ui::setup::WizardPhase::Verifying;
        reopened.check1 = crate::ui::setup::CheckStatus::Running;
        assert!(!apply_setup_result(
            &mut reopened,
            cancelled_id,
            crate::ui::setup::SetupCheck::Connectivity,
            Ok(()),
        ));
        assert_eq!(reopened.check1, crate::ui::setup::CheckStatus::Running);
    }

    fn verified_model(
        key: &str,
        phase: crate::ui::setup::WizardPhase,
    ) -> crate::ui::setup::SetupWizardModel {
        let mut model = crate::ui::setup::SetupWizardModel {
            provider: "openai".into(),
            key: zeroize::Zeroizing::new(key.into()),
            storage: crate::ui::setup::Storage::Keychain,
            test_translation: true,
            phase,
            verification_id: crate::ui::setup::VerificationId(7),
            verification_key: zeroize::Zeroizing::new(key.into()),
            ..Default::default()
        };
        model.verification_scope = Some(capture_setup_scope(&model));
        model
    }

    #[test]
    fn edit_during_verification_invalidates_result_before_model_mutation() {
        let mut generation = 7;
        let mut model = verified_model("sk-verified", crate::ui::setup::WizardPhase::Verifying);
        let verification_id = model.verification_id;
        model.key = zeroize::Zeroizing::new("sk-edited".into());

        assert!(invalidate_changed_verification(&mut generation, &mut model));
        assert_eq!(model.phase, crate::ui::setup::WizardPhase::Entry);
        assert_ne!(model.verification_id, verification_id);
        assert!(!apply_setup_result(
            &mut model,
            verification_id,
            crate::ui::setup::SetupCheck::Connectivity,
            Ok(()),
        ));
        assert!(!crate::ui::setup::save_enabled(&model));
    }

    #[test]
    fn edit_after_done_cannot_save_an_unverified_model() {
        let mut generation = 7;
        let mut model = verified_model("sk-verified", crate::ui::setup::WizardPhase::Done);
        model.provider = "deepseek".into();

        assert!(invalidate_changed_verification(&mut generation, &mut model));
        assert_eq!(model.phase, crate::ui::setup::WizardPhase::Entry);
        assert!(!crate::ui::setup::save_enabled(&model));
        assert!(verified_key_for_save(&model).is_err());
    }

    #[test]
    fn environment_verification_resolves_the_profile_variable() {
        let variable = "OPENAI_API_KEY";
        let previous = std::env::var_os(variable);
        std::env::set_var(variable, "sk-from-env");
        let model = crate::ui::setup::SetupWizardModel {
            provider: "openai".into(),
            storage: crate::ui::setup::Storage::Env,
            ..Default::default()
        };
        let key = verification_key(&model).unwrap();
        if let Some(previous) = previous {
            std::env::set_var(variable, previous);
        } else {
            std::env::remove_var(variable);
        }
        assert_eq!(&*key, "sk-from-env");
        assert!(model.key.is_empty());
    }

    #[tokio::test]
    async fn provider_switch_sample_request_uses_selected_profile_model() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .and(body_json(serde_json::json!({
                "model": "gpt-4o-mini",
                "messages": [
                    {"role": "system", "content": "system"},
                    {"role": "user", "content": "Hello, world."}
                ]
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [{"message": {"content": "Hallo, Welt."}}]
            })))
            .expect(1)
            .mount(&server)
            .await;

        let cfg = Config::default();
        assert_eq!(cfg.provider.kind, "anthropic");
        let check_cfg =
            sample_check_config(&cfg, "openai", crate::ui::setup::default_base_url("openai"))
                .unwrap();
        assert_eq!(check_cfg.provider.kind, "openai");
        let provider = crate::llm::openai::OpenAiCompatibleProvider::new(
            crate::config::ProviderEndpoint::parse(&server.uri(), true).unwrap(),
            zeroize::Zeroizing::new("sk-test".into()),
            &check_cfg.provider.model,
            std::time::Duration::from_secs(10),
        )
        .unwrap();

        let result = provider.complete("system", "Hello, world.").await.unwrap();
        assert_eq!(result, "Hallo, Welt.");
    }
}
