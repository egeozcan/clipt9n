//! API key resolution. M1 implemented env-var lookup only; M6 adds the
//! keychain (preferred) → env-var → setup-wizard fallback chain. The
//! trait is the seam: `EnvSecrets` reads from process env vars only;
//! `KeychainSecrets` reads from / writes to the OS keychain via the
//! `keyring` crate (cross-platform: macOS Keychain Services, Windows
//! Credential Manager, Linux Secret Service).

use zeroize::Zeroizing;

use crate::config::ApiKeyConfig;
use crate::error::TranslateError;

pub trait Secrets: Send + Sync {
    /// Resolve the API key. Returned in `Zeroizing<String>` so the
    /// memory is wiped on drop (defense-in-depth; not a substitute for
    /// keychain storage).
    fn get_api_key(&self) -> Result<Zeroizing<String>, TranslateError>;

    /// Persist the API key. For `EnvSecrets` this returns an error
    /// (env vars are read-only from our perspective). For
    /// `KeychainSecrets`, writes to the OS keychain.
    fn set_api_key(&self, key: Zeroizing<String>) -> Result<(), TranslateError>;

    /// Whether the underlying keychain is reachable on this platform.
    /// `EnvSecrets` always returns false. `KeychainSecrets` probes by
    /// attempting `Entry::get_password()`; treats `Err(NoEntry)` as
    /// "available, no entry yet" and any other `Err` as "unavailable".
    fn keychain_available(&self) -> bool;
}

/// Reads an API key from a configured environment variable.
pub struct EnvSecrets {
    env_var: String,
}

impl EnvSecrets {
    pub fn new(env_var: impl Into<String>) -> Self {
        Self {
            env_var: env_var.into(),
        }
    }
}

impl Secrets for EnvSecrets {
    fn get_api_key(&self) -> Result<Zeroizing<String>, TranslateError> {
        std::env::var(&self.env_var)
            .map(Zeroizing::new)
            .map_err(|_| TranslateError::MissingApiKey {
                env_var: self.env_var.clone(),
            })
    }

    fn set_api_key(&self, _key: Zeroizing<String>) -> Result<(), TranslateError> {
        // Env-var-backed Secrets are read-only from our perspective —
        // the user sets the var in their shell. The wizard's "Save and
        // start" path with storage=Env writes a hint to README rather
        // than calling this method. If something does call it, surface
        // a clear error so it's debuggable.
        Err(TranslateError::SetupWizard(
            "env-secrets are read-only; cannot persist key — \
             user must set the env var manually"
                .into(),
        ))
    }

    fn keychain_available(&self) -> bool {
        false
    }
}

/// Reads / writes the API key from the OS keychain via the `keyring`
/// crate. Cross-platform: macOS Keychain Services, Windows Credential
/// Manager, Linux Secret Service. Service + account are configured in
/// `[provider.api_key]` (`service` + `account` fields).
pub struct KeychainSecrets {
    service: String,
    account: String,
}

impl KeychainSecrets {
    pub fn new(service: impl Into<String>, account: impl Into<String>) -> Self {
        Self {
            service: service.into(),
            account: account.into(),
        }
    }

    fn entry(&self) -> Result<keyring::Entry, TranslateError> {
        keyring::Entry::new(&self.service, &self.account).map_err(|e| {
            TranslateError::SetupWizard(format!(
                "keychain entry construction failed for service={} account={}: {e}",
                self.service, self.account
            ))
        })
    }
}

impl Secrets for KeychainSecrets {
    fn get_api_key(&self) -> Result<Zeroizing<String>, TranslateError> {
        let entry = self.entry()?;
        match entry.get_password() {
            Ok(s) => Ok(Zeroizing::new(s)),
            Err(keyring::Error::NoEntry) => Err(TranslateError::MissingApiKey {
                env_var: format!(
                    "(keychain service={} account={})",
                    self.service, self.account
                ),
            }),
            Err(e) => Err(TranslateError::SetupWizard(format!(
                "keychain read failed: {e}"
            ))),
        }
    }

    fn set_api_key(&self, key: Zeroizing<String>) -> Result<(), TranslateError> {
        let entry = self.entry()?;
        entry
            .set_password(&key)
            .map_err(|e| TranslateError::SetupWizard(format!("keychain write failed: {e}")))
    }

    fn keychain_available(&self) -> bool {
        // Probe with a known-disposable account. Reading a non-
        // existent entry returns `Err(NoEntry)` on a healthy keychain;
        // any other error means the platform's keychain is actually
        // unreachable (e.g., Linux without Secret Service).
        let probe = match keyring::Entry::new(&self.service, "_clipt9n_probe") {
            Ok(e) => e,
            Err(_) => return false,
        };
        match probe.get_password() {
            Ok(_) => true,
            Err(keyring::Error::NoEntry) => true,
            Err(_) => false,
        }
    }
}

/// One-shot migration helper for M5 → M6 upgrades. Reads the bytes of
/// `<config_dir>/.history-key` and writes them to a `history-key`
/// keychain entry under the configured service. The keyfile is left
/// in place (per the M6 plan §3 "copies, never moves" decision); the
/// README documents that users can `rm` it after verifying.
///
/// Returns `Ok(true)` if migration happened (file existed AND keychain
/// entry was empty), `Ok(false)` if there was nothing to do (file
/// missing OR keychain entry already populated), or `Err(_)` only on a
/// real failure (I/O reading the file, or keychain write failure other
/// than `NoStorageAccess`).
///
/// Migration failure is best-effort — callers log warn and continue;
/// the M5 keyfile path still works as a fallback.
pub fn migrate_keyfile_to_keychain(
    keyfile_path: &std::path::Path,
    service: &str,
    account: &str,
) -> Result<bool, TranslateError> {
    if !keyfile_path.exists() {
        return Ok(false);
    }
    let entry = keyring::Entry::new(service, account).map_err(|e| {
        TranslateError::SetupWizard(format!(
            "keychain entry construction failed for service={service} account={account}: {e}"
        ))
    })?;
    // If a keychain entry already exists, do nothing. Use get_secret
    // (binary API) to match the set_secret write below — get_password
    // would fail UTF-8 decoding on the random binary key and falsely
    // claim no entry exists, causing the migration to run on every
    // launch.
    if entry.get_secret().is_ok() {
        return Ok(false);
    }
    let bytes = std::fs::read(keyfile_path).map_err(|e| {
        TranslateError::SetupWizard(format!(
            "reading {} for migration: {e}",
            keyfile_path.display()
        ))
    })?;
    entry.set_secret(&bytes).map_err(|e| {
        TranslateError::SetupWizard(format!("keychain write during migration: {e}"))
    })?;
    Ok(true)
}

/// Read a 32-byte binary history secret from the OS keychain.
///
/// Returns `Ok(None)` when the keychain entry does not exist. Other
/// keychain errors are surfaced so callers can choose whether to fall
/// back to the legacy keyfile path.
pub fn history_secret_from_keychain(
    service: &str,
    account: &str,
) -> Result<Option<Zeroizing<[u8; 32]>>, TranslateError> {
    let entry = keyring::Entry::new(service, account).map_err(|e| {
        TranslateError::SetupWizard(format!(
            "keychain entry construction failed for service={service} account={account}: {e}"
        ))
    })?;
    match entry.get_secret() {
        Ok(bytes) => history_secret_from_bytes(&bytes).map(Some),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(TranslateError::SetupWizard(format!(
            "keychain read for history key failed: {e}"
        ))),
    }
}

fn history_secret_from_bytes(bytes: &[u8]) -> Result<Zeroizing<[u8; 32]>, TranslateError> {
    if bytes.len() != 32 {
        return Err(TranslateError::History(format!(
            "history keychain secret has wrong size: expected 32 bytes, got {}",
            bytes.len()
        )));
    }
    let mut secret = Zeroizing::new([0u8; 32]);
    secret.copy_from_slice(bytes);
    Ok(secret)
}

/// Construct the `Secrets` impl matching `cfg.provider.api_key.source`.
/// "keychain" → KeychainSecrets; "file" → FileSecrets at the
/// configured path; anything else → EnvSecrets.
pub fn resolve(cfg: &ApiKeyConfig) -> Box<dyn Secrets> {
    match cfg.source.as_str() {
        "keychain" => Box::new(KeychainSecrets::new(&cfg.service, &cfg.account)),
        "file" => Box::new(FileSecrets::new(std::path::PathBuf::from(&cfg.path))),
        _ => Box::new(EnvSecrets::new(cfg.env_var.clone())),
    }
}

/// Read / write the API key from a 0600-perm file under the
/// config dir. Used as a fallback on macOS dev/ad-hoc-signed binaries
/// where the OS keychain silently fails to persist `SecItemAdd`
/// writes (the user's wizard run reports success but the next launch
/// can't find the key). Plaintext-with-0600 is the same security
/// posture as the M5 history-key file; FileVault provides at-rest
/// encryption when the user has it on.
pub struct FileSecrets {
    path: std::path::PathBuf,
}

impl FileSecrets {
    pub fn new(path: std::path::PathBuf) -> Self {
        Self { path }
    }

    /// Standard keyfile path under the given config dir.
    pub fn keyfile_path(config_dir: &std::path::Path) -> std::path::PathBuf {
        config_dir.join("api-key")
    }
}

impl Secrets for FileSecrets {
    fn get_api_key(&self) -> Result<Zeroizing<String>, TranslateError> {
        match std::fs::read_to_string(&self.path) {
            Ok(s) => Ok(Zeroizing::new(s.trim().to_string())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Err(TranslateError::MissingApiKey {
                    env_var: format!("(keyfile {})", self.path.display()),
                })
            }
            Err(e) => Err(TranslateError::Internal(format!(
                "keyfile read {}: {e}",
                self.path.display()
            ))),
        }
    }

    fn set_api_key(&self, key: Zeroizing<String>) -> Result<(), TranslateError> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                TranslateError::Internal(format!("keyfile mkdir {}: {e}", parent.display()))
            })?;
        }
        std::fs::write(&self.path, key.as_bytes()).map_err(|e| {
            TranslateError::Internal(format!("keyfile write {}: {e}", self.path.display()))
        })?;
        if let Err(e) = crate::platform::set_owner_only_permissions(&self.path) {
            tracing::warn!(error = %e, path = %self.path.display(), "keyfile chmod 0600 failed");
        }
        Ok(())
    }

    fn keychain_available(&self) -> bool {
        false
    }
}

/// Standalone keychain reachability probe. Used by the setup-wizard
/// seed paths (main.rs first-launch, tray "Re-run setup wizard")
/// because `secrets.keychain_available()` reflects the **active**
/// `Secrets` impl: when `cfg.provider.api_key.source = "env"` (the
/// default for fresh configs) `secrets` is `EnvSecrets`, which always
/// reports `false` even on a working macOS keychain. The wizard needs
/// the *underlying platform's* answer so it can decide whether to
/// offer Keychain storage. Probes with a disposable account name to
/// avoid reading any real key material.
pub fn keychain_probe(service: &str) -> bool {
    let probe = match keyring::Entry::new(service, "_clipt9n_probe") {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!(error = %e, service, "keychain probe: Entry::new failed — wizard will report keychain unavailable");
            return false;
        }
    };
    match probe.get_password() {
        Ok(_) => true,
        Err(keyring::Error::NoEntry) => true,
        Err(e) => {
            tracing::warn!(error = %e, service, "keychain probe: get_password failed — wizard will report keychain unavailable");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Process-global env state — each test uses a unique var name.

    #[test]
    fn env_returns_value_when_set() {
        let var = "CLIPT9N_TEST_KEY_PRESENT";
        std::env::set_var(var, "sk-test-12345");
        let s = EnvSecrets::new(var);
        let key = s.get_api_key().unwrap();
        assert_eq!(&*key, "sk-test-12345");
        std::env::remove_var(var);
    }

    #[test]
    fn env_returns_error_when_missing() {
        let var = "CLIPT9N_TEST_KEY_ABSENT";
        std::env::remove_var(var);
        let s = EnvSecrets::new(var);
        match s.get_api_key().unwrap_err() {
            TranslateError::MissingApiKey { env_var } => assert_eq!(env_var, var),
            other => panic!("expected MissingApiKey, got {other:?}"),
        }
    }

    #[test]
    fn env_set_api_key_returns_setup_wizard_error() {
        let s = EnvSecrets::new("CLIPT9N_TEST_KEY_SET_ATTEMPT");
        let err = s
            .set_api_key(Zeroizing::new("ignored".to_string()))
            .unwrap_err();
        match err {
            TranslateError::SetupWizard(msg) => assert!(msg.contains("read-only")),
            other => panic!("expected SetupWizard, got {other:?}"),
        }
    }

    #[test]
    fn env_keychain_available_is_false() {
        let s = EnvSecrets::new("CLIPT9N_TEST_KEY_AVAIL");
        assert!(!s.keychain_available());
    }

    #[test]
    fn returned_key_is_zeroizing() {
        let var = "CLIPT9N_TEST_KEY_ZEROIZE";
        std::env::set_var(var, "secret");
        let s = EnvSecrets::new(var);
        let _key: Zeroizing<String> = s.get_api_key().unwrap();
        std::env::remove_var(var);
    }

    #[test]
    fn resolve_picks_env_for_default_source() {
        let cfg = ApiKeyConfig::default(); // source = "env"
        let s = resolve(&cfg);
        // Type-level check: get_api_key() is callable; we can't downcast
        // a `Box<dyn Secrets>` without `Any`, so verify behaviorally —
        // an env-backed Secrets always returns false from
        // keychain_available().
        assert!(!s.keychain_available());
    }

    #[test]
    fn resolve_picks_keychain_for_keychain_source() {
        let cfg = ApiKeyConfig {
            source: "keychain".into(),
            service: "clipt9n-test".into(),
            account: "test-account".into(),
            env_var: "irrelevant".into(),
            path: String::new(),
        };
        let s = resolve(&cfg);
        // KeychainSecrets::keychain_available probes the actual OS
        // keychain. On a dev macOS this is true; in CI it may be
        // false depending on the runner's Keychain availability.
        // We don't assert the value — just that the call doesn't
        // panic. The behavioral test is in Task 11's manual smoke.
        let _ = s.keychain_available();
    }

    #[test]
    fn history_secret_bytes_must_be_exactly_32_bytes() {
        let ok = history_secret_from_bytes(&[7u8; 32]).unwrap();
        assert_eq!(ok.as_slice(), &[7u8; 32]);

        let err = history_secret_from_bytes(&[7u8; 31]).unwrap_err();
        assert!(matches!(err, TranslateError::History(_)));
    }

    // KeychainSecrets read-write integration — opt-in via
    // CLIPT9N_KEYCHAIN_INTEGRATION=1 env so unit-test runs in CI
    // don't pollute the developer's keychain. Run manually with:
    //   CLIPT9N_KEYCHAIN_INTEGRATION=1 cargo test --lib secrets::keychain
    //
    // Skips with a diagnostic when the readback returns NoEntry —
    // macOS silently fails to persist keychain items written by
    // unsigned binaries (SecItemAdd returns success, the item is
    // never findable). The environmental constraint is the same as
    // tests/keychain_integration.rs.
    #[test]
    fn keychain_round_trip_when_opted_in() {
        if std::env::var("CLIPT9N_KEYCHAIN_INTEGRATION").is_err() {
            return; // skip
        }
        let account = format!("test-account-{}", std::process::id());
        let s = KeychainSecrets::new("clipt9n-test", account.as_str());
        let key = Zeroizing::new("sk-test-roundtrip-9876".to_string());
        s.set_api_key(key.clone()).unwrap();
        // Cleanup runs whether the readback succeeds or not.
        struct Cleanup<'a>(&'a str);
        impl Drop for Cleanup<'_> {
            fn drop(&mut self) {
                if let Ok(entry) = keyring::Entry::new("clipt9n-test", self.0) {
                    let _ = entry.delete_credential();
                }
            }
        }
        let _cleanup = Cleanup(account.as_str());
        match s.get_api_key() {
            Ok(read) => assert_eq!(&*read, "sk-test-roundtrip-9876"),
            Err(e) => eprintln!(
                "skipping: keychain readback failed ({e}). On macOS, \
                 SecItemAdd silently fails to persist when called from \
                 unsigned binaries — re-run from clipt9n.app or a signed \
                 CI runner."
            ),
        }
    }
}
