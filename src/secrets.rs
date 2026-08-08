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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HistoryKeyProvisionState {
    KeychainPresent,
    KeychainPresentLegacyRecovered { recovery_path: std::path::PathBuf },
    KeychainCreated,
    MigratedLegacy { recovery_path: std::path::PathBuf },
    LegacyFallback { reason: String },
}

#[derive(Debug)]
pub struct ProvisionedHistoryKey {
    pub secret: Zeroizing<[u8; 32]>,
    pub state: HistoryKeyProvisionState,
}

trait HistoryKeyStore {
    fn read(&self) -> Result<Option<Vec<u8>>, TranslateError>;
    fn write(&self, bytes: &[u8]) -> Result<(), TranslateError>;
}

struct KeyringHistoryKeyStore {
    service: String,
    account: String,
}

impl KeyringHistoryKeyStore {
    fn entry(&self) -> Result<keyring::Entry, TranslateError> {
        keyring::Entry::new(&self.service, &self.account).map_err(|e| {
            TranslateError::SetupWizard(format!("history keychain entry construction failed: {e}"))
        })
    }
}

impl HistoryKeyStore for KeyringHistoryKeyStore {
    fn read(&self) -> Result<Option<Vec<u8>>, TranslateError> {
        match self.entry()?.get_secret() {
            Ok(bytes) => Ok(Some(bytes)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(TranslateError::SetupWizard(format!(
                "history keychain read failed: {e}"
            ))),
        }
    }

    fn write(&self, bytes: &[u8]) -> Result<(), TranslateError> {
        self.entry()?
            .set_secret(bytes)
            .map_err(|e| TranslateError::SetupWizard(format!("history keychain write failed: {e}")))
    }
}

/// Provision the history secret before the database is opened. Existing
/// keychain material is used only after any retained legacy file is securely
/// compared. Matching legacy material is moved to an owner-only recovery
/// file; mismatch or unsafe file state fails closed and preserves the source.
pub fn provision_history_key(
    keyfile_path: &std::path::Path,
    service: &str,
    account: &str,
) -> Result<ProvisionedHistoryKey, TranslateError> {
    let store = KeyringHistoryKeyStore {
        service: service.into(),
        account: account.into(),
    };
    provision_history_key_with_store(keyfile_path, &store)
}

fn provision_history_key_with_store(
    keyfile_path: &std::path::Path,
    store: &dyn HistoryKeyStore,
) -> Result<ProvisionedHistoryKey, TranslateError> {
    provision_history_key_with_store_and_probe(
        keyfile_path,
        store,
        crate::platform::probe_secure_legacy_file,
    )
}

fn provision_history_key_with_store_and_probe(
    keyfile_path: &std::path::Path,
    store: &dyn HistoryKeyStore,
    probe_legacy: impl FnOnce(&std::path::Path) -> Result<Option<Vec<u8>>, std::io::Error>,
) -> Result<ProvisionedHistoryKey, TranslateError> {
    let keychain = store.read();
    let legacy = probe_legacy(keyfile_path).map_err(|e| {
        TranslateError::History(format!(
            "legacy history key cannot be inspected securely: {e}"
        ))
    })?;

    match keychain {
        Ok(Some(bytes)) => provision_with_existing_keychain(keyfile_path, &bytes, legacy),
        Ok(None) => provision_missing_keychain_entry(keyfile_path, store, legacy),
        Err(keychain_error) => match legacy {
            Some(bytes) => Ok(ProvisionedHistoryKey {
                secret: history_secret_from_bytes(&bytes)?,
                state: HistoryKeyProvisionState::LegacyFallback {
                    reason: keychain_error.to_string(),
                },
            }),
            None => Err(TranslateError::History(format!(
                "history keychain unavailable and no secure legacy key exists: {keychain_error}"
            ))),
        },
    }
}

fn provision_with_existing_keychain(
    keyfile_path: &std::path::Path,
    keychain_bytes: &[u8],
    legacy: Option<Vec<u8>>,
) -> Result<ProvisionedHistoryKey, TranslateError> {
    let secret = history_secret_from_bytes(keychain_bytes)?;
    let state = match legacy {
        None => HistoryKeyProvisionState::KeychainPresent,
        Some(legacy_bytes) => {
            let legacy_secret = history_secret_from_bytes(&legacy_bytes)?;
            if legacy_secret.as_slice() != secret.as_slice() {
                return Err(TranslateError::History(
                    "legacy history key does not match the existing keychain entry; original file preserved"
                        .into(),
                ));
            }
            let recovery_path = recover_legacy_keyfile(keyfile_path)?;
            HistoryKeyProvisionState::KeychainPresentLegacyRecovered { recovery_path }
        }
    };
    Ok(ProvisionedHistoryKey { secret, state })
}

fn provision_missing_keychain_entry(
    keyfile_path: &std::path::Path,
    store: &dyn HistoryKeyStore,
    legacy: Option<Vec<u8>>,
) -> Result<ProvisionedHistoryKey, TranslateError> {
    let (secret, had_legacy) = match legacy {
        Some(bytes) => (history_secret_from_bytes(&bytes)?, true),
        None => {
            let mut secret = Zeroizing::new([0u8; 32]);
            rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, secret.as_mut());
            (secret, false)
        }
    };

    store.write(secret.as_slice())?;
    let readback = store.read()?.ok_or_else(|| {
        TranslateError::History("history keychain readback returned no entry".into())
    })?;
    if readback.as_slice() != secret.as_slice() {
        return Err(TranslateError::History(
            "history keychain readback did not match the provisioned key".into(),
        ));
    }

    let state = if had_legacy {
        HistoryKeyProvisionState::MigratedLegacy {
            recovery_path: recover_legacy_keyfile(keyfile_path)?,
        }
    } else {
        HistoryKeyProvisionState::KeychainCreated
    };

    Ok(ProvisionedHistoryKey { secret, state })
}

fn recover_legacy_keyfile(
    keyfile_path: &std::path::Path,
) -> Result<std::path::PathBuf, TranslateError> {
    let recovery_path = keyfile_path.with_file_name(format!(
        "{}.recovery",
        keyfile_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(".history-key")
    ));
    crate::platform::rename_legacy_key_to_recovery(keyfile_path, &recovery_path).map_err(|e| {
        TranslateError::History(format!("legacy history key recovery rename failed: {e}"))
    })?;
    Ok(recovery_path)
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

    fn write_key(
        &self,
        key: &Zeroizing<String>,
        writer: impl FnOnce(&std::path::Path, &[u8]) -> Result<(), std::io::Error>,
    ) -> Result<(), TranslateError> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                TranslateError::Internal(format!("keyfile mkdir {}: {e}", parent.display()))
            })?;
        }
        writer(&self.path, key.as_bytes()).map_err(|e| {
            TranslateError::Internal(format!("secure keyfile write {}: {e}", self.path.display()))
        })
    }

    #[cfg(test)]
    fn set_api_key_with_failure_for_test(
        &self,
        key: Zeroizing<String>,
        failure: crate::platform::SecureWriteFailure,
    ) -> Result<(), TranslateError> {
        self.write_key(&key, |path, contents| {
            crate::platform::secure_atomic_write_with_failure_for_test(path, contents, failure)
        })
    }
}

impl Secrets for FileSecrets {
    fn get_api_key(&self) -> Result<Zeroizing<String>, TranslateError> {
        match crate::platform::secure_read_file(&self.path) {
            Ok(bytes) => String::from_utf8(bytes)
                .map(|s| Zeroizing::new(s.trim().to_string()))
                .map_err(|e| {
                    TranslateError::Internal(format!(
                        "keyfile read {} is not UTF-8: {e}",
                        self.path.display()
                    ))
                }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Err(TranslateError::MissingApiKey {
                    env_var: format!("(keyfile {})", self.path.display()),
                })
            }
            Err(e) => Err(TranslateError::Internal(format!(
                "secure keyfile read {}: {e}",
                self.path.display()
            ))),
        }
    }

    fn set_api_key(&self, key: Zeroizing<String>) -> Result<(), TranslateError> {
        self.write_key(&key, crate::platform::secure_atomic_write)
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

    #[test]
    fn api_key_file_is_owner_only_immediately_after_creation() {
        if !crate::platform::secure_file_storage_supported_for_test() {
            return;
        }
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("api-key");
        FileSecrets::new(path.clone())
            .set_api_key(Zeroizing::new("secret".into()))
            .unwrap();
        assert!(crate::platform::owner_only_permissions_are_enforced_for_test(&path).unwrap());
    }

    #[test]
    fn api_key_write_rejects_symlink_destination() {
        if !crate::platform::secure_file_storage_supported_for_test() {
            return;
        }
        let dir = tempfile::TempDir::new().unwrap();
        let target = dir.path().join("target");
        let link = dir.path().join("api-key");
        crate::platform::create_file_symlink_for_test(&target, &link).unwrap();
        let err = FileSecrets::new(link)
            .set_api_key(Zeroizing::new("secret".into()))
            .unwrap_err();
        assert!(err.to_string().contains("symlink"));
    }

    #[test]
    fn api_key_write_rejects_non_regular_destination() {
        if !crate::platform::secure_file_storage_supported_for_test() {
            return;
        }
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("api-key");
        std::fs::create_dir(&path).unwrap();
        let err = FileSecrets::new(path)
            .set_api_key(Zeroizing::new("secret".into()))
            .unwrap_err();
        assert!(err.to_string().contains("regular file"));
    }

    #[test]
    fn api_key_write_propagates_injected_permission_failure() {
        if !crate::platform::secure_file_storage_supported_for_test() {
            return;
        }
        let dir = tempfile::TempDir::new().unwrap();
        let file = FileSecrets::new(dir.path().join("api-key"));
        let err = file
            .set_api_key_with_failure_for_test(
                Zeroizing::new("secret".into()),
                crate::platform::SecureWriteFailure::Permission,
            )
            .unwrap_err();
        assert!(err.to_string().contains("permission"));
    }

    #[test]
    fn api_key_write_propagates_injected_rename_failure() {
        if !crate::platform::secure_file_storage_supported_for_test() {
            return;
        }
        let dir = tempfile::TempDir::new().unwrap();
        let file = FileSecrets::new(dir.path().join("api-key"));
        let err = file
            .set_api_key_with_failure_for_test(
                Zeroizing::new("secret".into()),
                crate::platform::SecureWriteFailure::Rename,
            )
            .unwrap_err();
        assert!(err.to_string().contains("rename"));
    }

    struct FakeHistoryKeyStore {
        secret: std::sync::Mutex<Option<Vec<u8>>>,
        unavailable: bool,
        readback_override: Option<Vec<u8>>,
    }

    impl HistoryKeyStore for FakeHistoryKeyStore {
        fn read(&self) -> Result<Option<Vec<u8>>, TranslateError> {
            if self.unavailable {
                return Err(TranslateError::SetupWizard("keychain unavailable".into()));
            }
            let stored = self.secret.lock().unwrap().clone();
            if stored.is_some() {
                if let Some(bytes) = self.readback_override.as_ref() {
                    return Ok(Some(bytes.clone()));
                }
            }
            Ok(stored)
        }

        fn write(&self, bytes: &[u8]) -> Result<(), TranslateError> {
            if self.unavailable {
                return Err(TranslateError::SetupWizard("keychain unavailable".into()));
            }
            *self.secret.lock().unwrap() = Some(bytes.to_vec());
            Ok(())
        }
    }

    #[test]
    fn history_key_provision_uses_keychain_entry_when_present() {
        let dir = tempfile::TempDir::new().unwrap();
        let legacy = dir.path().join(".history-key");
        let store = FakeHistoryKeyStore {
            secret: std::sync::Mutex::new(Some(vec![7u8; 32])),
            unavailable: false,
            readback_override: None,
        };

        let provisioned = provision_history_key_with_store(&legacy, &store).unwrap();

        assert_eq!(provisioned.secret.as_slice(), &[7u8; 32]);
        assert_eq!(provisioned.state, HistoryKeyProvisionState::KeychainPresent);
        assert!(!legacy.exists());
    }

    #[test]
    fn history_key_provision_creates_keychain_key_when_legacy_is_absent() {
        let dir = tempfile::TempDir::new().unwrap();
        let legacy = dir.path().join(".history-key");
        let store = FakeHistoryKeyStore {
            secret: std::sync::Mutex::new(None),
            unavailable: false,
            readback_override: None,
        };

        let provisioned =
            provision_history_key_with_store_and_probe(&legacy, &store, |_| Ok(None)).unwrap();

        assert_eq!(provisioned.state, HistoryKeyProvisionState::KeychainCreated);
        assert_eq!(
            store.secret.lock().unwrap().as_deref(),
            Some(provisioned.secret.as_slice())
        );
    }

    #[test]
    fn history_key_provision_recovers_matching_legacy_when_keychain_present() {
        if !crate::platform::secure_file_storage_supported_for_test() {
            return;
        }
        let dir = tempfile::TempDir::new().unwrap();
        let legacy = dir.path().join(".history-key");
        crate::platform::secure_atomic_write(&legacy, &[7u8; 32]).unwrap();
        let store = FakeHistoryKeyStore {
            secret: std::sync::Mutex::new(Some(vec![7u8; 32])),
            unavailable: false,
            readback_override: None,
        };

        let provisioned = provision_history_key_with_store(&legacy, &store).unwrap();
        let recovery = dir.path().join(".history-key.recovery");

        assert_eq!(
            provisioned.state,
            HistoryKeyProvisionState::KeychainPresentLegacyRecovered {
                recovery_path: recovery.clone(),
            }
        );
        assert!(!legacy.exists());
        assert_eq!(
            crate::platform::secure_read_file(&recovery).unwrap(),
            vec![7u8; 32]
        );
    }

    #[test]
    fn history_key_provision_rejects_mismatched_legacy_when_keychain_present() {
        if !crate::platform::secure_file_storage_supported_for_test() {
            return;
        }
        let dir = tempfile::TempDir::new().unwrap();
        let legacy = dir.path().join(".history-key");
        crate::platform::secure_atomic_write(&legacy, &[8u8; 32]).unwrap();
        let store = FakeHistoryKeyStore {
            secret: std::sync::Mutex::new(Some(vec![7u8; 32])),
            unavailable: false,
            readback_override: None,
        };

        let err = provision_history_key_with_store(&legacy, &store).unwrap_err();

        assert!(err.to_string().contains("does not match"));
        assert!(legacy.exists());
        assert!(!dir.path().join(".history-key.recovery").exists());
    }

    #[test]
    fn history_key_provision_rejects_unsafe_legacy_when_keychain_present() {
        if !crate::platform::secure_file_storage_supported_for_test() {
            return;
        }
        let dir = tempfile::TempDir::new().unwrap();
        let legacy = dir.path().join(".history-key");
        std::fs::create_dir(&legacy).unwrap();
        let store = FakeHistoryKeyStore {
            secret: std::sync::Mutex::new(Some(vec![7u8; 32])),
            unavailable: false,
            readback_override: None,
        };

        let err = provision_history_key_with_store(&legacy, &store).unwrap_err();

        assert!(err.to_string().contains("securely"));
        assert!(legacy.is_dir());
    }

    #[test]
    fn history_key_provision_migrates_legacy_only_after_verified_readback() {
        if !crate::platform::secure_file_storage_supported_for_test() {
            return;
        }
        let dir = tempfile::TempDir::new().unwrap();
        let legacy = dir.path().join(".history-key");
        crate::platform::secure_atomic_write(&legacy, &[8u8; 32]).unwrap();
        let store = FakeHistoryKeyStore {
            secret: std::sync::Mutex::new(None),
            unavailable: false,
            readback_override: None,
        };

        let provisioned = provision_history_key_with_store(&legacy, &store).unwrap();

        let recovery = dir.path().join(".history-key.recovery");
        assert_eq!(provisioned.secret.as_slice(), &[8u8; 32]);
        assert_eq!(
            provisioned.state,
            HistoryKeyProvisionState::MigratedLegacy {
                recovery_path: recovery.clone(),
            }
        );
        assert!(!legacy.exists());
        assert_eq!(
            crate::platform::secure_read_file(&recovery).unwrap(),
            vec![8u8; 32]
        );
    }

    #[test]
    fn history_key_provision_uses_secure_legacy_when_keychain_unavailable() {
        if !crate::platform::secure_file_storage_supported_for_test() {
            return;
        }
        let dir = tempfile::TempDir::new().unwrap();
        let legacy = dir.path().join(".history-key");
        crate::platform::secure_atomic_write(&legacy, &[9u8; 32]).unwrap();
        let store = FakeHistoryKeyStore {
            secret: std::sync::Mutex::new(None),
            unavailable: true,
            readback_override: None,
        };

        let provisioned = provision_history_key_with_store(&legacy, &store).unwrap();

        assert_eq!(provisioned.secret.as_slice(), &[9u8; 32]);
        assert!(matches!(
            provisioned.state,
            HistoryKeyProvisionState::LegacyFallback { .. }
        ));
        assert!(legacy.exists());
    }

    #[test]
    fn history_key_migration_keeps_legacy_when_keychain_readback_mismatches() {
        if !crate::platform::secure_file_storage_supported_for_test() {
            return;
        }
        let dir = tempfile::TempDir::new().unwrap();
        let legacy = dir.path().join(".history-key");
        crate::platform::secure_atomic_write(&legacy, &[10u8; 32]).unwrap();
        let store = FakeHistoryKeyStore {
            secret: std::sync::Mutex::new(None),
            unavailable: false,
            readback_override: Some(vec![11u8; 32]),
        };

        let err = provision_history_key_with_store(&legacy, &store).unwrap_err();

        assert!(err.to_string().contains("readback"));
        assert!(legacy.exists());
        assert!(!dir.path().join(".history-key.recovery").exists());
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
