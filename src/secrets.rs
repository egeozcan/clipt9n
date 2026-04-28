//! API key resolution. M1 implements env-var lookup only. M6 adds keychain
//! (preferred) → env-var → setup-wizard fallback chain via the same `Secrets`
//! trait surface.

use zeroize::Zeroizing;

use crate::error::TranslateError;

pub trait Secrets: Send + Sync {
    /// Resolve the API key. Returned in a `Zeroizing<String>` so it's wiped
    /// from memory on drop (defense-in-depth; not a substitute for keychain
    /// storage, which lands in M6).
    fn get_api_key(&self) -> Result<Zeroizing<String>, TranslateError>;
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
}

#[cfg(test)]
mod tests {
    use super::*;

    // These tests touch process-global env state. They each use a unique
    // variable name to avoid interfering with each other when run in parallel.

    #[test]
    fn returns_value_when_env_var_set() {
        let var = "CLIPT9N_TEST_KEY_PRESENT";
        std::env::set_var(var, "sk-test-12345");
        let s = EnvSecrets::new(var);
        let key = s.get_api_key().unwrap();
        assert_eq!(&*key, "sk-test-12345");
        std::env::remove_var(var);
    }

    #[test]
    fn returns_error_when_env_var_missing() {
        let var = "CLIPT9N_TEST_KEY_ABSENT";
        std::env::remove_var(var);
        let s = EnvSecrets::new(var);
        let err = s.get_api_key().unwrap_err();
        match err {
            TranslateError::MissingApiKey { env_var } => assert_eq!(env_var, var),
            other => panic!("expected MissingApiKey, got {other:?}"),
        }
    }

    #[test]
    fn returned_key_is_zeroizing() {
        let var = "CLIPT9N_TEST_KEY_ZEROIZE";
        std::env::set_var(var, "secret");
        let s = EnvSecrets::new(var);
        let _key: Zeroizing<String> = s.get_api_key().unwrap();
        // Type-level assertion: if `_key` weren't `Zeroizing<String>`, the let
        // binding above would fail to compile.
        std::env::remove_var(var);
    }
}
