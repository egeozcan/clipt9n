//! Cross-restart state. Currently persists only the last-used slot index
//! (1–6), so Enter on the prompt window can repeat it. Custom prompts are
//! never persisted (spec privacy rule).

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::TranslateError;

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct State {
    pub last_slot: Option<u8>,
}

impl State {
    /// Read state from `path`. Missing file or malformed TOML returns
    /// `State::default()` — last-action recall is best-effort, never blocks.
    pub fn load(path: &Path) -> Self {
        let Ok(contents) = std::fs::read_to_string(path) else {
            return Self::default();
        };
        toml::from_str(&contents).unwrap_or_default()
    }

    /// Write state to `path`, creating parent dirs as needed. Returns an
    /// error on failure but the caller is expected to log and continue.
    pub fn save(&self, path: &Path) -> Result<(), TranslateError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| TranslateError::Config(format!("creating state dir: {e}")))?;
        }
        let toml_str = toml::to_string(self)
            .map_err(|e| TranslateError::Config(format!("encoding state: {e}")))?;
        std::fs::write(path, toml_str)
            .map_err(|e| TranslateError::Config(format!("writing state: {e}")))
    }

    pub fn record_slot(&mut self, slot: u8) {
        if (1..=6).contains(&slot) {
            self.last_slot = Some(slot);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn missing_file_returns_default() {
        let s = State::load(Path::new("/tmp/clipt9n-nonexistent-state-12345.toml"));
        assert_eq!(s, State::default());
        assert!(s.last_slot.is_none());
    }

    #[test]
    fn round_trip_persists_slot() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("state.toml");
        let mut s = State::default();
        s.record_slot(2);
        s.save(&path).unwrap();
        let loaded = State::load(&path);
        assert_eq!(loaded.last_slot, Some(2));
    }

    #[test]
    fn record_slot_rejects_out_of_range() {
        let mut s = State::default();
        s.record_slot(0);
        assert!(s.last_slot.is_none());
        s.record_slot(7);
        assert!(s.last_slot.is_none());
        s.record_slot(3);
        assert_eq!(s.last_slot, Some(3));
    }

    #[test]
    fn malformed_toml_returns_default() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("state.toml");
        std::fs::write(&path, "this is not :::: valid toml").unwrap();
        let s = State::load(&path);
        assert_eq!(s, State::default());
    }
}
