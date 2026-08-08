//! Transactional configuration and credential commit flow.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use zeroize::Zeroizing;

use crate::config::Config;
use crate::error::TranslateError;
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
    fn store(&self, candidate: &mut Config, credential: Credential) -> Result<(), TranslateError>;
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
        self.credentials.store(&mut candidate, credential)?;
        candidate.validate()?;
        self.fs.replace(&candidate)?;
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
}

static TEMP_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

impl AtomicConfigStore for DiskAtomicConfig {
    fn replace(&self, candidate: &Config) -> Result<(), TranslateError> {
        let contents = toml::to_string_pretty(candidate)
            .map_err(|e| TranslateError::Config(format!("serializing config: {e}")))?;
        let parent = self
            .path
            .parent()
            .ok_or_else(|| TranslateError::Config("config path has no parent".into()))?;
        std::fs::create_dir_all(parent)
            .map_err(|e| TranslateError::Config(format!("creating {}: {e}", parent.display())))?;

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
            std::fs::rename(&temp_path, &self.path).map_err(|e| {
                TranslateError::Config(format!(
                    "replacing {} with {}: {e}",
                    self.path.display(),
                    temp_path.display()
                ))
            })?;
            Ok(())
        })();
        if staged.is_err() {
            let _ = std::fs::remove_file(&temp_path);
        }
        staged
    }
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

impl CredentialStore for SystemCredentialStore {
    fn store(&self, candidate: &mut Config, credential: Credential) -> Result<(), TranslateError> {
        let Credential::Store(key) = credential else {
            return Ok(());
        };
        match candidate.provider.api_key.source.as_str() {
            "keychain" => {
                let entry = crate::secrets::KeychainSecrets::new(
                    &candidate.provider.api_key.service,
                    &candidate.provider.api_key.account,
                );
                entry.set_api_key(key.clone())?;
                let verify = crate::secrets::KeychainSecrets::new(
                    &candidate.provider.api_key.service,
                    &candidate.provider.api_key.account,
                );
                let readback_ok = matches!(verify.get_api_key(), Ok(read) if *read == *key);
                if !readback_ok {
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
            }
            "file" => {
                let path = PathBuf::from(&candidate.provider.api_key.path);
                crate::secrets::FileSecrets::new(path).set_api_key(key)
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
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
        fn store(
            &self,
            _candidate: &mut Config,
            _credential: Credential,
        ) -> Result<(), TranslateError> {
            Err(TranslateError::SetupWizard("denied".into()))
        }
    }

    #[derive(Default)]
    struct RecordingCredentialStore {
        calls: Arc<Mutex<usize>>,
    }

    impl CredentialStore for RecordingCredentialStore {
        fn store(
            &self,
            _candidate: &mut Config,
            credential: Credential,
        ) -> Result<(), TranslateError> {
            if matches!(credential, Credential::Store(_)) {
                *self.calls.lock().unwrap() += 1;
            }
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
    fn failed_config_replace_does_not_publish_candidate() {
        let old_toml = "[provider]\ntype = \"anthropic\"\n";
        let fs = MemoryAtomicConfig::fail_rename(old_toml);
        let result = ConfigCommitter::new(fs.clone(), RecordingCredentialStore::default())
            .commit(candidate(), credential());
        assert!(result.is_err());
        assert_eq!(fs.contents(), old_toml);
    }

    #[test]
    fn successful_commit_returns_the_persisted_candidate() {
        let fs = MemoryAtomicConfig::containing(String::new());
        let committed = ConfigCommitter::new(fs.clone(), RecordingCredentialStore::default())
            .commit(candidate(), credential())
            .unwrap();
        assert_eq!(committed.config.provider.kind, "openai");
        assert!(fs.contents().contains("type = \"openai\""));
    }
}
