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

    /// Lock the connection, running `f` inside the guard. If the mutex
    /// is poisoned, log an error and recover via `into_inner` — the
    /// underlying `Connection` is still usable.
    fn with_conn<T>(
        &self,
        f: impl FnOnce(&Connection) -> Result<T, TranslateError>,
    ) -> Result<T, TranslateError> {
        match self.conn.lock() {
            Ok(guard) => f(&guard),
            Err(poisoned) => {
                tracing::error!("history mutex poisoned; attempting recovery");
                f(&poisoned.into_inner())
            }
        }
    }

    /// Number of rows currently stored. Test helper; viewer uses
    /// `query` length instead.
    pub fn count(&self) -> Result<i64, TranslateError> {
        self.with_conn(|conn| {
            conn.query_row("SELECT COUNT(*) FROM entries", [], |r| r.get(0))
                .map_err(|e| TranslateError::History(format!("count: {e}")))
        })
    }

    /// Encrypt the source/result fields and persist the row. Returns
    /// the new row's `id` (test convenience; production callers don't
    /// usually consume it).
    pub fn insert(&self, entry: NewEntry) -> Result<i64, TranslateError> {
        let (source_ct, source_nonce) = encrypt_optional(&self.key, entry.source.as_deref())?;
        let (result_ct, result_nonce) = encrypt_optional(&self.key, entry.result.as_deref())?;
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO entries
                 (created_at, action, source_lang, target_lang, char_count,
                  source_ciphertext, source_nonce, result_ciphertext, result_nonce)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                rusqlite::params![
                    entry.created_at,
                    entry.action,
                    entry.source_lang,
                    entry.target_lang,
                    entry.char_count,
                    source_ct,
                    source_nonce.as_ref().map(|n| &n[..]),
                    result_ct,
                    result_nonce.as_ref().map(|n| &n[..]),
                ],
            )
            .map_err(|e| TranslateError::History(format!("insert: {e}")))?;
            Ok(conn.last_insert_rowid())
        })
    }

    /// Insert with retention cap. After insert, prune by `created_at`
    /// ASC so the table never holds more than `max_entries`. Used by
    /// the App's per-translation hook so users don't see unbounded
    /// growth past `[history] max_entries` (default 100).
    pub fn insert_with_cap(
        &self,
        entry: NewEntry,
        max_entries: usize,
    ) -> Result<i64, TranslateError> {
        let id = self.insert(entry)?;
        self.with_conn(|conn| {
            // SQLite supports DELETE with subquery; this prunes any entries
            // beyond the newest `max_entries`.
            conn.execute(
                "DELETE FROM entries
                 WHERE id NOT IN (
                     SELECT id FROM entries
                     ORDER BY created_at DESC
                     LIMIT ?1
                 )",
                rusqlite::params![max_entries as i64],
            )
            .map_err(|e| TranslateError::History(format!("prune: {e}")))?;
            Ok(())
        })?;
        Ok(id)
    }

    /// Read up to `limit` newest rows that match `filter`. Decryption
    /// is per-row; any row whose ciphertext fails to decrypt is logged
    /// at warn and dropped from the result.
    pub fn query(
        &self,
        filter: &QueryFilter,
        limit: usize,
    ) -> Result<Vec<HistoryEntry>, TranslateError> {
        let rows = self.with_conn(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, created_at, action, source_lang, target_lang, char_count,
                            source_ciphertext, source_nonce, result_ciphertext, result_nonce
                     FROM entries
                     ORDER BY created_at DESC
                     LIMIT ?1",
                )
                .map_err(|e| TranslateError::History(format!("prepare: {e}")))?;
            let rows: Vec<RawRow> = stmt
                .query_map([limit as i64], |row| {
                    Ok(RawRow {
                        id: row.get(0)?,
                        created_at: row.get(1)?,
                        action: row.get(2)?,
                        source_lang: row.get(3)?,
                        target_lang: row.get(4)?,
                        char_count: row.get(5)?,
                        source_ct: row.get(6)?,
                        source_nonce: row.get(7)?,
                        result_ct: row.get(8)?,
                        result_nonce: row.get(9)?,
                    })
                })
                .map_err(|e| TranslateError::History(format!("query: {e}")))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| TranslateError::History(format!("query row: {e}")))?;
            Ok(rows)
        })?;

        let mut out = Vec::new();
        for raw in rows {
            let source = match decrypt_optional(&self.key, &raw.source_ct, &raw.source_nonce) {
                Ok(opt) => opt,
                Err(e) => {
                    tracing::warn!(error = %e, id = raw.id, "history row source decrypt failed; skipping row");
                    continue;
                }
            };
            let result = match decrypt_optional(&self.key, &raw.result_ct, &raw.result_nonce) {
                Ok(opt) => opt,
                Err(e) => {
                    tracing::warn!(error = %e, id = raw.id, "history row result decrypt failed; skipping row");
                    continue;
                }
            };
            let entry = HistoryEntry {
                id: raw.id,
                created_at: raw.created_at,
                action: raw.action,
                source_lang: raw.source_lang,
                target_lang: raw.target_lang,
                char_count: raw.char_count,
                source,
                result,
            };
            if filter_matches(&entry, filter) {
                out.push(entry);
            }
        }
        Ok(out)
    }

    pub fn delete(&self, id: i64) -> Result<(), TranslateError> {
        self.with_conn(|conn| {
            conn.execute("DELETE FROM entries WHERE id = ?1", [id])
                .map_err(|e| TranslateError::History(format!("delete: {e}")))?;
            Ok(())
        })
    }

    /// Wipe every row. Spec §7: "Clear all deletes all rows but leaves
    /// the encryption key in place." The keyfile is untouched.
    pub fn clear_all(&self) -> Result<(), TranslateError> {
        self.with_conn(|conn| {
            conn.execute("DELETE FROM entries", [])
                .map_err(|e| TranslateError::History(format!("clear_all: {e}")))?;
            Ok(())
        })
    }
}

/// Return type of `encrypt_optional`: (ciphertext, nonce) where both
/// are `None` when the input plaintext is `None`.
type EncryptedOptional = (Option<Vec<u8>>, Option<[u8; 12]>);

struct RawRow {
    id: i64,
    created_at: i64,
    action: String,
    source_lang: Option<String>,
    target_lang: Option<String>,
    char_count: i64,
    source_ct: Option<Vec<u8>>,
    source_nonce: Option<Vec<u8>>,
    result_ct: Option<Vec<u8>>,
    result_nonce: Option<Vec<u8>>,
}

/// Encrypt an optional plaintext. `None` → both columns are NULL.
/// `Some(s)` → returns ciphertext + 12-byte nonce.
fn encrypt_optional(
    key: &Zeroizing<[u8; 32]>,
    plaintext: Option<&str>,
) -> Result<EncryptedOptional, TranslateError> {
    match plaintext {
        None => Ok((None, None)),
        Some(s) => {
            let (ct, nonce) = crate::history::crypto::encrypt(key, s.as_bytes())?;
            Ok((Some(ct), Some(nonce)))
        }
    }
}

/// Decrypt an optional ciphertext. `None`/`None` → `None`. `Some`/`Some` →
/// `Some(Zeroizing<String>)`. Anything else (e.g. ciphertext but no
/// nonce, or invalid UTF-8 after decrypt) is treated as a corruption
/// case and returns `Err`.
fn decrypt_optional(
    key: &Zeroizing<[u8; 32]>,
    ciphertext: &Option<Vec<u8>>,
    nonce: &Option<Vec<u8>>,
) -> Result<Option<Zeroizing<String>>, TranslateError> {
    match (ciphertext, nonce) {
        (None, None) => Ok(None),
        (Some(ct), Some(n)) => {
            if n.len() != 12 {
                return Err(TranslateError::History(format!(
                    "nonce wrong size: expected 12, got {}",
                    n.len()
                )));
            }
            let mut nonce_arr = [0u8; 12];
            nonce_arr.copy_from_slice(n);
            let pt_bytes = crate::history::crypto::decrypt(key, ct, &nonce_arr)?;
            let s = String::from_utf8(pt_bytes.to_vec()).map_err(|e| {
                TranslateError::History(format!("decrypted bytes are not utf-8: {e}"))
            })?;
            Ok(Some(Zeroizing::new(s)))
        }
        _ => Err(TranslateError::History(
            "row has half-NULL ciphertext/nonce".into(),
        )),
    }
}

fn filter_matches(entry: &HistoryEntry, filter: &QueryFilter) -> bool {
    let Some(q) = filter.query.as_ref() else {
        return true;
    };
    let q = q.trim();
    if q.is_empty() {
        return true;
    }
    let q_lc = q.to_lowercase();
    if entry.action.to_lowercase().contains(&q_lc) {
        return true;
    }
    if let Some(s) = entry.source_lang.as_deref() {
        if s.to_lowercase().contains(&q_lc) {
            return true;
        }
    }
    if let Some(t) = entry.target_lang.as_deref() {
        if t.to_lowercase().contains(&q_lc) {
            return true;
        }
    }
    if let Some(s) = entry.source.as_ref() {
        if s.to_lowercase().contains(&q_lc) {
            return true;
        }
    }
    if let Some(r) = entry.result.as_ref() {
        if r.to_lowercase().contains(&q_lc) {
            return true;
        }
    }
    false
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

    fn fixture_entry(action: &str, source: &str, result: &str) -> NewEntry {
        NewEntry {
            created_at: 1_700_000_000, // 2023-11-14
            action: action.into(),
            source_lang: Some("en".into()),
            target_lang: Some("de".into()),
            char_count: source.chars().count() as i64,
            source: Some(source.into()),
            result: Some(result.into()),
        }
    }

    #[test]
    fn insert_then_query_returns_decrypted_row() {
        let h = History::in_memory(test_key()).unwrap();
        h.insert(fixture_entry("translate", "Hello", "Hallo"))
            .unwrap();
        let rows = h.query(&QueryFilter::default(), 100).unwrap();
        assert_eq!(rows.len(), 1);
        let r = &rows[0];
        assert_eq!(r.action, "translate");
        assert_eq!(r.source_lang.as_deref(), Some("en"));
        assert_eq!(r.target_lang.as_deref(), Some("de"));
        assert_eq!(r.char_count, 5);
        assert_eq!(r.source.as_ref().unwrap().as_str(), "Hello");
        assert_eq!(r.result.as_ref().unwrap().as_str(), "Hallo");
    }

    #[test]
    fn insert_with_none_text_columns_writes_null_blobs() {
        let h = History::in_memory(test_key()).unwrap();
        let mut e = fixture_entry("translate", "", "");
        // Simulate `[history] store_text = false` mode: caller passes None.
        e.source = None;
        e.result = None;
        h.insert(e).unwrap();
        let rows = h.query(&QueryFilter::default(), 100).unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].source.is_none());
        assert!(rows[0].result.is_none());
    }

    #[test]
    fn query_returns_rows_ordered_newest_first() {
        let h = History::in_memory(test_key()).unwrap();
        for ts in [1_000, 2_000, 3_000] {
            let mut e = fixture_entry("translate", "x", "y");
            e.created_at = ts;
            h.insert(e).unwrap();
        }
        let rows = h.query(&QueryFilter::default(), 100).unwrap();
        let times: Vec<i64> = rows.iter().map(|r| r.created_at).collect();
        assert_eq!(times, vec![3_000, 2_000, 1_000]);
    }

    #[test]
    fn query_respects_limit() {
        let h = History::in_memory(test_key()).unwrap();
        for i in 0..10 {
            let mut e = fixture_entry("translate", "x", "y");
            e.created_at = i;
            h.insert(e).unwrap();
        }
        let rows = h.query(&QueryFilter::default(), 3).unwrap();
        assert_eq!(rows.len(), 3);
    }

    #[test]
    fn query_filter_matches_decrypted_source_and_result_case_insensitive() {
        let h = History::in_memory(test_key()).unwrap();
        h.insert(fixture_entry(
            "translate",
            "Smart Table demo",
            "Smart Table Demo",
        ))
        .unwrap();
        h.insert(fixture_entry("rewrite", "completely different", "noise"))
            .unwrap();

        let rows = h
            .query(
                &QueryFilter {
                    query: Some("smart".into()),
                },
                100,
            )
            .unwrap();
        assert_eq!(
            rows.len(),
            1,
            "only one row contains 'smart' (case-insensitive)"
        );
    }

    #[test]
    fn query_filter_matches_action_label() {
        let h = History::in_memory(test_key()).unwrap();
        h.insert(fixture_entry("translate", "a", "b")).unwrap();
        h.insert(fixture_entry("rewrite", "c", "d")).unwrap();
        let rows = h
            .query(
                &QueryFilter {
                    query: Some("rewr".into()),
                },
                100,
            )
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].action, "rewrite");
    }

    #[test]
    fn query_skips_half_null_ciphertext_rows() {
        let h = History::in_memory(test_key()).unwrap();
        {
            let conn = h.conn.lock().unwrap_or_else(|e| e.into_inner());
            conn.execute(
                "INSERT INTO entries
                 (created_at, action, source_lang, target_lang, char_count,
                  source_ciphertext, source_nonce, result_ciphertext, result_nonce)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, NULL, NULL)",
                rusqlite::params![
                    1_700_000_000i64,
                    "translate",
                    "en",
                    "de",
                    15i64,
                    b"nonce protected".as_slice(),
                ],
            )
            .unwrap();
        }

        let rows = h.query(&QueryFilter::default(), 100).unwrap();
        assert!(rows.is_empty(), "corrupt half-NULL rows are skipped");
    }

    #[test]
    fn delete_removes_a_specific_row() {
        let h = History::in_memory(test_key()).unwrap();
        h.insert(fixture_entry("translate", "a", "b")).unwrap();
        h.insert(fixture_entry("translate", "c", "d")).unwrap();
        let rows = h.query(&QueryFilter::default(), 100).unwrap();
        assert_eq!(rows.len(), 2);
        let id = rows[0].id;
        h.delete(id).unwrap();
        let rows = h.query(&QueryFilter::default(), 100).unwrap();
        assert_eq!(rows.len(), 1);
        assert_ne!(rows[0].id, id);
    }

    #[test]
    fn clear_all_removes_every_row_but_preserves_the_db() {
        let h = History::in_memory(test_key()).unwrap();
        for _ in 0..3 {
            h.insert(fixture_entry("translate", "x", "y")).unwrap();
        }
        assert_eq!(h.count().unwrap(), 3);
        h.clear_all().unwrap();
        assert_eq!(h.count().unwrap(), 0);
        // After clear, new inserts still work.
        h.insert(fixture_entry("translate", "after", "clear"))
            .unwrap();
        assert_eq!(h.count().unwrap(), 1);
    }

    #[test]
    fn insert_prunes_oldest_when_above_max() {
        let h = History::in_memory(test_key()).unwrap();
        for ts in 0..150 {
            let mut e = fixture_entry("translate", "x", "y");
            e.created_at = ts;
            h.insert_with_cap(e, 100).unwrap();
        }
        assert_eq!(h.count().unwrap(), 100);
        // Oldest (ts=0..49) should be gone; newest (ts=149) present.
        let rows = h.query(&QueryFilter::default(), 200).unwrap();
        let oldest_kept = rows.iter().map(|r| r.created_at).min().unwrap();
        let newest = rows.iter().map(|r| r.created_at).max().unwrap();
        assert_eq!(newest, 149);
        assert!(oldest_kept >= 50, "oldest 50 rows should have been pruned");
    }

    #[test]
    fn round_trip_through_real_file_persists_across_reopens() {
        let dir = TempDir::new().unwrap();
        let db = dir.path().join("history.db");
        {
            let h = History::open(&db, test_key()).unwrap();
            h.insert(fixture_entry("translate", "persisted", "encrypted"))
                .unwrap();
        }
        // New History over the same file.
        let h = History::open(&db, test_key()).unwrap();
        let rows = h.query(&QueryFilter::default(), 100).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].source.as_ref().unwrap().as_str(), "persisted");
        assert_eq!(rows[0].result.as_ref().unwrap().as_str(), "encrypted");
    }

    #[test]
    fn rows_undecryptable_with_wrong_key_are_silently_skipped() {
        let dir = TempDir::new().unwrap();
        let db = dir.path().join("history.db");
        {
            let h = History::open(&db, test_key()).unwrap();
            h.insert(fixture_entry("translate", "tagged-A", "result-A"))
                .unwrap();
        }
        // Reopen with a different key — decrypts fail per row.
        let other_key = derive_key(&Zeroizing::new([99u8; 32])).unwrap();
        let h = History::open(&db, other_key).unwrap();
        let rows = h.query(&QueryFilter::default(), 100).unwrap();
        // Per cross-cutting decision §6: per-row decrypt errors → log
        // warn + skip. Either zero rows returned, or rows with None
        // source/result. We accept either pattern; choose the simpler
        // (zero rows = full skip) for M5.
        assert_eq!(rows.len(), 0, "rows that fail to decrypt are skipped");
    }

    #[test]
    fn round_trip_with_100_random_strings() {
        // Spec exit criterion §M5 #5: encrypt → store → load → decrypt
        // for 100 random strings of varying length.
        use rand::distributions::{Alphanumeric, DistString};
        let h = History::in_memory(test_key()).unwrap();
        let mut originals = Vec::with_capacity(100);
        for i in 0..100 {
            let len = (i % 50) + 1;
            let s = Alphanumeric.sample_string(&mut rand::thread_rng(), len);
            let mut e = fixture_entry("translate", &s, &s);
            e.created_at = i as i64;
            h.insert(e).unwrap();
            originals.push(s);
        }
        let rows = h.query(&QueryFilter::default(), 200).unwrap();
        assert_eq!(rows.len(), 100);
        // Newest-first ordering: rows[0] corresponds to originals[99].
        for (i, row) in rows.iter().enumerate() {
            let original_idx = 99 - i;
            assert_eq!(
                row.source.as_ref().unwrap().as_str(),
                originals[original_idx].as_str()
            );
        }
    }

    #[test]
    fn poison_recovery_still_serves_operations() {
        // Simulate a poisoned mutex: insert a row, poison the lock,
        // then verify count still works via with_conn's recovery path.
        let h = std::sync::Arc::new(History::in_memory(test_key()).unwrap());
        h.insert(fixture_entry("translate", "before", "poison")).unwrap();
        assert_eq!(h.count().unwrap(), 1);

        // Poison the mutex by panicking while holding the lock.
        let h2 = std::sync::Arc::clone(&h);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = h2.conn.lock().unwrap();
            panic!("simulated panic inside lock");
        }));
        assert!(result.is_err(), "panic should have poisoned the mutex");

        // After poison, with_conn should recover and still serve queries.
        assert_eq!(h.count().unwrap(), 1);
        let rows = h.query(&QueryFilter::default(), 100).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].source.as_ref().unwrap().as_str(), "before");

        // Insert after poison should also work.
        h.insert(fixture_entry("translate", "after", "poison")).unwrap();
        assert_eq!(h.count().unwrap(), 2);
    }
}
