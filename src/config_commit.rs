//! Transactional configuration and credential commit flow.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use zeroize::Zeroizing;

use crate::config::Config;
use crate::error::TranslateError;
use crate::platform::Platform;
use crate::secrets::Secrets;

#[derive(Debug)]
pub enum Credential {
    Keep,
    Store(Zeroizing<String>),
}

pub struct CommittedConfig {
    pub config: Config,
}

pub trait AtomicConfigStore {
    fn replace(&self, candidate: &Config) -> Result<(), TranslateError>;
}

pub trait CredentialStore {
    /// Opaque, zeroizing state sufficient to undo a completed credential
    /// write if the following config replacement fails.
    type Rollback;

    fn store(
        &self,
        candidate: &mut Config,
        credential: Credential,
    ) -> Result<Self::Rollback, TranslateError>;

    fn rollback(&self, rollback: Self::Rollback) -> Result<(), TranslateError>;
}

pub struct ConfigCommitter<F, S> {
    fs: F,
    credentials: S,
}

impl<F, S> ConfigCommitter<F, S>
where
    F: AtomicConfigStore,
    S: CredentialStore,
{
    pub fn new(fs: F, credentials: S) -> Self {
        Self { fs, credentials }
    }

    pub fn commit(
        &self,
        mut candidate: Config,
        credential: Credential,
    ) -> Result<CommittedConfig, TranslateError> {
        candidate.validate()?;
        let rollback = self.credentials.store(&mut candidate, credential)?;
        if let Err(validation_error) = candidate.validate() {
            return match self.credentials.rollback(rollback) {
                Ok(()) => Err(validation_error),
                Err(rollback_error) => Err(TranslateError::Config(format!(
                    "credential-adjusted config validation failed ({validation_error}); restoring the previous credential also failed ({rollback_error})"
                ))),
            };
        }
        if let Err(commit_error) = self.fs.replace(&candidate) {
            return match self.credentials.rollback(rollback) {
                Ok(()) => Err(commit_error),
                Err(rollback_error) => Err(TranslateError::Config(format!(
                    "config replacement failed ({commit_error}); restoring the previous credential also failed ({rollback_error})"
                ))),
            };
        }
        Ok(CommittedConfig { config: candidate })
    }
}

#[derive(Debug, Clone)]
pub struct DiskAtomicConfig {
    path: PathBuf,
}

impl DiskAtomicConfig {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    fn replace_with_platform<P: Platform + ?Sized>(
        &self,
        candidate: &Config,
        platform: &P,
    ) -> Result<(), TranslateError> {
        let contents = toml::to_string_pretty(candidate)
            .map_err(|e| TranslateError::Config(format!("serializing config: {e}")))?;
        let parent = self
            .path
            .parent()
            .ok_or_else(|| TranslateError::Config("config path has no parent".into()))?;
        std::fs::create_dir_all(parent)
            .map_err(|e| TranslateError::Config(format!("creating {}: {e}", parent.display())))?;

        let previous = match std::fs::read(&self.path) {
            Ok(bytes) => Some(bytes),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => {
                return Err(TranslateError::Config(format!(
                    "snapshotting existing config {}: {error}",
                    self.path.display()
                )))
            }
        };
        let (temp_path, mut temp) = create_same_directory_temp(parent, &self.path)?;
        let staged = (|| {
            temp.write_all(contents.as_bytes()).map_err(|e| {
                TranslateError::Config(format!("writing {}: {e}", temp_path.display()))
            })?;
            temp.flush().map_err(|e| {
                TranslateError::Config(format!("flushing {}: {e}", temp_path.display()))
            })?;
            temp.sync_all().map_err(|e| {
                TranslateError::Config(format!("syncing {}: {e}", temp_path.display()))
            })?;
            platform.replace_file(&temp_path, &self.path).map_err(|e| {
                TranslateError::Config(format!(
                    "replacing {} with {}: {e}",
                    self.path.display(),
                    temp_path.display()
                ))
            })?;
            if let Err(sync_error) = platform.sync_parent_directory(parent) {
                return match restore_visible_config(
                    platform,
                    parent,
                    &self.path,
                    previous.as_deref(),
                ) {
                    Ok(()) => Err(TranslateError::Config(format!(
                        "syncing config directory {}: {sync_error}; previous config was restored",
                        parent.display()
                    ))),
                    Err(restore_error) => Err(TranslateError::Config(format!(
                        "syncing config directory {}: {sync_error}; restoring the previous config also failed ({restore_error})",
                        parent.display()
                    ))),
                };
            }
            Ok(())
        })();
        if staged.is_err() {
            let _ = std::fs::remove_file(&temp_path);
        }
        staged
    }
}

static TEMP_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

impl AtomicConfigStore for DiskAtomicConfig {
    fn replace(&self, candidate: &Config) -> Result<(), TranslateError> {
        self.replace_with_platform(candidate, &crate::platform::current())
    }
}

fn restore_visible_config<P: Platform + ?Sized>(
    platform: &P,
    parent: &Path,
    destination: &Path,
    previous: Option<&[u8]>,
) -> Result<(), TranslateError> {
    let Some(previous) = previous else {
        std::fs::remove_file(destination).map_err(|error| {
            TranslateError::Config(format!(
                "removing newly-created config {} during rollback: {error}",
                destination.display()
            ))
        })?;
        return platform.sync_parent_directory(parent).map_err(|error| {
            TranslateError::Config(format!(
                "syncing config directory {} after rollback: {error}",
                parent.display()
            ))
        });
    };

    let (temp_path, mut temp) = create_same_directory_temp(parent, destination)?;
    let restored = (|| {
        temp.write_all(previous).map_err(|error| {
            TranslateError::Config(format!(
                "writing config rollback {}: {error}",
                temp_path.display()
            ))
        })?;
        temp.flush().map_err(|error| {
            TranslateError::Config(format!(
                "flushing config rollback {}: {error}",
                temp_path.display()
            ))
        })?;
        temp.sync_all().map_err(|error| {
            TranslateError::Config(format!(
                "syncing config rollback {}: {error}",
                temp_path.display()
            ))
        })?;
        platform
            .replace_file(&temp_path, destination)
            .map_err(|error| {
                TranslateError::Config(format!(
                    "restoring previous config {}: {error}",
                    destination.display()
                ))
            })?;
        platform.sync_parent_directory(parent).map_err(|error| {
            TranslateError::Config(format!(
                "syncing config directory {} after rollback: {error}",
                parent.display()
            ))
        })
    })();
    if restored.is_err() {
        let _ = std::fs::remove_file(&temp_path);
    }
    restored
}

fn create_same_directory_temp(
    parent: &Path,
    destination: &Path,
) -> Result<(PathBuf, File), TranslateError> {
    let file_name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("config.toml");
    for _ in 0..32 {
        let sequence = TEMP_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = parent.join(format!(
            ".{file_name}.{}.{}.tmp",
            std::process::id(),
            sequence
        ));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => {
                return Err(TranslateError::Config(format!(
                    "creating temporary config {}: {e}",
                    path.display()
                )))
            }
        }
    }
    Err(TranslateError::Config(
        "could not allocate a unique temporary config file".into(),
    ))
}

#[derive(Debug, Clone)]
pub struct SystemCredentialStore {
    config_dir: PathBuf,
}

impl SystemCredentialStore {
    pub fn new(config_dir: impl Into<PathBuf>) -> Self {
        Self {
            config_dir: config_dir.into(),
        }
    }
}

enum SystemCredentialRollbackState {
    None,
    File {
        path: PathBuf,
        previous: Option<Zeroizing<Vec<u8>>>,
    },
    Keychain {
        service: String,
        account: String,
        previous: Option<Zeroizing<String>>,
        fallback: Option<(PathBuf, Option<Zeroizing<Vec<u8>>>)>,
    },
}

/// Opaque rollback snapshot for [`SystemCredentialStore`]. Secret material is
/// zeroized on drop and is never exposed through the public commit interface.
#[doc(hidden)]
pub struct SystemCredentialRollback(SystemCredentialRollbackState);

fn snapshot_secret_file(path: &Path) -> Result<Option<Zeroizing<Vec<u8>>>, TranslateError> {
    match crate::platform::secure_read_file(path) {
        Ok(bytes) => Ok(Some(Zeroizing::new(bytes))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(TranslateError::Internal(format!(
            "secure credential snapshot {}: {error}",
            path.display()
        ))),
    }
}

fn restore_secret_file(
    path: &Path,
    previous: Option<Zeroizing<Vec<u8>>>,
) -> Result<(), TranslateError> {
    match previous {
        Some(bytes) => crate::platform::secure_atomic_write(path, &bytes).map_err(|error| {
            TranslateError::Internal(format!(
                "secure credential rollback write {}: {error}",
                path.display()
            ))
        }),
        None => match crate::platform::secure_read_file(path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(TranslateError::Internal(format!(
                "secure credential rollback check {}: {error}",
                path.display()
            ))),
            Ok(mut bytes) => {
                zeroize::Zeroize::zeroize(&mut bytes);
                std::fs::remove_file(path).map_err(|error| {
                    TranslateError::Internal(format!(
                        "secure credential rollback remove {}: {error}",
                        path.display()
                    ))
                })?;
                if let Some(parent) = path.parent() {
                    crate::platform::current()
                        .sync_parent_directory(parent)
                        .map_err(|error| {
                            TranslateError::Internal(format!(
                                "syncing credential directory {} after rollback: {error}",
                                parent.display()
                            ))
                        })?;
                }
                Ok(())
            }
        },
    }
}

impl CredentialStore for SystemCredentialStore {
    type Rollback = SystemCredentialRollback;

    fn store(
        &self,
        candidate: &mut Config,
        credential: Credential,
    ) -> Result<Self::Rollback, TranslateError> {
        let Credential::Store(key) = credential else {
            return Ok(SystemCredentialRollback(
                SystemCredentialRollbackState::None,
            ));
        };
        match candidate.provider.api_key.source.as_str() {
            "keychain" => {
                let service = candidate.provider.api_key.service.clone();
                let account = candidate.provider.api_key.account.clone();
                let entry = crate::secrets::KeychainSecrets::new(&service, &account);
                let previous = entry.snapshot_api_key()?;
                let fallback = if crate::platform::secure_file_storage_supported() {
                    let path = crate::secrets::FileSecrets::keyfile_path(&self.config_dir);
                    let previous = snapshot_secret_file(&path)?;
                    Some((path, previous))
                } else {
                    None
                };
                let rollback = SystemCredentialRollback(
                    SystemCredentialRollbackState::Keychain {
                        service,
                        account,
                        previous,
                        fallback,
                    },
                );

                let write_result = (|| {
                    entry.set_api_key(key.clone())?;
                    let readback_ok = matches!(entry.get_api_key(), Ok(read) if *read == *key);
                    if !readback_ok {
                        if !crate::platform::secure_file_storage_supported() {
                            return Err(TranslateError::SetupWizard(
                                "keychain write did not persist and secure file fallback is unavailable on this platform".into(),
                            ));
                        }
                        let keyfile = crate::secrets::FileSecrets::keyfile_path(&self.config_dir);
                        crate::secrets::FileSecrets::new(keyfile.clone()).set_api_key(key)?;
                        candidate.provider.api_key.source = "file".into();
                        candidate.provider.api_key.path = keyfile.to_string_lossy().into_owned();
                        tracing::warn!(
                            path = %keyfile.display(),
                            "keychain write didn't persist; fell back to 0600 keyfile"
                        );
                    }
                    Ok(())
                })();
                if let Err(write_error) = write_result {
                    return match self.rollback(rollback) {
                        Ok(()) => Err(write_error),
                        Err(rollback_error) => Err(TranslateError::Config(format!(
                            "credential write failed ({write_error}); restoring the previous credential also failed ({rollback_error})"
                        ))),
                    };
                }
                Ok(rollback)
            }
            "file" => {
                let path = PathBuf::from(&candidate.provider.api_key.path);
                let previous = snapshot_secret_file(&path)?;
                crate::secrets::FileSecrets::new(path.clone()).set_api_key(key)?;
                Ok(SystemCredentialRollback(
                    SystemCredentialRollbackState::File { path, previous },
                ))
            }
            "env" | "prompt" => Err(TranslateError::Config(format!(
                "cannot save a typed API key to environment storage; set {} and clear the key field",
                candidate.provider.api_key.env_var
            ))),
            other => Err(TranslateError::Config(format!(
                "unsupported credential storage '{other}'"
            ))),
        }
    }

    fn rollback(&self, rollback: Self::Rollback) -> Result<(), TranslateError> {
        match rollback.0 {
            SystemCredentialRollbackState::None => Ok(()),
            SystemCredentialRollbackState::File { path, previous } => {
                restore_secret_file(&path, previous)
            }
            SystemCredentialRollbackState::Keychain {
                service,
                account,
                previous,
                fallback,
            } => {
                let entry = crate::secrets::KeychainSecrets::new(service, account);
                let keychain_result = entry.restore_api_key(previous);
                let fallback_result = match fallback {
                    Some((path, previous)) => restore_secret_file(&path, previous),
                    None => Ok(()),
                };
                match (keychain_result, fallback_result) {
                    (Ok(()), Ok(())) => Ok(()),
                    (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
                    (Err(keychain_error), Err(file_error)) => Err(TranslateError::Config(format!(
                        "keychain rollback failed ({keychain_error}); fallback-file rollback also failed ({file_error})"
                    ))),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use std::cell::Cell;
    use std::sync::{Arc, Mutex};
    use zeroize::Zeroizing;

    #[derive(Clone)]
    struct MemoryAtomicConfig {
        contents: Arc<Mutex<String>>,
        fail_replace: bool,
    }

    impl MemoryAtomicConfig {
        fn containing(contents: impl Into<String>) -> Self {
            Self {
                contents: Arc::new(Mutex::new(contents.into())),
                fail_replace: false,
            }
        }

        fn fail_rename(contents: impl Into<String>) -> Self {
            Self {
                contents: Arc::new(Mutex::new(contents.into())),
                fail_replace: true,
            }
        }

        fn contents(&self) -> String {
            self.contents.lock().unwrap().clone()
        }
    }

    impl AtomicConfigStore for MemoryAtomicConfig {
        fn replace(&self, candidate: &Config) -> Result<(), TranslateError> {
            if self.fail_replace {
                return Err(TranslateError::Config("rename denied".into()));
            }
            *self.contents.lock().unwrap() = toml::to_string_pretty(candidate).unwrap();
            Ok(())
        }
    }

    struct FailingCredentialStore;

    impl CredentialStore for FailingCredentialStore {
        type Rollback = ();

        fn store(
            &self,
            _candidate: &mut Config,
            _credential: Credential,
        ) -> Result<Self::Rollback, TranslateError> {
            Err(TranslateError::SetupWizard("denied".into()))
        }

        fn rollback(&self, _rollback: Self::Rollback) -> Result<(), TranslateError> {
            Ok(())
        }
    }

    #[derive(Clone, Default)]
    struct MemoryCredentialStore {
        secret: Arc<Mutex<Option<Zeroizing<String>>>>,
        store_calls: Arc<Mutex<usize>>,
        rollback_calls: Arc<Mutex<usize>>,
    }

    impl MemoryCredentialStore {
        fn containing(value: Option<&str>) -> Self {
            Self {
                secret: Arc::new(Mutex::new(
                    value.map(|value| Zeroizing::new(value.to_string())),
                )),
                ..Default::default()
            }
        }

        fn value(&self) -> Option<String> {
            self.secret
                .lock()
                .unwrap()
                .as_ref()
                .map(|value| value.to_string())
        }
    }

    impl CredentialStore for MemoryCredentialStore {
        type Rollback = Option<Zeroizing<String>>;

        fn store(
            &self,
            _candidate: &mut Config,
            credential: Credential,
        ) -> Result<Self::Rollback, TranslateError> {
            let Credential::Store(key) = credential else {
                return Ok(self.secret.lock().unwrap().clone());
            };
            *self.store_calls.lock().unwrap() += 1;
            Ok(self.secret.lock().unwrap().replace(key))
        }

        fn rollback(&self, rollback: Self::Rollback) -> Result<(), TranslateError> {
            *self.rollback_calls.lock().unwrap() += 1;
            *self.secret.lock().unwrap() = rollback;
            Ok(())
        }
    }

    fn candidate() -> Config {
        let mut candidate = Config::default();
        candidate.provider.kind = "openai".into();
        candidate.provider.model = "gpt-4o-mini".into();
        candidate
    }

    fn credential() -> Credential {
        Credential::Store(Zeroizing::new("sk-new".into()))
    }

    #[test]
    fn failed_secret_write_preserves_old_config_file() {
        let old_toml = "[provider]\ntype = \"anthropic\"\n";
        let fs = MemoryAtomicConfig::containing(old_toml);
        let result = ConfigCommitter::new(fs.clone(), FailingCredentialStore)
            .commit(candidate(), credential());
        assert!(result.is_err());
        assert_eq!(fs.contents(), old_toml);
    }

    #[test]
    fn failed_config_replace_restores_the_previous_credential() {
        let old_toml = "[provider]\ntype = \"anthropic\"\n";
        let fs = MemoryAtomicConfig::fail_rename(old_toml);
        let credentials = MemoryCredentialStore::containing(Some("sk-old"));
        let result =
            ConfigCommitter::new(fs.clone(), credentials.clone()).commit(candidate(), credential());
        assert!(result.is_err());
        assert_eq!(fs.contents(), old_toml);
        assert_eq!(credentials.value().as_deref(), Some("sk-old"));
        assert_eq!(*credentials.rollback_calls.lock().unwrap(), 1);
    }

    #[test]
    fn failed_config_replace_removes_a_newly_created_credential() {
        let old_toml = "[provider]\ntype = \"anthropic\"\n";
        let fs = MemoryAtomicConfig::fail_rename(old_toml);
        let credentials = MemoryCredentialStore::containing(None);

        let result =
            ConfigCommitter::new(fs, credentials.clone()).commit(candidate(), credential());

        assert!(result.is_err());
        assert_eq!(credentials.value(), None);
        assert_eq!(*credentials.rollback_calls.lock().unwrap(), 1);
    }

    #[derive(Default)]
    struct RecordingFilePlatform {
        parent_syncs: Cell<usize>,
        replacements: Cell<usize>,
        fail_sync: bool,
    }

    impl Platform for RecordingFilePlatform {
        fn replace_file(&self, source: &Path, destination: &Path) -> Result<(), std::io::Error> {
            self.replacements.set(self.replacements.get() + 1);
            std::fs::rename(source, destination)
        }

        fn sync_parent_directory(&self, _parent: &Path) -> Result<(), std::io::Error> {
            self.parent_syncs.set(self.parent_syncs.get() + 1);
            if self.fail_sync {
                Err(std::io::Error::other("injected directory sync failure"))
            } else {
                Ok(())
            }
        }
    }

    #[test]
    fn disk_atomic_config_replaces_existing_config_file_and_syncs_parent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[provider]\ntype = \"anthropic\"\n").unwrap();
        let platform = RecordingFilePlatform::default();

        DiskAtomicConfig::new(&path)
            .replace_with_platform(&candidate(), &platform)
            .unwrap();

        let persisted = Config::load(&path).unwrap();
        assert_eq!(persisted.provider.kind, "openai");
        assert_eq!(persisted.provider.model, "gpt-4o-mini");
        assert_eq!(platform.parent_syncs.get(), 1);
        assert_eq!(
            std::fs::read_dir(dir.path()).unwrap().count(),
            1,
            "temporary file should be renamed, not left behind"
        );
    }

    #[test]
    fn directory_sync_failure_restores_the_previous_visible_config() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let old_toml = "[provider]\ntype = \"anthropic\"\n";
        std::fs::write(&path, old_toml).unwrap();
        let platform = RecordingFilePlatform {
            fail_sync: true,
            ..Default::default()
        };

        let error = DiskAtomicConfig::new(&path)
            .replace_with_platform(&candidate(), &platform)
            .unwrap_err();

        assert!(error.to_string().contains("syncing config directory"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), old_toml);
        assert_eq!(platform.replacements.get(), 2);
    }

    #[test]
    fn system_file_store_restores_existing_secret_after_config_failure() {
        if !crate::platform::secure_file_storage_supported_for_test() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let key_path = dir.path().join("api-key");
        crate::platform::secure_atomic_write(&key_path, b"sk-old").unwrap();
        let mut candidate = candidate();
        candidate.provider.api_key.source = "file".into();
        candidate.provider.api_key.path = key_path.to_string_lossy().into_owned();
        let fs = MemoryAtomicConfig::fail_rename("old config");

        let result = ConfigCommitter::new(fs, SystemCredentialStore::new(dir.path()))
            .commit(candidate, credential());

        assert!(result.is_err());
        assert_eq!(
            crate::platform::secure_read_file(&key_path).unwrap(),
            b"sk-old"
        );
    }

    #[test]
    fn system_file_store_removes_new_secret_after_config_failure() {
        if !crate::platform::secure_file_storage_supported_for_test() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let key_path = dir.path().join("api-key");
        let mut candidate = candidate();
        candidate.provider.api_key.source = "file".into();
        candidate.provider.api_key.path = key_path.to_string_lossy().into_owned();
        let fs = MemoryAtomicConfig::fail_rename("old config");

        let result = ConfigCommitter::new(fs, SystemCredentialStore::new(dir.path()))
            .commit(candidate, credential());

        assert!(result.is_err());
        assert!(!key_path.exists());
    }

    #[test]
    fn successful_commit_returns_the_persisted_candidate() {
        let fs = MemoryAtomicConfig::containing(String::new());
        let committed = ConfigCommitter::new(fs.clone(), MemoryCredentialStore::default())
            .commit(candidate(), credential())
            .unwrap();
        assert_eq!(committed.config.provider.kind, "openai");
        assert!(fs.contents().contains("type = \"openai\""));
    }
}
