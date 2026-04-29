//! Encrypted history storage. SQLite (rusqlite, bundled) at
//! `<config_dir>/history.db`. Schema per spec §7: metadata is plaintext
//! for searchability, source/result text are encrypted at the
//! application layer with ChaCha20-Poly1305.
//!
//! Discipline: the inner `Connection` is wrapped in `Mutex` because
//! rusqlite is `Send + !Sync`. Hold the lock briefly per call; never
//! across an `await`. Failures are always recoverable — corruption →
//! disabled flag, write failure → log + drop, decryption error per row
//! → log + skip.

use std::path::Path;
use std::sync::Mutex;

use rusqlite::Connection;
use zeroize::Zeroizing;

use crate::error::TranslateError;

/// One history row (decrypted).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryEntry {
    pub id: i64,
    /// Unix epoch seconds, NOT NULL per spec §7.
    pub created_at: i64,
    pub action: String,
    pub source_lang: Option<String>,
    pub target_lang: Option<String>,
    pub char_count: i64,
    /// `None` if `[history] store_text = false` OR the row's ciphertext
    /// failed to decrypt (treated as "redacted at user request" or
    /// "key mismatch" depending on context).
    pub source: Option<Zeroizing<String>>,
    pub result: Option<Zeroizing<String>>,
}

/// Plaintext input to `insert`. The `source`/`result` strings are
/// taken by value so the caller can drop them as soon as the call
/// returns; we encrypt and discard the plaintext immediately.
#[derive(Debug)]
pub struct NewEntry {
    pub created_at: i64,
    pub action: String,
    pub source_lang: Option<String>,
    pub target_lang: Option<String>,
    pub char_count: i64,
    /// `None` honors `[history] store_text = false`. The caller passes
    /// `None` in that mode; the schema columns become NULL.
    pub source: Option<String>,
    pub result: Option<String>,
}

/// Filter for `query`. M5 only supports text-substring filter (Rust-side,
/// post-decrypt) plus a hard cap on rows. No SQL `LIKE` — see decision §13.
#[derive(Debug, Clone, Default)]
pub struct QueryFilter {
    /// Optional case-insensitive substring filter applied to source AND
    /// result text after decryption. Pair label is also matched (the
    /// viewer renders `pair` like "DE → EN" — we match against
    /// `source_lang`/`target_lang`/`action`).
    pub query: Option<String>,
}

pub struct History {
    conn: Mutex<Connection>,
    #[allow(dead_code)] // Read by Task 6 (insert/query); attribute removed there.
    key: Zeroizing<[u8; 32]>,
}

impl std::fmt::Debug for History {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("History")
            .field("conn", &"Mutex<Connection>")
            .finish_non_exhaustive()
    }
}

impl History {
    /// Open the SQLite database at `path` and run schema migrations.
    /// `key` must be the derived AEAD key from `crypto::derive_key` (or
    /// `crypto::load_and_derive`).
    pub fn open(path: &Path, key: Zeroizing<[u8; 32]>) -> Result<Self, TranslateError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                TranslateError::History(format!("creating history dir {}: {e}", parent.display()))
            })?;
        }
        let conn = Connection::open(path)
            .map_err(|e| TranslateError::History(format!("opening {}: {e}", path.display())))?;
        Self::migrate(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
            key,
        })
    }

    /// Build a `History` backed by `:memory:`. Used in unit tests.
    pub fn in_memory(key: Zeroizing<[u8; 32]>) -> Result<Self, TranslateError> {
        let conn = Connection::open_in_memory()
            .map_err(|e| TranslateError::History(format!("open in-memory: {e}")))?;
        Self::migrate(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
            key,
        })
    }

    fn migrate(conn: &Connection) -> Result<(), TranslateError> {
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS entries (
                id                  INTEGER PRIMARY KEY AUTOINCREMENT,
                created_at          INTEGER NOT NULL,
                action              TEXT NOT NULL,
                source_lang         TEXT,
                target_lang         TEXT,
                char_count          INTEGER NOT NULL,
                source_ciphertext   BLOB,
                source_nonce        BLOB,
                result_ciphertext   BLOB,
                result_nonce        BLOB
            );
            CREATE INDEX IF NOT EXISTS idx_created_at ON entries (created_at DESC);
            "#,
        )
        .map_err(|e| TranslateError::History(format!("schema migrate: {e}")))?;
        Ok(())
    }

    /// Number of rows currently stored. Test helper; viewer uses
    /// `query` length instead.
    pub fn count(&self) -> Result<i64, TranslateError> {
        let conn = self.conn.lock().expect("history mutex poisoned");
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM entries", [], |r| r.get(0))
            .map_err(|e| TranslateError::History(format!("count: {e}")))?;
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::crypto::{derive_key, load_or_create_keyfile};
    use tempfile::TempDir;

    fn test_key() -> Zeroizing<[u8; 32]> {
        derive_key(&Zeroizing::new([42u8; 32])).unwrap()
    }

    #[test]
    fn open_in_memory_succeeds_and_count_is_zero() {
        let h = History::in_memory(test_key()).unwrap();
        assert_eq!(h.count().unwrap(), 0);
    }

    #[test]
    fn open_creates_db_file_on_first_run() {
        let dir = TempDir::new().unwrap();
        let db = dir.path().join("history.db");
        assert!(!db.exists());
        let _h = History::open(&db, test_key()).unwrap();
        assert!(db.exists());
    }

    #[test]
    fn schema_is_idempotent() {
        // Opening twice should not error (CREATE IF NOT EXISTS).
        let dir = TempDir::new().unwrap();
        let db = dir.path().join("history.db");
        let _h1 = History::open(&db, test_key()).unwrap();
        let _h2 = History::open(&db, test_key()).unwrap();
    }

    #[test]
    fn open_creates_parent_dir_if_missing() {
        let dir = TempDir::new().unwrap();
        let nested = dir.path().join("does/not/exist/history.db");
        let _h = History::open(&nested, test_key()).unwrap();
        assert!(nested.exists());
    }

    #[test]
    fn open_with_corrupt_file_returns_history_error() {
        let dir = TempDir::new().unwrap();
        let db = dir.path().join("history.db");
        std::fs::write(&db, b"not a sqlite database").unwrap();
        let err = History::open(&db, test_key()).unwrap_err();
        assert!(matches!(err, TranslateError::History(_)));
    }

    #[test]
    fn open_uses_keyfile_via_crypto_module() {
        // End-to-end: create keyfile + derive + open. Smoke test that
        // the wiring lines up.
        let dir = TempDir::new().unwrap();
        let kf = dir.path().join(".history-key");
        let secret = load_or_create_keyfile(&kf).unwrap();
        let key = derive_key(&secret).unwrap();
        let db = dir.path().join("history.db");
        let _h = History::open(&db, key).unwrap();
    }
}
