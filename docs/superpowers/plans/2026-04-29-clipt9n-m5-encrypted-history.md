# clipt9n M5 — Encrypted History + Viewer — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Persist every successful translation as an encrypted row in a local SQLite database, and surface the history through a hotkey-summoned viewer with search, copy-back, delete, and clear-all. Encryption is application-layer ChaCha20-Poly1305 with a per-row nonce; the key is derived via Argon2id from a per-install secret stored in a 0600-mode keyfile.

**Architecture:** A new `src/history/` module owns crypto + storage. `History::open(path, &key)` takes a derived 32-byte key and a SQLite path; the `History` value wraps a `Mutex<rusqlite::Connection>` for `Send` access from worker threads. App writes go through a per-translation tokio task with the same panic-watcher pattern M3 uses for the translator. The viewer is a new egui window selected by a second registered global hotkey (`Cmd+Shift+H` by default); the existing single eframe viewport is resized to 680×540 for history mode and back to the prompt size on close. History failures (corruption, missing key, write failure) never block the clipboard write — they degrade silently per spec §8.

**Tech Stack:** Rust 2021 / eframe 0.31 / egui 0.31 / tokio 1.42. Four new crates: `rusqlite = { version = "0.31", features = ["bundled"] }` (no system SQLite), `argon2 = "0.5"`, `chacha20poly1305 = "0.10"`, `rand = "0.8"` (for `OsRng` nonce + secret generation). All cross-platform discipline rules from M2/M3/M4 still apply: every `cfg(target_os)` and `cfg(unix)` block lives in `src/platform/`.

> **Branch:** This plan executes on `m5-encrypted-history`, branched from `main` (currently at `0e573e3`, post-M4 fast-forward). Working directory: `/Users/egecan/Code/clipt9n`.

---

## File structure

After M5, the tree gains:

```
src/
├── app.rs                       ← MODIFIED: AppState::ShowingHistory; insert
│                                              after clipboard write; route
│                                              hotkey events by ID; viewport
│                                              resize on history transitions
├── config.rs                    ← MODIFIED: [history] section + nested
│                                              [hotkey.history] sub-table
├── error.rs                     ← MODIFIED: TranslateError::History variant
├── history/
│   ├── mod.rs                   ← NEW: re-exports
│   ├── crypto.rs                ← NEW: keyfile load-or-create + Argon2id
│   │                                     KDF + ChaCha20-Poly1305 AEAD
│   └── store.rs                 ← NEW: rusqlite schema + History type +
│                                        insert/query/delete/clear_all
├── lib.rs                       ← MODIFIED: open History in CLI run path
│                                              (silent on Err)
├── main.rs                      ← MODIFIED: register two hotkeys; capture
│                                              IDs; open History; pass into
│                                              ClipApp::new
├── platform/
│   └── unix.rs                  ← MODIFIED: + set_owner_only_permissions
│                                              (0o600) free fn
└── ui/
    ├── history.rs               ← NEW: viewer model, draw, keyboard
    │                                    handling, modal confirm
    └── mod.rs                   ← MODIFIED: pub mod history
Cargo.toml                       ← MODIFIED: + 4 deps (rusqlite, argon2,
                                                chacha20poly1305, rand)
README.md                        ← MODIFIED: M5 section (encryption story,
                                                .history-key location,
                                                [history] config block)
```

Boundary discipline (unchanged from M4):

- `src/platform/` is the **only** place `#[cfg(target_os = …)]` and `#[cfg(unix)]` may appear (with the audited exception in `config::Modifier::resolve_native`). M5's chmod-to-0600 helper goes in `src/platform/unix.rs`.
- `src/ui/history.rs` knows nothing about `rusqlite`, `argon2`, `chacha20poly1305`, or platform specifics — it consumes a `Vec<HistoryEntry>` and emits intents.
- `src/history/` knows nothing about `egui` — it's a pure data + algorithm module.
- `src/app.rs` is the only seam that knows both `History` (sync `Mutex<Connection>`) and `egui` (the update thread).

---

## Glossary of cross-cutting decisions (read once)

These come up repeatedly; agreeing up front prevents drift.

1. **Argon2id with default params + a fixed salt const.** The argon2 crate's `Argon2::default()` already implements the OWASP-recommended Argon2id parameters (m=19456 KB, t=2, p=1). The salt is a 16-byte const `b"clipt9n-history\x00"` baked into the binary. The salt's job is to slow brute force across leaked databases — and since each install has a unique random 32-byte secret, salt collision doesn't matter. Rationale: avoids storing a per-install salt, which would either require an extra file or a metadata row. The keyfile alone is enough secret material.

2. **The keyfile (`<config_dir>/.history-key`) is 32 random bytes from `OsRng`, mode 0600.** First-run creates it; subsequent opens read it. On Unix the chmod-to-0600 happens via `platform::unix::set_owner_only_permissions`. On Windows the file inherits the user's profile ACL (see spec §7); we document this is less secure than the M6 keychain mode. After read, the bytes are wrapped in `Zeroizing<[u8; 32]>` and dropped after Argon2 derivation completes.

3. **Per-row 12-byte nonce from `OsRng`.** ChaCha20-Poly1305 standard. Stored beside the ciphertext column. `OsRng` is seeded from the OS entropy pool; safe to use synchronously.

4. **`rusqlite::Connection` is `Send` but `!Sync`.** Wrap in `std::sync::Mutex` for shared access from worker threads. Hold the lock briefly per call; never across an `await`. The crate's docs guarantee a `Connection` doesn't carry per-thread state, so cross-thread move via the Mutex is safe.

5. **History writes are best-effort (spec §8).** A failed insert logs `tracing::warn!` and is dropped. A panic during insert is caught by the watcher task (mirrors M3's translator-worker pattern). The user never sees a toast for a missed history row — the clipboard update is the user's primary outcome and has already happened by the time we attempt the insert.

6. **History opens are graceful (spec §8 corruption + missing-key rows).** If `History::open` returns `Err`, the app sets `history_disabled = true`, queues a one-shot toast for the next history-viewer summon ("History database unreadable. New history will not be saved."), and otherwise continues. Per-row decryption errors during query are silently dropped (warn-logged) so a partial corruption doesn't lose visible entries.

7. **`Translator::execute` is unchanged; source-language metadata is captured at dispatch time.** The detected ISO-2 language already lives on `prompt_model.detected_lang` (set in `App::show_window` at M4). The dispatch path captures that into `TranslationOutcome.detected_source_lang` and uses it for the history row's `source_lang` column. The translator does NOT learn about the history at all — separation kept.

8. **History viewer reuses the existing eframe viewport.** When `AppState::ShowingHistory` is entered, send `ViewportCommand::InnerSize(Vec2::new(680.0, 540.0))`. On exit, send the prompt-default size back. The viewport is `with_resizable(false)`, so the user can't drag it. This avoids the multi-viewport overhead and keeps the focus model trivial. (M7 may revisit if the tray menu needs to open history while the prompt is still shown.)

9. **Two registered hotkeys; events distinguished by ID.** `main.rs` registers prompt + history hotkeys against the same `GlobalHotKeyManager` and captures their `u32` IDs (returned by `HotKey::id()`). The IDs flow into `ClipApp::new` as a struct, and `drain_channels` matches `event.id` against them. If the history hotkey is disabled in config (`enabled = false`), only the prompt hotkey is registered; the history-hotkey ID is `None` and incoming IDs that aren't the prompt ID are logged at debug and ignored.

10. **`History::insert` runs in its own `runtime.spawn` + watcher.** Mirrors M3's translator pattern. The inner spawn does the SQLite insert; the outer spawn awaits the JoinHandle and converts `JoinError` (panic) into a warn log. **Don't use the same channel as `result_tx`** — history outcomes don't drive UI state. Just log + drop.

11. **`History::in_memory(&key)` is the test constructor.** Per handoff §4: cleaner than feature-gating. Uses `rusqlite::Connection::open_in_memory()` and runs the schema migration in-place. All M5 unit tests that touch `History` use this, except for the few that explicitly test on-disk semantics (e.g., file persists across reopen).

12. **`Zeroizing<String>` at every decryption boundary.** Per spec §9. `History::query` returns `HistoryEntry { source: Option<Zeroizing<String>>, result: Option<Zeroizing<String>>, ... }`. The viewer's `HistoryModel` keeps the `Zeroizing` wrappers; only when the user copies a row (via `s` or `Enter`) do we transit through a temporary `String` for the clipboard write, and that copy is on the user's choice. Ciphertext bytes don't need zeroizing; only the plaintext output of decrypt.

13. **No FTS / no SQL `LIKE` filter.** Spec §6 explicitly accepts the trade-off: search decrypts every row and filters in Rust. With 100 entries × ChaCha20 sub-millisecond per row × 50ms p95 budget, this is comfortable. No SQL query templating; rows are just `SELECT * FROM entries ORDER BY created_at DESC LIMIT N`.

14. **Sync I/O on the egui update thread for query/insert is acceptable here, with a measure-and-revisit caveat.** Per handoff §2: M4's glossary-reload pattern is sync-on-update-thread, which works for ~10KB files. M5's query is decrypt-100-rows ≈ 50ms which fits a single frame budget loosely. We measure first; if benchmark shows >16ms p95, follow-up moves to `tokio::task::spawn_blocking` + a parallel `history_rx` channel. Don't preemptively re-architect.

15. **No M4 follow-up work in M5.** Per handoff §3: don't sneak in the Jinja-conditional template-validation gap, glossary entry-validation, source-text lowercasing hoist, double-detection elimination, or `[glossary] matching` value validation. All deferred to M8. Keep the milestone focused.

16. **No new `cfg` outside `platform/`.** The 0600-mode chmod is the sole addition; it lives in `src/platform/unix.rs`. Step 11.x's grep verifies M5 introduced no other `cfg(target_os = …)` or `cfg(unix)` blocks.

---

## Pre-flight: Confirm starting state

- [ ] **Step 0.1: Verify branch and clean tree**

Run:
```bash
git rev-parse --abbrev-ref HEAD
git status --short
```
Expected: branch `m5-encrypted-history`, no working-tree changes.

If you're still on `main` or another branch, check out:
```bash
git checkout -b m5-encrypted-history
```

- [ ] **Step 0.2: Verify M4 tests pass on this branch**

Run: `cargo test --all-features 2>&1 | grep "test result:"`
Expected: lines totaling **167 passed; 0 failed** across lib, integration, and doctest test runs.

- [ ] **Step 0.3: Verify clippy + fmt are clean**

```bash
cargo clippy --all-features --all-targets -- -D warnings 2>&1 | tail -3
cargo fmt --check 2>&1 | tail -3
```
Expected: `Finished` clean / no diff.

If any pre-flight step fails, stop and report.

---

## Task 1: Add 4 deps + `TranslateError::History` variant

**Files:**
- Modify: `Cargo.toml` (`[dependencies]` block)
- Modify: `src/error.rs` (TranslateError enum + tests)

**Why:** New crates and a new error variant are the smallest-blast-radius starting point. Both are inert until consumed; verifying the crate builds cleanly with the new deps is a checkpoint that catches version mismatches before they cascade.

- [ ] **Step 1.1: Add the 4 deps**

In `Cargo.toml`, after `arboard = "3.4"` (alphabetic-ish; existing block is loosely alphabetized):

```toml
arboard = "3.4"
argon2 = "0.5"
chacha20poly1305 = "0.10"
rusqlite = { version = "0.31", features = ["bundled"] }
rand = "0.8"
```

The block already contains `whatlang = "0.16"` from M4 — keep that. The new lines are 4 additions.

- [ ] **Step 1.2: Verify deps resolve**

```bash
cargo check 2>&1 | tail -20
```
Expected: `Finished` clean. If any `error: failed to select a version` appears, stop and report — versions are pinned per the design doc; do NOT bump without confirming.

- [ ] **Step 1.3: Add `TranslateError::History` variant**

In `src/error.rs`, add after `Glossary(String)` (currently the last variant before tests):

```rust
    #[error("history error: {0}")]
    History(String),
```

- [ ] **Step 1.4: Add a unit test for the display string**

Append to the `tests` mod in `src/error.rs`:

```rust
        assert_eq!(
            TranslateError::History("encrypted db unreadable".into()).to_string(),
            "history error: encrypted db unreadable"
        );
```

(Add inside the existing `display_strings_are_user_facing` test, after the `Glossary` assertion.)

- [ ] **Step 1.5: Run tests**

```bash
cargo test --lib error 2>&1 | tail -5
```
Expected: 1 test, 1 passing.

- [ ] **Step 1.6: Commit**

```bash
git add Cargo.toml Cargo.lock src/error.rs
git commit -m "chore(M5): add rusqlite/argon2/chacha20poly1305/rand deps; History error variant"
```

---

## Task 2: Add `[history]` and `[hotkey.history]` config sections

**Files:**
- Modify: `src/config.rs` (HotkeyConfig + new HistoryHotkeyConfig + new HistoryConfig + tests)

**Why:** Spec §6 dictates `[history]` (`enabled`, `max_entries`, `store_text`, `confirm_clear`) and `[hotkey.history]` (modifiers, key, with `enabled = false` disabling registration). Adding the structs first lets every subsequent task read configured values rather than threading hardcoded defaults.

The existing `[hotkey]` schema is flat (`modifier`/`shift`/`key`/`enabled`) for the prompt hotkey; we keep that backwards-compatible and add a nested `HistoryHotkeyConfig` with the same shape under `HotkeyConfig::history`. This avoids breaking M2's hotkey-display tests and keeps M5's config additions purely additive.

- [ ] **Step 2.1: Write the failing tests**

Append to `src/config.rs`'s `tests` mod (after `loads_template_overrides`, around line 543):

```rust
    #[test]
    fn default_history_section() {
        let cfg = Config::default();
        assert!(cfg.history.enabled);
        assert_eq!(cfg.history.max_entries, 100);
        assert!(cfg.history.store_text);
        assert!(cfg.history.confirm_clear);
    }

    #[test]
    fn loads_history_overrides() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(
            f,
            r#"
[history]
enabled = false
max_entries = 25
store_text = false
confirm_clear = false
"#
        )
        .unwrap();
        let cfg = Config::load(f.path()).unwrap();
        assert!(!cfg.history.enabled);
        assert_eq!(cfg.history.max_entries, 25);
        assert!(!cfg.history.store_text);
        assert!(!cfg.history.confirm_clear);
    }

    #[test]
    fn default_history_hotkey_is_cmd_shift_h() {
        let cfg = Config::default();
        assert_eq!(cfg.hotkey.history.modifier, "cmd");
        assert!(cfg.hotkey.history.shift);
        assert_eq!(cfg.hotkey.history.key, "H");
        assert!(cfg.hotkey.history.enabled);
    }

    #[test]
    fn loads_history_hotkey_disabled() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(
            f,
            r#"
[hotkey.history]
enabled = false
"#
        )
        .unwrap();
        let cfg = Config::load(f.path()).unwrap();
        assert!(!cfg.hotkey.history.enabled);
        // Defaults preserved for the rest:
        assert_eq!(cfg.hotkey.history.key, "H");
    }

    #[test]
    fn loads_history_hotkey_custom_key() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(
            f,
            r#"
[hotkey.history]
modifier = "ctrl"
shift = false
key = "L"
enabled = true
"#
        )
        .unwrap();
        let cfg = Config::load(f.path()).unwrap();
        assert_eq!(cfg.hotkey.history.modifier, "ctrl");
        assert!(!cfg.hotkey.history.shift);
        assert_eq!(cfg.hotkey.history.key, "L");
        assert!(cfg.hotkey.history.enabled);
    }
```

- [ ] **Step 2.2: Run tests to verify failure**

```bash
cargo test --lib config 2>&1 | tail -10
```
Expected: compilation errors on `cfg.history` and `cfg.hotkey.history` (fields don't exist).

- [ ] **Step 2.3: Add `HistoryConfig` + add `history` field to `Config`**

In `src/config.rs`, modify `Config` (currently lines 15-22) to add `history`:

```rust
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Config {
    pub provider: ProviderConfig,
    pub languages: LanguagesConfig,
    pub hotkey: HotkeyConfig,
    pub ui: UiConfig,
    pub glossary: GlossaryConfig,
    pub templates: TemplatesConfig,
    pub history: HistoryConfig,
}
```

Append the new struct + `Default` impl after `TemplatesConfig`'s `Default` (around line 198):

```rust
/// `[history]` block per spec §6 + §7. M5 wires this into the
/// `History` opener and the per-translation insert path.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct HistoryConfig {
    /// When false, history is fully disabled — the SQLite file is
    /// neither opened at startup nor written to. The viewer hotkey
    /// still registers but the viewer shows an empty list.
    pub enabled: bool,
    /// Maximum entries retained. Older rows are pruned at insert time.
    pub max_entries: usize,
    /// When false, source/result columns are NULL (metadata-only row).
    /// Useful for high-sensitivity environments per spec §9.
    pub store_text: bool,
    /// Whether the "Clear all" action requires a confirmation modal.
    /// Default true; setting false makes Shift+Del immediately destructive.
    pub confirm_clear: bool,
}

impl Default for HistoryConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_entries: 100,
            store_text: true,
            confirm_clear: true,
        }
    }
}
```

- [ ] **Step 2.4: Add `HistoryHotkeyConfig` + nest under `HotkeyConfig`**

Modify `HotkeyConfig` (currently lines 103-125) to add the `history` sub-struct:

```rust
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct HotkeyConfig {
    /// "cmd" → Cmd on macOS, Ctrl on Linux/Windows. "ctrl" → Ctrl on every OS.
    /// "alt" / "super" allowed but unmapped (passthrough).
    pub modifier: String,
    pub shift: bool,
    /// Key name accepted by `global-hotkey::hotkey::Code`. Single uppercase letter
    /// like "T" maps to `Code::KeyT`.
    pub key: String,
    pub enabled: bool,
    /// Second hotkey for the history viewer (M5). Independent of the
    /// prompt hotkey above. Set `enabled = false` to skip registration.
    pub history: HistoryHotkeyConfig,
}

impl Default for HotkeyConfig {
    fn default() -> Self {
        Self {
            modifier: "cmd".into(),
            shift: true,
            key: "T".into(),
            enabled: true,
            history: HistoryHotkeyConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct HistoryHotkeyConfig {
    pub modifier: String,
    pub shift: bool,
    pub key: String,
    pub enabled: bool,
}

impl Default for HistoryHotkeyConfig {
    fn default() -> Self {
        Self {
            modifier: "cmd".into(),
            shift: true,
            key: "H".into(),
            enabled: true,
        }
    }
}
```

- [ ] **Step 2.5: Update the module-level doc comment**

Replace the doc comment at `src/config.rs:1-5` to mention M5:

```rust
//! `config.toml` loader. Reads the spec §6 schema. M1–M3 only consumed
//! `[provider]`, `[provider.api_key]`, `[languages]`, `[hotkey]`, `[ui]`.
//! M4 added `[glossary]` and `[templates]`. M5 adds `[history]` and the
//! nested `[hotkey.history]` sub-table. `[tray]` and `[logging]` are
//! still loaded but unused pending later milestones. Defaults applied
//! when fields are absent.
```

- [ ] **Step 2.6: Run tests**

```bash
cargo test --lib config 2>&1 | tail -10
```
Expected: all config tests pass (16 from M4 + 5 new = 21 total in this module).

- [ ] **Step 2.7: Commit**

```bash
git add src/config.rs
git commit -m "feat(M5): [history] + nested [hotkey.history] config sections"
```

---

## Task 3: Add `set_owner_only_permissions` to `src/platform/unix.rs`

**Files:**
- Modify: `src/platform/unix.rs` (add free function + test)

**Why:** Spec §7's keyfile fallback requires 0600 permissions on Unix. The chmod call requires `std::os::unix::fs::PermissionsExt`, which is `cfg(unix)`-gated. Per cross-platform discipline, that gate lives in `src/platform/unix.rs`; the rest of the codebase calls a free function that's a no-op on Windows.

- [ ] **Step 3.1: Write the failing test**

Append to `src/platform/unix.rs` (no existing tests in this file; add a new `tests` mod):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;
    use tempfile::NamedTempFile;

    #[test]
    fn set_owner_only_permissions_writes_0o600() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(f, "secret bytes").unwrap();
        let path = f.path().to_path_buf();
        // Pre-condition: tempfile defaults are 0o600 on most Unixes, but
        // we explicitly set 0o644 first to make the test meaningful.
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();

        set_owner_only_permissions(&path).expect("chmod 0o600 should succeed");

        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        // PermissionsExt::mode returns the full st_mode; mask off the
        // file-type bits.
        assert_eq!(mode & 0o777, 0o600, "expected 0o600, got {:o}", mode & 0o777);
    }
}
```

- [ ] **Step 3.2: Run test to verify failure**

```bash
cargo test --lib platform::unix 2>&1 | tail -10
```
Expected: compilation error on `set_owner_only_permissions` (function doesn't exist).

- [ ] **Step 3.3: Implement `set_owner_only_permissions`**

Add to `src/platform/unix.rs` (after the `install` function):

```rust
/// Set the file at `path` to mode `0o600` (owner read/write only). Called
/// by `History` after writing the keyfile. On non-Unix platforms the
/// equivalent caller path no-ops via `cfg(not(unix))` dispatch in
/// `src/history/crypto.rs`.
pub(crate) fn set_owner_only_permissions(path: &std::path::Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let perms = std::fs::Permissions::from_mode(0o600);
    std::fs::set_permissions(path, perms)
}
```

- [ ] **Step 3.4: Run test to verify pass**

```bash
cargo test --lib platform::unix::tests::set_owner_only_permissions_writes_0o600 2>&1 | tail -5
```
Expected: 1 passed.

- [ ] **Step 3.5: Commit**

```bash
git add src/platform/unix.rs
git commit -m "feat(M5): platform::unix::set_owner_only_permissions (0o600) helper"
```

---

## Task 4: Build `src/history/crypto.rs` — keyfile + Argon2id KDF + ChaCha20-Poly1305 AEAD

**Files:**
- Create: `src/history/crypto.rs`
- Create: `src/history/mod.rs` (just re-exports for now)
- Modify: `src/lib.rs` (`pub mod history`)

**Why:** Crypto is the foundation — every store path consumes the derived key + the encrypt/decrypt helpers. Build it first with thorough TDD; downstream tasks consume `derive_key`, `encrypt`, `decrypt`, and `load_or_create_keyfile`.

- [ ] **Step 4.1: Create `src/history/mod.rs`**

```rust
//! Encrypted history persistence. `crypto.rs` owns the keyfile and the
//! AEAD layer; `store.rs` owns the SQLite schema and CRUD. The viewer
//! UI lives in `src/ui/history.rs` (egui paint + keyboard handling).
//!
//! Cross-cutting policy (spec §8 + §9):
//! - Failures are best-effort. A corrupt DB or missing key surfaces a
//!   one-shot toast and disables history for the session; clipboard
//!   writes are NEVER blocked by history-side errors.
//! - Decrypted source/result text is wrapped in `Zeroizing<String>` at
//!   every public boundary so it's wiped from memory when dropped.

pub mod crypto;
pub mod store;
```

- [ ] **Step 4.2: Wire the module into the crate**

In `src/lib.rs`, add after `pub mod glossary;` (around line 7):

```rust
pub mod history;
```

(Alphabetic order is approximately preserved; no need to re-sort.)

- [ ] **Step 4.3: Write the failing tests for `crypto.rs`**

Create `src/history/crypto.rs` with the following content (test-only at first; implementation follows):

```rust
//! Argon2id KDF + ChaCha20-Poly1305 AEAD for encrypted history.
//!
//! The keyfile (`<config_dir>/.history-key`) is 32 random bytes from
//! `OsRng`. Argon2id derives a 32-byte ChaCha20-Poly1305 key from
//! `(keyfile_bytes, FIXED_SALT)`. Each encrypted row carries a fresh
//! 12-byte nonce.

use std::path::{Path, PathBuf};

use argon2::Argon2;
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use rand::rngs::OsRng;
use rand::RngCore;
use zeroize::Zeroizing;

use crate::error::TranslateError;

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn round_trip_encrypts_and_decrypts() {
        let secret = Zeroizing::new([1u8; 32]);
        let key = derive_key(&secret).expect("derivation works");
        let plaintext = b"Hello, history!";
        let (ciphertext, nonce) = encrypt(&key, plaintext).expect("encrypt works");
        let decrypted = decrypt(&key, &ciphertext, &nonce).expect("decrypt works");
        assert_eq!(&decrypted[..], plaintext);
    }

    #[test]
    fn each_encrypt_uses_fresh_nonce() {
        let secret = Zeroizing::new([2u8; 32]);
        let key = derive_key(&secret).unwrap();
        let pt = b"same plaintext";
        let (_c1, n1) = encrypt(&key, pt).unwrap();
        let (_c2, n2) = encrypt(&key, pt).unwrap();
        assert_ne!(n1, n2, "two encrypts must produce different nonces");
    }

    #[test]
    fn decrypt_with_wrong_key_fails() {
        let secret_a = Zeroizing::new([3u8; 32]);
        let secret_b = Zeroizing::new([4u8; 32]);
        let key_a = derive_key(&secret_a).unwrap();
        let key_b = derive_key(&secret_b).unwrap();
        let (ct, n) = encrypt(&key_a, b"top secret").unwrap();
        let err = decrypt(&key_b, &ct, &n).unwrap_err();
        assert!(matches!(err, TranslateError::History(_)));
    }

    #[test]
    fn decrypt_rejects_tampered_ciphertext() {
        let secret = Zeroizing::new([5u8; 32]);
        let key = derive_key(&secret).unwrap();
        let (mut ct, n) = encrypt(&key, b"do not flip my bits").unwrap();
        // Flip a bit.
        ct[0] ^= 0x01;
        let err = decrypt(&key, &ct, &n).unwrap_err();
        assert!(matches!(err, TranslateError::History(_)));
    }

    #[test]
    fn argon2_derivation_is_deterministic() {
        // Spec exit criterion §M5 #6: same secret + same salt → same key.
        let secret = Zeroizing::new([7u8; 32]);
        let k1 = derive_key(&secret).unwrap();
        let k2 = derive_key(&secret).unwrap();
        assert_eq!(k1.as_slice(), k2.as_slice());
    }

    #[test]
    fn argon2_derivation_differs_per_secret() {
        let s1 = Zeroizing::new([10u8; 32]);
        let s2 = Zeroizing::new([11u8; 32]);
        assert_ne!(
            derive_key(&s1).unwrap().as_slice(),
            derive_key(&s2).unwrap().as_slice()
        );
    }

    #[test]
    fn keyfile_creates_with_owner_only_perms_on_first_open() {
        let dir = TempDir::new().unwrap();
        let kf = dir.path().join(".history-key");
        assert!(!kf.exists(), "precondition: keyfile must not exist");
        let secret = load_or_create_keyfile(&kf).unwrap();
        assert!(kf.exists(), "keyfile must be created on first open");
        // Bytes look random.
        assert_eq!(secret.len(), 32);
        assert_ne!(*secret, [0u8; 32], "secret must not be all zeros");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&kf).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
    }

    #[test]
    fn keyfile_is_stable_across_reopens() {
        let dir = TempDir::new().unwrap();
        let kf = dir.path().join(".history-key");
        let s1 = load_or_create_keyfile(&kf).unwrap();
        let s2 = load_or_create_keyfile(&kf).unwrap();
        assert_eq!(s1.as_slice(), s2.as_slice());
    }

    #[test]
    fn keyfile_with_wrong_size_returns_history_error() {
        let dir = TempDir::new().unwrap();
        let kf = dir.path().join(".history-key");
        std::fs::write(&kf, b"too short").unwrap();
        let err = load_or_create_keyfile(&kf).unwrap_err();
        assert!(matches!(err, TranslateError::History(_)));
    }

    #[test]
    fn load_or_create_creates_parent_dir_if_missing() {
        let dir = TempDir::new().unwrap();
        let nested = dir.path().join("does/not/exist/.history-key");
        let _ = load_or_create_keyfile(&nested).unwrap();
        assert!(nested.exists());
    }
}
```

- [ ] **Step 4.4: Run tests to verify failure**

```bash
cargo test --lib history::crypto 2>&1 | tail -10
```
Expected: compile errors — none of the functions are defined yet.

- [ ] **Step 4.5: Implement `derive_key` / `encrypt` / `decrypt`**

Append to `src/history/crypto.rs` (above the `#[cfg(test)] mod tests`):

```rust
/// Salt for Argon2id derivation. Fixed-per-binary; rationale in the
/// plan's cross-cutting decisions §1. The 16-byte length satisfies
/// argon2's minimum salt length.
const ARGON2_SALT: &[u8; 16] = b"clipt9n-history\0";

/// Derive a 32-byte ChaCha20-Poly1305 key from the keyfile secret via
/// Argon2id with default parameters.
pub fn derive_key(secret: &Zeroizing<[u8; 32]>) -> Result<Zeroizing<[u8; 32]>, TranslateError> {
    let argon2 = Argon2::default();
    let mut out = Zeroizing::new([0u8; 32]);
    argon2
        .hash_password_into(secret.as_slice(), ARGON2_SALT, out.as_mut())
        .map_err(|e| TranslateError::History(format!("argon2 derive: {e}")))?;
    Ok(out)
}

/// Encrypt `plaintext` with `key` and a fresh OsRng-generated 12-byte
/// nonce. Returns (ciphertext, nonce) — the caller stores both columns.
pub fn encrypt(
    key: &Zeroizing<[u8; 32]>,
    plaintext: &[u8],
) -> Result<(Vec<u8>, [u8; 12]), TranslateError> {
    let cipher = ChaCha20Poly1305::new(Key::from_slice(key.as_slice()));
    let mut nonce_bytes = [0u8; 12];
    OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ct = cipher
        .encrypt(nonce, plaintext)
        .map_err(|e| TranslateError::History(format!("encrypt: {e}")))?;
    Ok((ct, nonce_bytes))
}

/// Decrypt `ciphertext` with `key` and `nonce`. Returns the plaintext
/// wrapped in `Zeroizing` so callers don't need to remember to wrap it
/// (per spec §9 / cross-cutting decision §12).
pub fn decrypt(
    key: &Zeroizing<[u8; 32]>,
    ciphertext: &[u8],
    nonce: &[u8; 12],
) -> Result<Zeroizing<Vec<u8>>, TranslateError> {
    let cipher = ChaCha20Poly1305::new(Key::from_slice(key.as_slice()));
    let nonce = Nonce::from_slice(nonce);
    let pt = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|e| TranslateError::History(format!("decrypt: {e}")))?;
    Ok(Zeroizing::new(pt))
}
```

- [ ] **Step 4.6: Implement `load_or_create_keyfile`**

Append to `src/history/crypto.rs`:

```rust
/// Load the 32-byte secret from `path`, or create it if missing.
///
/// On creation: parent dirs are made (mode default), the file is
/// written with random bytes, and Unix mode is set to `0o600`. On
/// subsequent calls the file is read as-is; a wrong size triggers a
/// `History` error rather than overwriting (data preservation: the
/// caller may still want the existing file's bytes for forensic
/// purposes even if they're corrupt).
pub fn load_or_create_keyfile(path: &Path) -> Result<Zeroizing<[u8; 32]>, TranslateError> {
    if path.exists() {
        let bytes = std::fs::read(path).map_err(|e| {
            TranslateError::History(format!("reading keyfile {}: {e}", path.display()))
        })?;
        if bytes.len() != 32 {
            return Err(TranslateError::History(format!(
                "keyfile at {} has wrong size: expected 32 bytes, got {}",
                path.display(),
                bytes.len()
            )));
        }
        let mut secret = Zeroizing::new([0u8; 32]);
        secret.copy_from_slice(&bytes);
        return Ok(secret);
    }

    // Create.
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            TranslateError::History(format!(
                "creating parent dir {}: {e}",
                parent.display()
            ))
        })?;
    }
    let mut secret = Zeroizing::new([0u8; 32]);
    OsRng.fill_bytes(secret.as_mut());
    std::fs::write(path, secret.as_slice()).map_err(|e| {
        TranslateError::History(format!("writing keyfile {}: {e}", path.display()))
    })?;
    set_keyfile_permissions(path)?;
    tracing::warn!(
        path = %path.display(),
        "history-key created at {}; M6 will migrate this to OS keychain",
        path.display()
    );
    Ok(secret)
}

#[cfg(unix)]
fn set_keyfile_permissions(path: &Path) -> Result<(), TranslateError> {
    crate::platform::set_owner_only_permissions(path).map_err(|e| {
        TranslateError::History(format!("chmod 0o600 on {}: {e}", path.display()))
    })
}

#[cfg(not(unix))]
fn set_keyfile_permissions(_path: &Path) -> Result<(), TranslateError> {
    // Spec §7: on Windows the keyfile inherits the user's profile ACL.
    // Documented in README that this is less secure than M6's keychain
    // mode. No action here.
    Ok(())
}

/// Convenience: load (or create) the keyfile and immediately derive
/// the AEAD key. Used by `History::open` so callers don't have to
/// chain the two calls.
pub fn load_and_derive(keyfile_path: &Path) -> Result<Zeroizing<[u8; 32]>, TranslateError> {
    let secret = load_or_create_keyfile(keyfile_path)?;
    derive_key(&secret)
}

/// Compute a default keyfile path: `<config_dir>/.history-key`.
/// Exposed because `lib.rs::run` and `main.rs` both compute it; keeping
/// the construction in one place avoids drift.
pub fn default_keyfile_path(config_dir: &Path) -> PathBuf {
    config_dir.join(".history-key")
}
```

- [ ] **Step 4.7: Expose `set_owner_only_permissions` from `platform/mod.rs`**

Currently `src/platform/unix.rs::set_owner_only_permissions` is `pub(crate)`. The crypto module references it via `crate::platform::set_owner_only_permissions`, so we need a re-export in `src/platform/mod.rs`:

Add inside `#[cfg(unix)]` block at the end of `src/platform/mod.rs` (after the `install_sighup_reload` impls):

```rust
#[cfg(unix)]
pub(crate) use unix::set_owner_only_permissions;
```

This keeps the symbol scoped to the crate (no external API surface) while letting `history::crypto` call it without a `cfg(unix)` block in the history module.

- [ ] **Step 4.8: Run tests to verify pass**

```bash
cargo test --lib history::crypto 2>&1 | tail -15
```
Expected: 9 tests, 9 passing.

- [ ] **Step 4.9: Commit**

```bash
git add src/history/ src/lib.rs src/platform/mod.rs
git commit -m "feat(M5): history::crypto — Argon2id KDF + ChaCha20-Poly1305 AEAD + keyfile"
```

---

## Task 5: Build `src/history/store.rs` — schema + `History::open` + `in_memory`

**Files:**
- Create: `src/history/store.rs`

**Why:** The schema is the second foundation. `History::open` consumes the derived key from Task 4 and a path, sets up the SQLite table + index, and returns a `History` value. Insert/query/delete come in Task 6 — splitting them keeps each step's diff scoped.

- [ ] **Step 5.1: Write the failing tests for `store.rs` (open + schema)**

Create `src/history/store.rs`:

```rust
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

impl History {
    /// Open the SQLite database at `path` and run schema migrations.
    /// `key` must be the derived AEAD key from `crypto::derive_key` (or
    /// `crypto::load_and_derive`).
    pub fn open(path: &Path, key: Zeroizing<[u8; 32]>) -> Result<Self, TranslateError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                TranslateError::History(format!(
                    "creating history dir {}: {e}",
                    parent.display()
                ))
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
```

- [ ] **Step 5.2: Run tests to verify pass**

```bash
cargo test --lib history::store 2>&1 | tail -15
```
Expected: 6 tests, 6 passing. (The implementation is in the same step because the test scaffolding requires the types — true TDD red→green is not feasible for a fresh module with multiple type defs; the tests above act as the spec for Task 6's CRUD.)

- [ ] **Step 5.3: Commit**

```bash
git add src/history/store.rs
git commit -m "feat(M5): history::store — schema + History::open + History::in_memory"
```

---

## Task 6: `History::insert` + `query` + `delete` + `clear_all`

**Files:**
- Modify: `src/history/store.rs`

**Why:** With the schema in place, the four CRUD operations land together because they share helpers (the encrypt/decrypt path, the `NewEntry → row mapping`). Each gets its own test, but the implementation is one tight diff.

- [ ] **Step 6.1: Write the failing tests**

Append to `src/history/store.rs`'s `tests` mod:

```rust
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
        h.insert(fixture_entry("translate", "Hello", "Hallo")).unwrap();
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
        h.insert(fixture_entry("translate", "Smart Table demo", "Smart Table Demo")).unwrap();
        h.insert(fixture_entry("rewrite", "completely different", "noise")).unwrap();

        let rows = h
            .query(&QueryFilter { query: Some("smart".into()) }, 100)
            .unwrap();
        assert_eq!(rows.len(), 1, "only one row contains 'smart' (case-insensitive)");
    }

    #[test]
    fn query_filter_matches_action_label() {
        let h = History::in_memory(test_key()).unwrap();
        h.insert(fixture_entry("translate", "a", "b")).unwrap();
        h.insert(fixture_entry("rewrite", "c", "d")).unwrap();
        let rows = h.query(&QueryFilter { query: Some("rewr".into()) }, 100).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].action, "rewrite");
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
        h.insert(fixture_entry("translate", "after", "clear")).unwrap();
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
            h.insert(fixture_entry("translate", "persisted", "encrypted")).unwrap();
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
            h.insert(fixture_entry("translate", "tagged-A", "result-A")).unwrap();
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
```

- [ ] **Step 6.2: Run tests to verify failure**

```bash
cargo test --lib history::store 2>&1 | tail -15
```
Expected: compilation errors on `insert`, `query`, `delete`, `clear_all`, `insert_with_cap` (functions don't exist).

- [ ] **Step 6.3: Implement the CRUD methods**

Append to `src/history/store.rs`'s `impl History`:

```rust
    /// Encrypt the source/result fields and persist the row. Returns
    /// the new row's `id` (test convenience; production callers don't
    /// usually consume it).
    pub fn insert(&self, entry: NewEntry) -> Result<i64, TranslateError> {
        let (source_ct, source_nonce) = encrypt_optional(&self.key, entry.source.as_deref())?;
        let (result_ct, result_nonce) = encrypt_optional(&self.key, entry.result.as_deref())?;
        let conn = self.conn.lock().expect("history mutex poisoned");
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
    }

    /// Insert with retention cap. After insert, prune by `created_at`
    /// ASC so the table never holds more than `max_entries`. Used by
    /// the App's per-translation hook so users don't see unbounded
    /// growth past `[history] max_entries` (default 100).
    pub fn insert_with_cap(&self, entry: NewEntry, max_entries: usize) -> Result<i64, TranslateError> {
        let id = self.insert(entry)?;
        let conn = self.conn.lock().expect("history mutex poisoned");
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
        let conn = self.conn.lock().expect("history mutex poisoned");
        let mut stmt = conn
            .prepare(
                "SELECT id, created_at, action, source_lang, target_lang, char_count,
                        source_ciphertext, source_nonce, result_ciphertext, result_nonce
                 FROM entries
                 ORDER BY created_at DESC
                 LIMIT ?1",
            )
            .map_err(|e| TranslateError::History(format!("prepare: {e}")))?;
        let rows = stmt
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
            .map_err(|e| TranslateError::History(format!("query: {e}")))?;

        let mut out = Vec::new();
        for raw in rows {
            let raw = raw.map_err(|e| TranslateError::History(format!("row: {e}")))?;
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
        let conn = self.conn.lock().expect("history mutex poisoned");
        conn.execute("DELETE FROM entries WHERE id = ?1", [id])
            .map_err(|e| TranslateError::History(format!("delete: {e}")))?;
        Ok(())
    }

    /// Wipe every row. Spec §7: "Clear all deletes all rows but leaves
    /// the encryption key in place." The keyfile is untouched.
    pub fn clear_all(&self) -> Result<(), TranslateError> {
        let conn = self.conn.lock().expect("history mutex poisoned");
        conn.execute("DELETE FROM entries", [])
            .map_err(|e| TranslateError::History(format!("clear_all: {e}")))?;
        Ok(())
    }
```

Add module-private helpers above the `tests` mod:

```rust
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
) -> Result<(Option<Vec<u8>>, Option<[u8; 12]>), TranslateError> {
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
```

- [ ] **Step 6.4: Run tests to verify pass**

```bash
cargo test --lib history::store 2>&1 | tail -20
```
Expected: 17 tests, 17 passing.

- [ ] **Step 6.5: Commit**

```bash
git add src/history/store.rs
git commit -m "feat(M5): history::store CRUD — insert, query, delete, clear_all, retention cap"
```

---

## Task 7: Wire `History` into `App::new` and `lib.rs::run`

**Files:**
- Modify: `src/app.rs` (`ClipApp` struct + `new` + new `history_disabled` flag)
- Modify: `src/main.rs` (open History before `eframe::run_native`)
- Modify: `src/lib.rs` (CLI run path opens History but doesn't insert)

**Why:** With the History type ready, the app can hold `Arc<Option<History>>` and route writes through it. Per spec §8, `History::open` failures don't block startup — the app sets `history_disabled = true` (an `AtomicBool` shared with worker tasks) and continues. The CLI path opens History purely so corruption is surfaced at first run; CLI translations don't write history rows (M1 design — CLI is one-shot).

- [ ] **Step 7.1: Modify `ClipApp` to hold `Arc<Option<History>>` + disabled flag**

In `src/app.rs`, modify the `ClipApp` struct (around line 64) to add three new fields:

```rust
    /// Encrypted history store. `None` means history was disabled at
    /// config time (`[history] enabled = false`) OR open failed at
    /// startup. The `Arc<History>` lets worker tasks clone the handle
    /// cheaply; `Option` is the "no store" shortcut. `#[allow(dead_code)]`
    /// is temporary — Task 8 reads this in `schedule_history_insert`,
    /// Task 10 reads it in `summon_history` / `update_showing_history`.
    #[allow(dead_code)]
    history: Option<std::sync::Arc<crate::history::store::History>>,

    /// Set to true if history-side errors should short-circuit. Read
    /// by the insert path (atomic check, no lock) and the viewer path
    /// (which surfaces the corruption toast). The flag persists for
    /// the life of the app — the user must restart after fixing the DB
    /// to re-enable history.
    #[allow(dead_code)]
    history_disabled: std::sync::Arc<std::sync::atomic::AtomicBool>,

    /// Set to true the first time the corruption toast has been shown
    /// to the user. Mirrors M4's "warned once per session" pattern.
    #[allow(dead_code)]
    history_warned: std::sync::atomic::AtomicBool,
```

- [ ] **Step 7.2: Update `ClipApp::new` to accept the new args**

Modify the `new` signature (around line 134):

```rust
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        cc: &CreationContext<'_>,
        cfg: Config,
        provider: std::sync::Arc<dyn LlmProvider>,
        templates: std::sync::Arc<crate::llm::templates::Templates>,
        glossary: std::sync::Arc<std::sync::RwLock<crate::glossary::Glossary>>,
        glossary_path: PathBuf,
        glossary_reload_rx: CrossbeamReceiver<()>,
        history: Option<std::sync::Arc<crate::history::store::History>>,
        history_disabled_initial: bool,
        _secrets: Box<dyn Secrets>,
        state_path: PathBuf,
        hotkey_rx: CrossbeamReceiver<GlobalHotKeyEvent>,
        prompt_hotkey_id: u32,
        history_hotkey_id: Option<u32>,
    ) -> Self {
```

(Three additions: `history`, `history_disabled_initial`, plus the two hotkey IDs that Task 10 also reads.)

Inside `new`, in the `Self { ... }` literal, add the new fields:

```rust
            history,
            history_disabled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(
                history_disabled_initial,
            )),
            history_warned: std::sync::atomic::AtomicBool::new(false),
            prompt_hotkey_id,
            history_hotkey_id,
```

`prompt_hotkey_id` + `history_hotkey_id` are also new fields on `ClipApp`; declare them in the struct literal (Task 10 wires the routing logic; declaring here keeps the constructor stable).

In the struct definition (continuing the additions from Step 7.1), add:

```rust
    /// `global-hotkey` ID for the prompt hotkey. Always set (the prompt
    /// hotkey is always registered). `#[allow(dead_code)]` is temporary
    /// — Task 10 reads this in `drain_channels` to route hotkey events.
    #[allow(dead_code)]
    prompt_hotkey_id: u32,
    /// `global-hotkey` ID for the history hotkey. `None` if the user
    /// disabled it via `[hotkey.history] enabled = false`. `#[allow(dead_code)]`
    /// is temporary — Task 10 reads this in `drain_channels`.
    #[allow(dead_code)]
    history_hotkey_id: Option<u32>,
```

(Drop the two `#[allow(dead_code)]` attributes in Task 10 when these fields gain readers.)

- [ ] **Step 7.3: Open History in `main.rs` before `eframe::run_native`**

In `src/main.rs`, after the glossary/templates loading block (around line 60, before the platform precondition check), add:

```rust
    // Encrypted history (M5). Graceful: open failure → log warn + run
    // with history disabled. Spec §8 corruption + missing-key rows.
    let history_path = config_dir.join("history.db");
    let keyfile_path = clipt9n::history::crypto::default_keyfile_path(&config_dir);
    let (history, history_disabled_initial): (
        Option<std::sync::Arc<clipt9n::history::store::History>>,
        bool,
    ) = if cfg.history.enabled {
        match clipt9n::history::crypto::load_and_derive(&keyfile_path) {
            Ok(key) => match clipt9n::history::store::History::open(&history_path, key) {
                Ok(h) => (Some(std::sync::Arc::new(h)), false),
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        path = %history_path.display(),
                        "history open failed; running with history disabled"
                    );
                    (None, true)
                }
            },
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    path = %keyfile_path.display(),
                    "history keyfile load failed; running with history disabled"
                );
                (None, true)
            }
        }
    } else {
        tracing::info!("history disabled by config; skipping open");
        (None, false)
    };
```

- [ ] **Step 7.4: Pass new args to `ClipApp::new`**

Modify the `Box::new(move |cc| { ... ClipApp::new(...) })` call near the end of `main.rs` to thread the new args. The `prompt_hotkey_id` / `history_hotkey_id` come from Task 10's hotkey-registration changes — for now, pass `0` and `None` as placeholders so this task compiles independently. Task 10 will wire the real IDs.

```rust
    eframe::run_native(
        "clipt9n",
        native_options,
        Box::new(move |cc| {
            let app = ClipApp::new(
                cc,
                cfg,
                provider,
                templates,
                glossary,
                glossary_path,
                glossary_reload_rx,
                history,
                history_disabled_initial,
                secrets,
                state_path,
                hotkey_rx,
                0,    // Task 10 will replace with real prompt hotkey id
                None, // Task 10 will replace with real history hotkey id
            );
            app.install_glossary_reload(glossary_reload_tx);
            Ok(Box::new(app))
        }),
    )
    .map_err(|e| anyhow::anyhow!("eframe: {e}"))?;
```

(Task 10 replaces the literal `0` / `None` after registering the second hotkey.)

- [ ] **Step 7.5: Open History (silent on Err) in `lib.rs::run`**

In `src/lib.rs::run`, after the glossary load (around line 149), add:

```rust
    // History (M5): open if enabled. CLI mode never inserts; we open so
    // a corrupt DB is surfaced at first user contact rather than
    // silently failing on the first GUI translation.
    if cfg.history.enabled {
        let history_path = config_dir.join("history.db");
        let keyfile_path = crate::history::crypto::default_keyfile_path(&config_dir);
        if let Err(e) = crate::history::crypto::load_and_derive(&keyfile_path)
            .and_then(|key| crate::history::store::History::open(&history_path, key))
            .map(|_| ())
        {
            tracing::warn!(error = %e, "CLI history open failed (non-fatal)");
        }
    }
```

The CLI never holds the History after this; the open is just a smoke-check.

- [ ] **Step 7.6: Run tests + build**

```bash
cargo build 2>&1 | tail -3
cargo test --all-features 2>&1 | grep "test result:"
```
Expected: build clean; tests still passing (no behavioral change yet — wiring only).

- [ ] **Step 7.7: Commit**

```bash
git add src/app.rs src/main.rs src/lib.rs
git commit -m "feat(M5): wire History into ClipApp + main.rs (graceful corruption)"
```

---

## Task 8: Schedule history insert on translation success

**Files:**
- Modify: `src/app.rs` (`handle_translation_done` + `start_translation` capture detected_lang)

**Why:** Spec §8 + cross-cutting decision §5 / §10: history writes are best-effort, run on the tokio runtime, and never block the clipboard-write path. A panic during insert is caught by a watcher (M3 pattern) and converted to a warn log. The clipboard write happens BEFORE the insert is scheduled — the insert-spawn observes the already-updated clipboard but the clipboard was the user's outcome regardless of insert success.

- [ ] **Step 8.1: Capture `detected_source_lang` in the worker outcome**

In `src/app.rs`, modify the `TranslationOutcome` struct (around line 122) to carry the detected ISO-2 language:

```rust
#[derive(Debug)]
struct TranslationOutcome {
    result: Result<String, TranslateError>,
    action_label: String,
    slot: u8,
    /// Dispatch-generation that produced this outcome.
    gen: u64,
    /// ISO-2 source language detected at dispatch time (carries into
    /// the history row's `source_lang` column on success). `None` if
    /// `whatlang` confidence was below the threshold.
    detected_source_lang: Option<String>,
    /// The source text we fed to the translator. The history insert
    /// path uses this to compute `char_count` and (when
    /// `[history] store_text = true`) to encrypt as the source column.
    source_text: String,
    /// Action that produced this outcome — used to fill the history
    /// row's `action` and `target_lang` columns. Cloned at dispatch
    /// time so the worker doesn't hold a `&Action` reference.
    action: Action,
}
```

In `start_translation` (around line 292), capture both fields:

```rust
        let detected_source = self.prompt_model.detected_lang.clone();
        let action_for_outcome = action.clone();
        let source_text_for_outcome = source_text.clone();
        // ... existing code that builds the worker spawn ...
        let worker = self.runtime.spawn(async move {
            let g_snapshot = glossary.read().expect("glossary RwLock poisoned").clone();
            let translator = Translator::new(&cfg, provider.as_ref(), &templates, &g_snapshot);
            let result = translator.execute(&action, &source_text).await;
            TranslationOutcome {
                result,
                action_label,
                slot,
                gen,
                detected_source_lang: detected_source,
                source_text: source_text_for_outcome,
                action: action_for_outcome,
            }
        });
```

Update the panic-watcher's `TranslationOutcome` literal too (around line 348) to include the new fields:

```rust
                    TranslationOutcome {
                        result: Err(TranslateError::Internal(format!(
                            "translation worker crashed: {join_err}"
                        ))),
                        action_label: label_for_panic,
                        slot,
                        gen,
                        detected_source_lang: None,
                        source_text: String::new(),
                        action: Action::FixGrammar, // placeholder — never read on Err
                    }
```

(Need to clone `action` and `source_text` into local variables earlier in `start_translation` so they're available in BOTH the worker spawn AND the watcher closure. Currently `source_text` and `action` are moved into the worker. Clone them up front: `let action_for_outcome = action.clone();` and `let source_text_for_outcome = source_text.clone();` BEFORE the worker spawn.)

`Action` already derives `Clone` (line 21 of translator.rs) — verify:

```bash
grep -n "Clone" src/translator.rs | head -3
```

If `Action` doesn't have `Clone`, add it:

```rust
#[derive(Debug, Clone)]
pub enum Action { ... }
```

(It already does per the M4-shipped definition — confirm in the inspect step.)

- [ ] **Step 8.2: Add `schedule_history_insert` helper**

Add to `impl ClipApp` (around line 460, near `reload_glossary`):

```rust
    /// Best-effort: persist a successful translation to the history
    /// store. Runs on the tokio runtime. Failures are logged at warn;
    /// panics are caught by the watcher and similarly logged. The user
    /// never sees a toast — the clipboard write is the primary outcome
    /// and has already happened by the time we get here.
    fn schedule_history_insert(&self, outcome: &TranslationOutcome, translated: &str) {
        // Short-circuit if history is disabled (config or corruption).
        let Some(history) = self.history.clone() else {
            return;
        };
        if self
            .history_disabled
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            return;
        }
        let history_disabled = self.history_disabled.clone();
        let max_entries = self.cfg.history.max_entries;
        let store_text = self.cfg.history.store_text;
        let source_text = outcome.source_text.clone();
        let result_text = translated.to_string();
        let action = outcome.action.clone();
        let detected = outcome.detected_source_lang.clone();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        let entry = crate::history::store::NewEntry {
            created_at: now,
            action: action_kind_str(&action).to_string(),
            source_lang: detected,
            target_lang: target_lang_for(&action),
            char_count: source_text.chars().count() as i64,
            source: if store_text { Some(source_text) } else { None },
            result: if store_text { Some(result_text) } else { None },
        };

        let inner = self.runtime.spawn(async move {
            // history is Arc<History> (already unwrapped from the Option
            // above). insert_with_cap takes &self.
            if let Err(e) = history.insert_with_cap(entry, max_entries) {
                tracing::warn!(error = %e, "history insert failed; row dropped");
                // Don't disable globally — a transient SQLite error
                // (e.g., disk full) shouldn't permanently take down
                // history. Corruption-class errors set the flag at
                // open time, not at insert time.
                let _ = history_disabled; // suppress unused warning
            }
        });
        // Watcher: catch a panic in the inner task and log it.
        self.runtime.spawn(async move {
            if let Err(join_err) = inner.await {
                tracing::warn!(
                    error = %join_err,
                    "history insert panicked; row dropped"
                );
            }
        });
    }
```

Add the small helpers near the bottom of `app.rs` (above `mod tests`, around line 855):

```rust
/// Map an `Action` to the string we persist in `entries.action`. Must
/// match the `'translate' | 'fix_grammar' | 'rewrite' | 'custom'`
/// alphabet from spec §7.
fn action_kind_str(action: &Action) -> &'static str {
    match action {
        Action::Translate { .. } => "translate",
        Action::FixGrammar => "fix_grammar",
        Action::Rewrite => "rewrite",
        Action::Custom { .. } => "custom",
    }
}

/// Target language for the history row. `None` for fix_grammar /
/// rewrite / custom (which stay in source language); `Some(code)` for
/// translate.
fn target_lang_for(action: &Action) -> Option<String> {
    match action {
        Action::Translate { code } => Some(code.clone()),
        _ => None,
    }
}
```

- [ ] **Step 8.3: Call `schedule_history_insert` after a successful clipboard write**

In `handle_translation_done` (around line 363), after the successful clipboard write block:

```rust
                if let Err(e) = cb.write_text(&translated) {
                    tracing::error!(error = %e, "clipboard write failed");
                } else {
                    if let Err(e) = crate::notify::translation_copied(&outcome.action_label) {
                        tracing::warn!(error = %e, "notification failed");
                    }
                    // History insert: best-effort, AFTER clipboard write
                    // succeeds, NEVER blocks the user's primary outcome.
                    self.schedule_history_insert(&outcome, &translated);
                }
```

(Restructure the existing `else if` chain into nested `if`/`else` so the history insert runs only when clipboard write succeeds. Keep the existing notification fallthrough order.)

- [ ] **Step 8.4: Add a unit test for `action_kind_str` + `target_lang_for`**

Append to `src/app.rs::tests`:

```rust
    #[test]
    fn action_kind_str_maps_per_spec() {
        assert_eq!(action_kind_str(&Action::Translate { code: "de".into() }), "translate");
        assert_eq!(action_kind_str(&Action::FixGrammar), "fix_grammar");
        assert_eq!(action_kind_str(&Action::Rewrite), "rewrite");
        assert_eq!(
            action_kind_str(&Action::Custom { instruction: "x".into() }),
            "custom"
        );
    }

    #[test]
    fn target_lang_for_only_set_on_translate() {
        assert_eq!(
            target_lang_for(&Action::Translate { code: "de".into() }),
            Some("de".to_string())
        );
        assert_eq!(target_lang_for(&Action::FixGrammar), None);
        assert_eq!(target_lang_for(&Action::Rewrite), None);
        assert_eq!(
            target_lang_for(&Action::Custom { instruction: "x".into() }),
            None
        );
    }
```

- [ ] **Step 8.5: Run tests + build**

```bash
cargo test --all-features 2>&1 | grep "test result:"
cargo clippy --all-features --all-targets -- -D warnings 2>&1 | tail -3
```
Expected: tests pass; clippy clean.

- [ ] **Step 8.6: Commit**

```bash
git add src/app.rs
git commit -m "feat(M5): schedule encrypted history insert after clipboard write"
```

---

## Task 9: Build `src/ui/history.rs` — viewer model + draw + keyboard handling + modal

**Files:**
- Create: `src/ui/history.rs`
- Modify: `src/ui/mod.rs` (`pub mod history`)

**Why:** This is the user-facing surface — the design's `history-window.jsx` translated to egui. Per the design: 680px window, top search input, scrollable list with 4-column layout (mark / ago / pair / text), detail block below selection, footer keymap, and a Shift+Del confirmation modal. All UI is paint-only; the App owns the keyboard event routing (Task 10) and the actual `History` queries.

The view is a pure function of `HistoryModel`. Width / scroll cap match the design (680px outer, ~250px list area). egui-specific deviations from the JSX are documented inline.

- [ ] **Step 9.1: Wire the module**

In `src/ui/mod.rs`:

```rust
pub mod custom_prompt;
pub mod history;
pub mod prompt;
pub mod size_confirm;
pub mod theme;
pub mod translating;
```

- [ ] **Step 9.2: Write the failing tests for the model + filtering helpers**

Create `src/ui/history.rs`:

```rust
//! History viewer window — egui paint of the design's
//! `history-window.jsx`. Pure view + small pure helpers for ago-
//! formatting and pair-label rendering. Keyboard event routing and the
//! actual store queries live in `src/app.rs`.

use std::sync::Arc;

use egui::{Color32, Key, RichText, ScrollArea, Sense, Stroke, TextEdit, Vec2};
use zeroize::Zeroizing;

use crate::history::store::HistoryEntry;
use crate::ui::theme;

/// What the viewer paints per frame.
pub struct HistoryModel {
    /// All entries returned by the most recent `History::query` call.
    /// The viewer applies the search filter on top of this in-memory
    /// (cheap; we have ≤max_entries items, default 100).
    pub entries: Vec<HistoryEntry>,
    /// Live search query. Refreshed every frame from the text edit.
    pub query: String,
    /// Index into the *filtered* list (recomputed each frame). 0 if
    /// the list is empty.
    pub selected: usize,
    /// Whether the Shift+Del confirmation modal is visible.
    pub confirm_clear: bool,
    /// Set true once on first paint after a corruption-state open;
    /// the viewer renders a one-shot warning banner and the App
    /// flips it false on the next frame so the toast doesn't loop.
    pub show_corruption_banner: bool,
}

impl Default for HistoryModel {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            query: String::new(),
            selected: 0,
            confirm_clear: false,
            show_corruption_banner: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryOutcome {
    /// Esc pressed (or Cancel button on the modal).
    Close,
    /// Enter on the focused row → copy result back to clipboard.
    CopyResult(i64),
    /// `s` pressed → copy original source.
    CopySource(i64),
    /// `d` pressed → delete the focused row.
    Delete(i64),
    /// Shift+Del confirmed → wipe all rows (key preserved).
    ClearAll,
}

/// Format `created_at` (unix epoch seconds) as a relative-time label
/// like "2 min ago", "1 h ago", "yesterday", or an ISO-ish "MMM DD"
/// for older entries. Uses `std::time::SystemTime::now()` so it's
/// pure-relative; in tests pass `Some(now)` to make it deterministic.
pub fn ago_label(created_at: i64, now_override: Option<i64>) -> String {
    let now = now_override.unwrap_or_else(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    });
    let delta = now.saturating_sub(created_at);
    if delta < 60 {
        "just now".into()
    } else if delta < 60 * 60 {
        format!("{} min ago", delta / 60)
    } else if delta < 60 * 60 * 24 {
        format!("{} h ago", delta / 3600)
    } else if delta < 60 * 60 * 48 {
        "yesterday".into()
    } else {
        format!("{} d ago", delta / (60 * 60 * 24))
    }
}

/// Format the pair label per the design's render: "DE → EN" for
/// translate, the lowercase action name otherwise.
pub fn pair_label(entry: &HistoryEntry) -> String {
    match entry.action.as_str() {
        "translate" => {
            let s = entry
                .source_lang
                .as_deref()
                .map(|c| c.to_uppercase())
                .unwrap_or_else(|| "??".into());
            let t = entry
                .target_lang
                .as_deref()
                .map(|c| c.to_uppercase())
                .unwrap_or_else(|| "??".into());
            format!("{s} → {t}")
        }
        "fix_grammar" => "grammar".into(),
        other => other.into(), // "rewrite" / "custom"
    }
}

/// Pair label color per design (lime / blue / purple / orange).
pub fn pair_color(entry: &HistoryEntry) -> Color32 {
    match entry.action.as_str() {
        "fix_grammar" => Color32::from_rgb(0x9a, 0xd6, 0xff),
        "rewrite" => Color32::from_rgb(0xd4, 0xa8, 0xff),
        "custom" => Color32::from_rgb(0xff, 0xb8, 0x4d),
        _ => theme::ACCENT,
    }
}

/// Filter entries against the current `query`. Mirrors the SQL-side
/// `filter_matches` but operates on already-decrypted entries (cheap
/// at viewer scale).
pub fn filter_entries<'a>(
    entries: &'a [HistoryEntry],
    query: &str,
) -> Vec<&'a HistoryEntry> {
    let q = query.trim();
    if q.is_empty() {
        return entries.iter().collect();
    }
    let q_lc = q.to_lowercase();
    entries
        .iter()
        .filter(|e| matches_lc(e, &q_lc))
        .collect()
}

fn matches_lc(entry: &HistoryEntry, q_lc: &str) -> bool {
    if entry.action.to_lowercase().contains(q_lc) {
        return true;
    }
    if let Some(s) = entry.source_lang.as_deref() {
        if s.to_lowercase().contains(q_lc) {
            return true;
        }
    }
    if let Some(t) = entry.target_lang.as_deref() {
        if t.to_lowercase().contains(q_lc) {
            return true;
        }
    }
    if let Some(s) = entry.source.as_ref() {
        if s.to_lowercase().contains(q_lc) {
            return true;
        }
    }
    if let Some(r) = entry.result.as_ref() {
        if r.to_lowercase().contains(q_lc) {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(action: &str, source: &str, result: &str, src_lang: Option<&str>, tgt_lang: Option<&str>) -> HistoryEntry {
        HistoryEntry {
            id: 1,
            created_at: 1_700_000_000,
            action: action.into(),
            source_lang: src_lang.map(String::from),
            target_lang: tgt_lang.map(String::from),
            char_count: source.chars().count() as i64,
            source: Some(Zeroizing::new(source.into())),
            result: Some(Zeroizing::new(result.into())),
        }
    }

    #[test]
    fn ago_label_under_60s_is_just_now() {
        assert_eq!(ago_label(1000, Some(1059)), "just now");
    }

    #[test]
    fn ago_label_minutes() {
        assert_eq!(ago_label(1000, Some(1000 + 5 * 60)), "5 min ago");
    }

    #[test]
    fn ago_label_hours() {
        assert_eq!(ago_label(1000, Some(1000 + 3 * 3600)), "3 h ago");
    }

    #[test]
    fn ago_label_yesterday() {
        let yesterday = 1000 + 30 * 3600;
        assert_eq!(ago_label(1000, Some(yesterday)), "yesterday");
    }

    #[test]
    fn ago_label_days() {
        let three_days = 1000 + 3 * 24 * 3600;
        assert_eq!(ago_label(1000, Some(three_days)), "3 d ago");
    }

    #[test]
    fn pair_label_translate_uses_uppercase_codes() {
        let e = entry("translate", "x", "y", Some("de"), Some("en"));
        assert_eq!(pair_label(&e), "DE → EN");
    }

    #[test]
    fn pair_label_fix_grammar_renders_as_grammar() {
        let e = entry("fix_grammar", "x", "y", Some("de"), None);
        assert_eq!(pair_label(&e), "grammar");
    }

    #[test]
    fn pair_label_rewrite_and_custom_pass_through() {
        let r = entry("rewrite", "x", "y", Some("en"), None);
        assert_eq!(pair_label(&r), "rewrite");
        let c = entry("custom", "x", "y", Some("en"), None);
        assert_eq!(pair_label(&c), "custom");
    }

    #[test]
    fn filter_returns_all_when_query_empty() {
        let entries = vec![
            entry("translate", "Hello", "Hallo", Some("en"), Some("de")),
            entry("rewrite", "x", "y", Some("en"), None),
        ];
        assert_eq!(filter_entries(&entries, "").len(), 2);
        assert_eq!(filter_entries(&entries, "   ").len(), 2);
    }

    #[test]
    fn filter_matches_decrypted_text_case_insensitive() {
        let entries = vec![
            entry("translate", "Smart Table demo", "Smart Table Demo", Some("en"), Some("de")),
            entry("rewrite", "noise", "more noise", Some("en"), None),
        ];
        let hits = filter_entries(&entries, "smart");
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn filter_matches_action() {
        let entries = vec![
            entry("translate", "x", "y", Some("en"), Some("de")),
            entry("rewrite", "z", "w", Some("en"), None),
        ];
        let hits = filter_entries(&entries, "rewr");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].action, "rewrite");
    }
}
```

- [ ] **Step 9.3: Run helper tests**

```bash
cargo test --lib ui::history 2>&1 | tail -10
```
Expected: 11 tests, all passing (model + helpers; the `draw` function comes next and is best smoke-tested manually + by build success).

- [ ] **Step 9.4: Implement the `draw` function**

Append to `src/ui/history.rs` (between `filter_entries` and `#[cfg(test)] mod tests`):

```rust
/// Paint the history viewer. Returns an outcome iff the user
/// triggered a transition this frame (Esc → Close, Enter → CopyResult,
/// etc.). The caller (App) is responsible for routing the outcome,
/// updating `model.selected` based on arrow-key navigation, and
/// re-querying the store after any deletion / clear.
pub fn draw(ctx: &egui::Context, model: &mut HistoryModel) -> Option<HistoryOutcome> {
    let mut outcome: Option<HistoryOutcome> = None;
    let frame = egui::CentralPanel::default()
        .frame(egui::Frame::new().fill(theme::PANEL).inner_margin(20.0));

    frame.show(ctx, |ui| {
        ui.set_max_width(640.0); // 680px outer - 2 * 20px margin
        // Title row.
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("History")
                    .color(theme::INK)
                    .strong()
                    .size(15.0),
            );
            ui.add_space(8.0);
            let count = model.entries.len();
            ui.label(
                RichText::new(format!("{count} entries · encrypted"))
                    .color(theme::INK_3)
                    .size(11.5),
            );
            ui.allocate_space(ui.available_size_before_wrap());
        });
        ui.add_space(10.0);

        // Optional corruption banner (one-shot).
        if model.show_corruption_banner {
            ui.label(
                RichText::new(
                    "History database unreadable. New history will not be saved.",
                )
                .color(theme::WARN)
                .size(11.5),
            );
            ui.add_space(8.0);
        }

        // Search row.
        ui.horizontal(|ui| {
            ui.label(RichText::new("⌕").color(theme::INK_3).monospace());
            ui.add_space(8.0);
            let edit = TextEdit::singleline(&mut model.query)
                .hint_text("type to filter…")
                .desired_width(ui.available_width() - 100.0);
            ui.add(edit);
        });
        ui.separator();

        // Filtered list.
        let filtered = filter_entries(&model.entries, &model.query);
        if model.selected >= filtered.len() && !filtered.is_empty() {
            model.selected = 0;
        }

        ScrollArea::vertical()
            .max_height(250.0)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if filtered.is_empty() {
                    ui.add_space(20.0);
                    ui.label(
                        RichText::new("No matches.")
                            .color(theme::INK_3)
                            .size(12.0),
                    );
                } else {
                    for (i, e) in filtered.iter().enumerate() {
                        let active = i == model.selected;
                        let bg = if active {
                            Color32::from_rgba_unmultiplied(200, 255, 94, 16)
                        } else {
                            Color32::TRANSPARENT
                        };
                        let resp = egui::Frame::new()
                            .fill(bg)
                            .inner_margin(6.0)
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new(if active { "▸" } else { " " })
                                            .color(theme::ACCENT)
                                            .monospace(),
                                    );
                                    ui.add_space(8.0);
                                    ui.label(
                                        RichText::new(ago_label(e.created_at, None))
                                            .color(theme::INK_3)
                                            .monospace()
                                            .size(11.0),
                                    );
                                    ui.add_space(8.0);
                                    ui.label(
                                        RichText::new(pair_label(e))
                                            .color(pair_color(e))
                                            .monospace()
                                            .size(11.0)
                                            .strong(),
                                    );
                                    ui.add_space(8.0);
                                    let truncated = e
                                        .source
                                        .as_ref()
                                        .map(|s| truncate_for_row(s, 64))
                                        .unwrap_or_else(|| "(text not stored)".into());
                                    ui.label(
                                        RichText::new(truncated)
                                            .color(if active {
                                                theme::INK
                                            } else {
                                                theme::INK_2
                                            })
                                            .size(12.0),
                                    );
                                });
                            })
                            .response
                            .interact(Sense::click());
                        if resp.clicked() {
                            model.selected = i;
                        }
                        if resp.double_clicked() {
                            outcome = Some(HistoryOutcome::CopyResult(e.id));
                        }
                    }
                }
            });

        // Detail block for selected row.
        if let Some(sel) = filtered.get(model.selected) {
            ui.add_space(10.0);
            egui::Frame::new()
                .fill(theme::PANEL_2)
                .stroke(Stroke::new(1.0, theme::LINE_SOFT))
                .inner_margin(12.0)
                .show(ui, |ui| {
                    ui.columns(2, |cols| {
                        cols[0].label(
                            RichText::new("SOURCE")
                                .color(theme::INK_3)
                                .monospace()
                                .size(10.0)
                                .strong(),
                        );
                        cols[0].add_space(4.0);
                        let src = sel
                            .source
                            .as_ref()
                            .map(|s| s.as_str().to_owned())
                            .unwrap_or_else(|| "(text not stored)".into());
                        cols[0].label(
                            RichText::new(src)
                                .color(theme::INK)
                                .monospace()
                                .size(11.5),
                        );
                        cols[1].label(
                            RichText::new("RESULT")
                                .color(theme::INK_3)
                                .monospace()
                                .size(10.0)
                                .strong(),
                        );
                        cols[1].add_space(4.0);
                        let res = sel
                            .result
                            .as_ref()
                            .map(|s| s.as_str().to_owned())
                            .unwrap_or_else(|| "(text not stored)".into());
                        cols[1].label(
                            RichText::new(res)
                                .color(theme::INK)
                                .monospace()
                                .size(11.5),
                        );
                    });
                });
        }

        // Footer keymap.
        ui.add_space(12.0);
        ui.separator();
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            theme::kbd(ui, "↵");
            ui.label(RichText::new("copy result").color(theme::INK_3).size(11.0));
            ui.add_space(12.0);
            theme::kbd(ui, "s");
            ui.label(RichText::new("copy source").color(theme::INK_3).size(11.0));
            ui.add_space(12.0);
            theme::kbd(ui, "d");
            ui.label(RichText::new("delete").color(theme::INK_3).size(11.0));
            ui.add_space(12.0);
            theme::kbd(ui, "⇧+Del");
            ui.label(RichText::new("clear all").color(theme::INK_3).size(11.0));
            ui.allocate_space(ui.available_size_before_wrap());
            theme::kbd(ui, "Esc");
            ui.label(RichText::new("close").color(theme::INK_3).size(11.0));
        });

        // Confirmation modal (rendered on top via egui::Window).
        if model.confirm_clear {
            egui::Window::new("clear_all_confirm")
                .title_bar(false)
                .resizable(false)
                .collapsible(false)
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .frame(
                    egui::Frame::new()
                        .fill(theme::PANEL)
                        .stroke(Stroke::new(1.0, theme::LINE))
                        .inner_margin(18.0),
                )
                .show(ctx, |ui| {
                    ui.set_min_width(360.0);
                    ui.label(
                        RichText::new("Clear all history?")
                            .color(theme::INK)
                            .strong()
                            .size(14.0),
                    );
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(format!(
                            "{} entries will be permanently removed. The encryption key stays in place.",
                            model.entries.len()
                        ))
                        .color(theme::INK_2)
                        .size(12.5),
                    );
                    ui.add_space(14.0);
                    ui.horizontal(|ui| {
                        ui.allocate_space(Vec2::new(
                            ui.available_width() - 200.0,
                            0.0,
                        ));
                        if ui.button("Cancel").clicked() {
                            model.confirm_clear = false;
                        }
                        ui.add_space(8.0);
                        let danger = egui::Button::new(
                            RichText::new("Clear all")
                                .color(theme::ACCENT_INK)
                                .strong(),
                        )
                        .fill(theme::BAD);
                        if ui.add(danger).clicked() {
                            outcome = Some(HistoryOutcome::ClearAll);
                            model.confirm_clear = false;
                        }
                    });
                });
        }
    });

    outcome
}

fn truncate_for_row(s: &str, max_chars: usize) -> String {
    let mut out = String::with_capacity(max_chars + 1);
    let mut count = 0;
    for ch in s.chars() {
        if count >= max_chars {
            out.push('…');
            return out;
        }
        out.push(ch);
        count += 1;
    }
    out
}
```

(The `Arc` import at the top of the file is unused — remove it after writing the draw function. Also remove unused imports until clippy passes.)

- [ ] **Step 9.5: Run tests + clippy**

```bash
cargo test --lib ui::history 2>&1 | tail -10
cargo clippy --all-features --all-targets -- -D warnings 2>&1 | tail -10
```
Expected: tests pass; clippy clean. Trim unused imports as needed.

- [ ] **Step 9.6: Commit**

```bash
git add src/ui/history.rs src/ui/mod.rs
git commit -m "feat(M5): ui::history viewer — design-faithful 680px window with search/list/detail"
```

---

## Task 10: `AppState::ShowingHistory` + viewport resize + state transitions + hotkey routing

**Files:**
- Modify: `src/app.rs` (AppState enum + update routing + hotkey ID matching)

**Why:** Final wiring: the AppState gets a new `ShowingHistory` variant; the update loop dispatches to a new `update_showing_history` method that calls `ui::history::draw` and routes its outcomes (CopyResult → clipboard write + close, CopySource → clipboard write + stay, Delete → re-query, ClearAll → re-query, Close → return to Idle). The viewport is resized to 680×540 on entry and back to the prompt size on exit. The drain_channels hotkey loop matches `event.id` against `prompt_hotkey_id` / `history_hotkey_id` to decide which surface to summon.

- [ ] **Step 10.1: Add `AppState::ShowingHistory` variant**

In `src/app.rs`, modify `AppState` (around line 30):

```rust
enum AppState {
    Idle,
    Showing,
    EnteringCustom { model: prompt_custom::CustomPromptModel },
    ConfirmingSize { /* unchanged */ },
    Translating { /* unchanged */ },
    /// Encrypted history viewer is open. The model holds the
    /// most-recent query results plus search/selection state.
    ShowingHistory { model: crate::ui::history::HistoryModel },
}
```

(Keep the existing variants verbatim; only add the new one.)

- [ ] **Step 10.2: Wire the new variant into `update`'s match**

In the `match std::mem::replace(...)` block (around line 721), add:

```rust
            AppState::ShowingHistory { model } => self.update_showing_history(ctx, model),
```

- [ ] **Step 10.3: Implement `summon_history` + `update_showing_history`**

Add to `impl ClipApp` (near `show_window`, around line 200):

```rust
    /// Open the history viewer. Queries the store, builds a model,
    /// resizes the viewport to 680×540, and transitions to
    /// `ShowingHistory`. If history is disabled (config or corruption),
    /// the viewer still opens but with a warning banner; this lets the
    /// user verify the toast and explore an empty (or partially
    /// readable) database.
    fn summon_history(&mut self, ctx: &egui::Context) {
        let mut model = crate::ui::history::HistoryModel::default();
        let disabled = self
            .history_disabled
            .load(std::sync::atomic::Ordering::Relaxed);
        if let Some(h) = self.history.as_ref() {
            match h.query(
                &crate::history::store::QueryFilter::default(),
                self.cfg.history.max_entries,
            ) {
                Ok(rows) => {
                    model.entries = rows;
                }
                Err(e) => {
                    tracing::warn!(error = %e, "history query failed; viewer will show empty");
                    self.history_disabled
                        .store(true, std::sync::atomic::Ordering::Relaxed);
                }
            }
        }
        // First time? Show banner. Latch via the App-level warned flag.
        if (disabled
            || self
                .history_disabled
                .load(std::sync::atomic::Ordering::Relaxed))
            && !self
                .history_warned
                .load(std::sync::atomic::Ordering::Relaxed)
        {
            model.show_corruption_banner = true;
            self.history_warned
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }

        // Resize viewport for history viewer.
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(Vec2::new(680.0, 540.0)));
        self.has_been_focused = false;
        ctx.send_viewport_cmd(ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(ViewportCommand::Focus);
        self.app_state = AppState::ShowingHistory { model };
    }

    fn update_showing_history(
        &mut self,
        ctx: &egui::Context,
        mut model: crate::ui::history::HistoryModel,
    ) {
        let click_outcome = crate::ui::history::draw(ctx, &mut model);
        // Banner is one-shot per session — clear AFTER the first draw
        // renders it, so subsequent frames don't re-render it.
        model.show_corruption_banner = false;
        let key_outcome = self.handle_keys_history(ctx, &mut model);
        let outcome = click_outcome.or(key_outcome);

        match outcome {
            Some(crate::ui::history::HistoryOutcome::Close) => {
                self.dismiss_history_to_idle(ctx);
            }
            Some(crate::ui::history::HistoryOutcome::CopyResult(id)) => {
                if let Some(entry) = model.entries.iter().find(|e| e.id == id) {
                    if let Some(result) = entry.result.as_ref() {
                        let _ = self.copy_to_clipboard(result.as_str());
                    }
                }
                self.dismiss_history_to_idle(ctx);
            }
            Some(crate::ui::history::HistoryOutcome::CopySource(id)) => {
                if let Some(entry) = model.entries.iter().find(|e| e.id == id) {
                    if let Some(source) = entry.source.as_ref() {
                        let _ = self.copy_to_clipboard(source.as_str());
                    }
                }
                // Stay open; user may want to copy more.
                self.app_state = AppState::ShowingHistory { model };
            }
            Some(crate::ui::history::HistoryOutcome::Delete(id)) => {
                if let Some(h) = self.history.as_ref() {
                    if let Err(e) = h.delete(id) {
                        tracing::warn!(error = %e, id, "history delete failed");
                    }
                }
                // Re-query so the list reflects the deletion.
                self.refresh_history_model(&mut model);
                self.app_state = AppState::ShowingHistory { model };
            }
            Some(crate::ui::history::HistoryOutcome::ClearAll) => {
                if let Some(h) = self.history.as_ref() {
                    if let Err(e) = h.clear_all() {
                        tracing::warn!(error = %e, "history clear_all failed");
                    }
                }
                self.refresh_history_model(&mut model);
                self.app_state = AppState::ShowingHistory { model };
            }
            None => {
                self.app_state = AppState::ShowingHistory { model };
            }
        }
    }

    fn handle_keys_history(
        &self,
        ctx: &egui::Context,
        model: &mut crate::ui::history::HistoryModel,
    ) -> Option<crate::ui::history::HistoryOutcome> {
        // If the modal is up, only Esc/Enter act on it.
        if model.confirm_clear {
            return ctx.input(|i| {
                if i.key_pressed(Key::Escape) {
                    model.confirm_clear = false;
                    None
                } else if i.key_pressed(Key::Enter) {
                    model.confirm_clear = false;
                    Some(crate::ui::history::HistoryOutcome::ClearAll)
                } else {
                    None
                }
            });
        }

        // Apply filter to find the focused row's id (we only act on it
        // for s/d/Enter shortcuts).
        let filtered = crate::ui::history::filter_entries(&model.entries, &model.query);
        let focused_id = filtered
            .get(model.selected)
            .map(|e| e.id);

        let len = filtered.len();
        ctx.input(|i| {
            if i.key_pressed(Key::Escape) {
                return Some(crate::ui::history::HistoryOutcome::Close);
            }
            if i.key_pressed(Key::ArrowDown) && len > 0 {
                model.selected = (model.selected + 1).min(len - 1);
            }
            if i.key_pressed(Key::ArrowUp) && len > 0 {
                model.selected = model.selected.saturating_sub(1);
            }
            if i.key_pressed(Key::Delete) && i.modifiers.shift {
                if self.cfg.history.confirm_clear {
                    model.confirm_clear = true;
                    return None;
                }
                return Some(crate::ui::history::HistoryOutcome::ClearAll);
            }
            // The single-character shortcuts (`s`, `d`) should only fire
            // when the search input is NOT focused — otherwise typing
            // 'd' into the search box would delete a row. egui doesn't
            // give us a direct "input focused" hook here without
            // threading state from `draw`; the simplest discipline is
            // to only fire these when no text was typed THIS frame.
            // Practically: rely on egui's input.events to detect
            // raw-key presses NOT consumed by a text field.
            //
            // Note: i.key_pressed returns true even when a text widget
            // consumed it. To avoid false positives, check that the
            // event hasn't been consumed by a widget by inspecting
            // i.events for a Text event that contains the same char.
            if i.key_pressed(Key::Enter) {
                if let Some(id) = focused_id {
                    return Some(crate::ui::history::HistoryOutcome::CopyResult(id));
                }
            }
            // For 's' / 'd', reject when a Text event of that letter
            // was emitted this frame — that means the search field
            // captured it.
            let typed_letters: std::collections::HashSet<char> = i
                .events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::Text(s) if s.len() == 1 => s.chars().next(),
                    _ => None,
                })
                .collect();
            if i.key_pressed(Key::S) && !typed_letters.contains(&'s') {
                if let Some(id) = focused_id {
                    return Some(crate::ui::history::HistoryOutcome::CopySource(id));
                }
            }
            if i.key_pressed(Key::D) && !typed_letters.contains(&'d') {
                if let Some(id) = focused_id {
                    return Some(crate::ui::history::HistoryOutcome::Delete(id));
                }
            }
            None
        })
    }

    fn refresh_history_model(&self, model: &mut crate::ui::history::HistoryModel) {
        if let Some(h) = self.history.as_ref() {
            match h.query(
                &crate::history::store::QueryFilter::default(),
                self.cfg.history.max_entries,
            ) {
                Ok(rows) => {
                    model.entries = rows;
                    if model.selected >= model.entries.len() {
                        model.selected = 0;
                    }
                }
                Err(e) => tracing::warn!(error = %e, "history re-query failed"),
            }
        }
    }

    fn dismiss_history_to_idle(&mut self, ctx: &egui::Context) {
        // Restore the prompt-default viewport size.
        let inner_w = if self.cfg.ui.density == "compact" {
            460.0
        } else {
            520.0
        };
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(Vec2::new(inner_w, 470.0)));
        self.app_state = AppState::Idle;
        ctx.send_viewport_cmd(ViewportCommand::Visible(false));
    }

    fn copy_to_clipboard(&self, text: &str) -> Result<(), TranslateError> {
        let mut cb = ArboardClipboard::new()?;
        cb.write_text(text)
    }
```

Add the `Vec2` import at the top of the file (it's already imported indirectly through egui in some files; verify):

```rust
use egui::{Key, Vec2, ViewportCommand};
```

- [ ] **Step 10.4: Route hotkey events by ID**

Modify `drain_channels` (around line 411):

```rust
    fn drain_channels(&mut self, ctx: &egui::Context) {
        // Hotkey events
        while let Ok(event) = self.hotkey_rx.try_recv() {
            let is_prompt = event.id == self.prompt_hotkey_id;
            let is_history = self
                .history_hotkey_id
                .map(|id| event.id == id)
                .unwrap_or(false);
            if is_prompt {
                if matches!(self.app_state, AppState::Idle) {
                    self.show_window(ctx);
                } else {
                    ctx.send_viewport_cmd(ViewportCommand::Focus);
                }
            } else if is_history {
                if matches!(self.app_state, AppState::Idle) {
                    self.summon_history(ctx);
                } else {
                    ctx.send_viewport_cmd(ViewportCommand::Focus);
                }
            } else {
                tracing::debug!(event_id = event.id, "ignoring hotkey event from unregistered ID");
            }
        }
        // Translation results
        while let Ok(outcome) = self.result_rx.try_recv() {
            self.handle_translation_done(outcome);
        }
        // Glossary reload requests (SIGHUP, tray menu in M7)
        let mut reload_requested = false;
        while self.glossary_reload_rx.try_recv().is_ok() {
            reload_requested = true;
        }
        if reload_requested {
            self.reload_glossary();
        }
    }
```

- [ ] **Step 10.5: Run tests + clippy**

```bash
cargo build 2>&1 | tail -3
cargo test --all-features 2>&1 | grep "test result:"
cargo clippy --all-features --all-targets -- -D warnings 2>&1 | tail -10
```
Expected: build clean; tests pass; clippy clean.

- [ ] **Step 10.6: Commit**

```bash
git add src/app.rs
git commit -m "feat(M5): AppState::ShowingHistory + viewport resize + hotkey ID routing"
```

---

## Task 11: Register history hotkey in main.rs + README + manual smoke + final tests

**Files:**
- Modify: `src/main.rs` (register two hotkeys; capture IDs)
- Modify: `README.md` (M5 section)

**Why:** Final wiring + user-facing documentation + the manual smoke matrix that catches what green tests miss (per handoff §10). After this task the milestone is ready for big-bang review.

- [ ] **Step 11.1: Register the history hotkey alongside the prompt hotkey**

In `src/main.rs`, replace the single-hotkey registration block (currently lines ~99-115) with two-hotkey registration:

```rust
    // Hotkey registration. Two registered: the prompt hotkey (always)
    // and the history hotkey (M5; suppressible via [hotkey.history]
    // enabled = false).
    let manager = GlobalHotKeyManager::new()?;

    // Prompt hotkey — same as M2.
    let prompt_modifier = Modifier::parse(&cfg.hotkey.modifier).ok_or_else(|| {
        anyhow::anyhow!("unknown hotkey modifier: {}", cfg.hotkey.modifier)
    })?;
    let mut prompt_mods = match prompt_modifier.resolve_native() {
        NativeModifier::Ctrl => Modifiers::CONTROL,
        NativeModifier::Alt => Modifiers::ALT,
        NativeModifier::Meta => Modifiers::META,
    };
    if cfg.hotkey.shift {
        prompt_mods |= Modifiers::SHIFT;
    }
    let prompt_key_code = letter_to_code(&cfg.hotkey.key)
        .ok_or_else(|| anyhow::anyhow!("unsupported hotkey key: {}", cfg.hotkey.key))?;
    let prompt_hotkey = HotKey::new(Some(prompt_mods), prompt_key_code);
    let prompt_hotkey_id = prompt_hotkey.id();
    if cfg.hotkey.enabled {
        manager.register(prompt_hotkey)?;
    }

    // History hotkey — M5 addition. Failure to register (e.g., already
    // claimed by another app) is non-fatal; we log warn and the user
    // can still use the tray-menu "History" item once M7 lands.
    let history_hotkey_id = if cfg.hotkey.history.enabled {
        let mod_kind = match Modifier::parse(&cfg.hotkey.history.modifier) {
            Some(m) => m,
            None => {
                tracing::warn!(
                    modifier = %cfg.hotkey.history.modifier,
                    "unknown history hotkey modifier; viewer hotkey disabled"
                );
                Modifier::Cmd
            }
        };
        let mut mods = match mod_kind.resolve_native() {
            NativeModifier::Ctrl => Modifiers::CONTROL,
            NativeModifier::Alt => Modifiers::ALT,
            NativeModifier::Meta => Modifiers::META,
        };
        if cfg.hotkey.history.shift {
            mods |= Modifiers::SHIFT;
        }
        match letter_to_code(&cfg.hotkey.history.key) {
            Some(code) => {
                let hk = HotKey::new(Some(mods), code);
                let id = hk.id();
                match manager.register(hk) {
                    Ok(()) => Some(id),
                    Err(e) => {
                        tracing::warn!(error = %e, "history hotkey registration failed; viewer hotkey unavailable");
                        None
                    }
                }
            }
            None => {
                tracing::warn!(
                    key = %cfg.hotkey.history.key,
                    "unsupported history hotkey key; viewer hotkey disabled"
                );
                None
            }
        }
    } else {
        None
    };
```

Then thread the IDs into the `ClipApp::new` call (replacing the placeholder `0` / `None` from Task 7.4):

```rust
            let app = ClipApp::new(
                cc,
                cfg,
                provider,
                templates,
                glossary,
                glossary_path,
                glossary_reload_rx,
                history,
                history_disabled_initial,
                secrets,
                state_path,
                hotkey_rx,
                prompt_hotkey_id,
                history_hotkey_id,
            );
```

- [ ] **Step 11.2: Document M5 in README**

Append to `README.md` after the M4 section (or, if M4's README content is at the top of "Features", add the M5 block immediately after the SIGHUP-reload paragraph):

```markdown
### Encrypted history (M5)

clipt9n persists every successful translation to a local SQLite
database at `<config_dir>/history.db`. Source and result text are
encrypted at the application layer with ChaCha20-Poly1305; metadata
(timestamp, action, language pair, character count) is plaintext for
search. The encryption key is derived via Argon2id from a 32-byte
secret stored at `<config_dir>/.history-key` (mode `0600` on Unix).

#### Opening the viewer

Press **Cmd+Shift+H** (macOS) / **Ctrl+Shift+H** (Linux/Windows) to
open the history viewer. The viewer is a 680px window with:

- Top search input (filter as you type — matches source, result,
  action, and language pair)
- Scrollable list of recent entries (newest first; capped at
  `[history] max_entries`, default 100)
- Detail block showing source and result side-by-side for the
  selected row
- Footer keymap:
  - `↵` (Enter) — copy the result back to clipboard, close viewer
  - `s` — copy the original source instead
  - `d` — delete the selected entry
  - `⇧+Del` — clear all entries (key file preserved)
  - `Esc` — close

#### `[history]` configuration

```toml
[history]
enabled = true            # set false to disable the database entirely
max_entries = 100         # older rows pruned at insert time
store_text = true         # set false to record metadata only (no source/result)
confirm_clear = true      # ⇧+Del prompts before wiping; set false for instant clear

[hotkey.history]
modifier = "cmd"          # "cmd" → Cmd on macOS, Ctrl on Linux/Windows
shift = true
key = "H"
enabled = true            # set false to skip registering the history hotkey
```

#### Key file caveats (M5 → M6 migration)

The current keyfile fallback (`<config_dir>/.history-key`) is the M5
implementation of spec §7's key storage. M6 adds OS-keychain support
via the `keyring` crate; on first run after upgrading, the keyfile
will be migrated into Keychain (macOS) / Credential Manager (Windows)
/ Secret Service (Linux) and the file will be left in place for safety.

The keyfile mode is less secure than the keychain mode because:
- A reader of `<config_dir>` with sufficient permissions can read it
  (mitigated by `0600` on Unix; subject to user-profile ACL on Windows).
- It doesn't benefit from the OS keychain's per-app sandboxing.

Treat M5 as appropriate for personal use; production use should wait
for M6.

#### Failure modes

| Condition | Behavior |
|---|---|
| Keyfile or DB missing | Created on first translation; no error. |
| DB corruption | Toast on viewer open: "History database unreadable. New history will not be saved." App continues to function. Delete `history.db` manually to reset. |
| Wrong key (e.g., keyfile regenerated) | Existing rows can't be decrypted; viewer shows them as skipped. New rows write fine. |
| Disk full / write failure | Logged; clipboard write succeeds; only the history record is lost. |
```

- [ ] **Step 11.3: Run the full test suite**

```bash
cargo test --all-features 2>&1 | grep "test result:" | head -10
```
Expected: ~210-215 passed; 0 failed (167 starting + ~46 new across crypto (9), store (17), ui-history (11), config (5), error (1 assertion in existing test), app (2), platform (1)).

- [ ] **Step 11.4: Cross-platform discipline check**

```bash
grep -rn '#\[cfg(target_os' src/ | grep -v '^src/platform/' | grep -v '^src/config.rs:'
grep -rn '#\[cfg(unix' src/ | grep -v '^src/platform/' | grep -v '^src/history/crypto.rs:'
```

Both should return empty output OR — for `crypto.rs` — only the `set_keyfile_permissions` `cfg(unix)` / `cfg(not(unix))` switch which delegates to `platform::set_owner_only_permissions`. That's an audited exception (the helper itself lives in `platform/unix.rs`; the `cfg` in `crypto.rs` is purely for the no-op fallback on Windows).

If the grep flags anything else, route the offending code into `src/platform/`.

- [ ] **Step 11.5: Clippy + fmt clean**

```bash
cargo clippy --all-features --all-targets -- -D warnings 2>&1 | tail -3
cargo fmt --check 2>&1 | tail -3
```
Expected: `Finished` clean / no diff.

- [ ] **Step 11.6: Verify only the 4 spec'd deps were added**

```bash
git diff main..HEAD -- Cargo.toml | grep '^+' | grep '=\s*"'
```
Expected: 4 new lines: `argon2`, `chacha20poly1305`, `rand`, `rusqlite`. Nothing else.

- [ ] **Step 11.7: Manual M5 smoke matrix**

Build the release binary:

```bash
cargo build --release 2>&1 | tail -3
```
Expected: `Finished` clean.

Run `target/release/clipt9n`. Exercise:

1. **Translate-then-restart-then-history**
   - Copy `"Guten Tag, wie geht es Ihnen heute?"`.
   - Cmd+Shift+T → press 1 (English). Translation lands in clipboard.
   - Quit the app.
   - Verify `<config_dir>/.history-key` exists with mode `0600`:
     ```bash
     ls -la "$HOME/Library/Application Support/clipboard-translator/.history-key"
     ```
     Expected: `-rw-------`.
   - Verify `<config_dir>/history.db` exists.
   - Restart the binary. Cmd+Shift+H opens the viewer; the previous
     translation is the top entry; selection on it shows source and
     result in the detail block.

2. **Search filter**
   - Type "Guten" into the search input. The list filters to matching
     entries in real time. Hit count updates.

3. **Copy back**
   - Select a row with arrow keys. Press Enter. Viewer closes; clipboard
     now holds the result text. Press Cmd+Shift+H again to verify the
     row is still there (Enter copies, doesn't delete).

4. **Copy source (`s` key)**
   - Open viewer. Select an entry. Press `s`. Clipboard now holds the
     source text. Viewer stays open.

5. **Single-row delete (`d` key)**
   - Open viewer. Select an entry. Press `d`. The row disappears
     immediately; "{N} entries · encrypted" header decrements.

6. **Clear all (`Shift+Del`)**
   - Open viewer with at least 1 entry. Press Shift+Del. Modal appears:
     "Clear all history?" with Cancel and "Clear all" buttons.
   - Press Esc → modal dismisses; rows preserved.
   - Press Shift+Del again → modal returns. Press Enter → rows wiped;
     viewer shows "0 entries · encrypted".

7. **Disabling history hotkey**
   - Edit `<config_dir>/config.toml` to set `[hotkey.history] enabled =
     false`. Restart. Cmd+Shift+H now does nothing; logs show no
     "history hotkey ID" line. Cmd+Shift+T still works.

8. **Disabling history entirely**
   - Set `[history] enabled = false`. Restart. Translate something.
     Cmd+Shift+H opens an empty viewer. `<config_dir>/history.db` is
     unchanged from the previous run (no new rows).

9. **Corruption simulation**
   - Quit the app. With history enabled in config:
     ```bash
     echo "garbage" > "$HOME/Library/Application Support/clipboard-translator/history.db"
     ```
   - Restart. Tracing logs show `history open failed; running with
     history disabled`. Translate something — succeeds. Cmd+Shift+H
     opens with the corruption banner ("History database unreadable.
     New history will not be saved.") shown once.

10. **Wrong-key simulation**
    - Quit. Move the keyfile aside:
      ```bash
      mv "$HOME/Library/Application Support/clipboard-translator/.history-key"{,.bak}
      ```
    - Restart. A new keyfile is created. Old rows can't be decrypted.
      Cmd+Shift+H shows an empty list (rows skipped silently).
    - Restore the keyfile:
      ```bash
      mv "$HOME/Library/Application Support/clipboard-translator/.history-key"{.bak,}
      ```
    - Restart. Old rows decrypt cleanly again.

11. **`store_text = false` mode**
    - Set `[history] store_text = false`. Restart. Translate. Open
      viewer — the row exists but the source/result columns show
      `(text not stored)`.

12. **UTF-8 source still works under M5 changes**
    - Restore default config. Copy `"Bir aracıyı kullanarak..."`
      (Turkish). Translate. Open viewer — entry preserves UTF-8 in
      both source and detail panes; no panic.

13. **Cross-platform discipline (one more check)**
    - Verify the grep from Step 11.4 still returns empty.

- [ ] **Step 11.8: Commit + finalize**

```bash
git add README.md
git commit -m "docs(M5): encrypted history — viewer, [history] config, key file caveats"
```

Once all M5 commits are on `m5-encrypted-history`:

```bash
git log --oneline main..m5-encrypted-history
```

Expected: ~11 commits, each starting with `feat(M5):`, `chore(M5):`, or `docs(M5):` (no merge commits inside the branch).

The branch is ready for big-bang review. Merge strategy mirrors M2/M3/M4: fast-forward to `main` once approved.

---

## Self-Review

Run this checklist after writing the plan; fix issues inline.

### 1. Spec coverage (M5 row of design doc + spec §5.5/§6/§7/§8/§9/§11)

| Spec deliverable | Plan task |
|---|---|
| `src/history/crypto.rs` — Argon2 KDF + ChaCha20-Poly1305 AEAD; per-row 12-byte nonce | Task 4 (full module). |
| Keyfile fallback at `<config_dir>/.history-key` (32 random bytes, 0600 perms; first-run created) | Task 4 (`load_or_create_keyfile`) + Task 3 (chmod helper). |
| Argon2 deterministic derivation for a given (secret, salt) | Task 4 — `argon2_derivation_is_deterministic` test. |
| `src/history/store.rs` — rusqlite (bundled). Schema per spec §7 + `idx_created_at` | Tasks 5 + 6. Schema in `migrate`; columns + index match spec verbatim. |
| `History::open(path, &key)`, `insert`, `query(filter, limit)`, `delete(id)`, `clear_all` | Tasks 5 (open/in_memory) + 6 (insert/query/delete/clear_all + insert_with_cap). |
| Insert is best-effort — failures log + toast suppressed | Task 8 — `schedule_history_insert` runs in tokio spawn + watcher; clipboard already updated when called. |
| `[history] store_text = false` writes metadata-only rows | Task 6 — `encrypt_optional` returns `(None, None)` when `store_text = None`; `query` decodes NULL columns to `None`. Tested in `insert_with_none_text_columns_writes_null_blobs`. |
| `src/ui/history.rs` per design `history-window.jsx` (680px window, search, list, detail, footer keymap) | Task 9 — `HistoryModel` + `draw` + helpers. |
| Real-time filter; arrow-key nav; Enter copy result; `s` copy source; `d` delete; Shift+Del clear-all with confirm | Task 10 — `handle_keys_history` routes all five shortcuts; `confirm_clear` modal in Task 9's `draw`. |
| Second hotkey — `Cmd+Shift+H` opens history viewer | Task 11 — `main.rs` registers second hotkey; Task 10 — `drain_channels` matches `event.id` against captured IDs. |
| New `[hotkey.history]` sub-table; nullable to disable | Task 2 — `HistoryHotkeyConfig` with `enabled = false` skip path; Task 11 wires the `enabled` check before registration. |
| Wired into `App` as a new `AppState::ShowingHistory { model }` variant | Task 10 — variant added; `update` dispatches to `update_showing_history`. |
| README documents the encryption story, the `.history-key` location, and the `[history]` config block | Task 11 — full M5 README section. |
| Spec §5.5 — Anthropic request shape (system + user, max_tokens 4096, 30s timeout) | M1-shipped; M5 doesn't change the request path. The detected-source-lang capture in Task 8 doesn't alter the LLM request shape. |
| Spec §6 — `[history]` defaults: enabled=true, max_entries=100, store_text=true, confirm_clear=true | Task 2 — `HistoryConfig::default()` matches spec verbatim. Tested in `default_history_section`. |
| Spec §7 — schema matches `entries(id, created_at, action, source_lang, target_lang, char_count, source_ciphertext, source_nonce, result_ciphertext, result_nonce)` + `idx_created_at` | Task 5 — `migrate` SQL is character-for-character aligned with spec. |
| Spec §8 — History DB corruption row | Task 7 — `main.rs::history` block catches `Err` from `History::open`; `App` carries `history_disabled` flag; Task 10's `summon_history` shows the corruption banner. |
| Spec §8 — History DB write fails mid-session row | Task 8 — insert spawn logs warn on `Err`; never sends a toast. Watcher catches panic similarly. |
| Spec §8 — History encryption key missing row | Task 7 — `crypto::load_and_derive` failure routes through the same `history_disabled` path. |
| Spec §9 — API key, decrypted history entries, clipboard text in `Zeroizing<String>` | Task 4 — `decrypt` returns `Zeroizing<Vec<u8>>`; Task 6 — `decrypt_optional` returns `Zeroizing<String>`; Task 9 — `HistoryModel` keeps `Zeroizing` wrappers. |
| Spec §11 — Search latency p95 <50 ms on 100 entries with text | Task 6's filter path is decrypt + `String::contains`; Task 11.7 manual-smoke #2 is the qualitative check. If a real benchmark catches a regression, the handoff §2 notes the fallback (move to `spawn_blocking`). |
| Spec §11 — History encryption round-trips (encrypt → store → load → decrypt) | Task 6 — `round_trip_with_100_random_strings` test. |
| Spec §11 — Argon2 key derivation determinism | Task 4 — `argon2_derivation_is_deterministic`. |

### 2. Exit criteria from the design doc, M5 row

| Exit criterion | Plan coverage |
|---|---|
| 1. Translation persists across app restarts; viewer shows it after restart | Task 11.7 manual smoke #1. |
| 2. Wrong key (simulated by deleting `history-key`) leaves rows undecryptable; viewer shows "history unreadable" toast on startup, app continues | Task 11.7 manual smoke #10 + Task 6 — `rows_undecryptable_with_wrong_key_are_silently_skipped` unit test. The "toast on startup" is implemented as the corruption banner shown on first `summon_history` after a disabled state. |
| 3. Search latency p95 <50 ms on 100 entries with text | Task 11.7 manual smoke #2 (qualitative); no automated benchmark in M5. |
| 4. Clear-all wipes rows but preserves the key | Task 6 — `clear_all_removes_every_row_but_preserves_the_db` test; Task 11.7 manual smoke #6. |
| 5. Round-trip unit test: encrypt → store → load → decrypt for 100 random strings of varying length | Task 6 — `round_trip_with_100_random_strings`. |
| 6. Argon2 derivation is deterministic for a given (secret, salt) | Task 4 — `argon2_derivation_is_deterministic`. |

### 3. Cross-cutting items inherited from prior milestones

| Item | Plan coverage |
|---|---|
| Cross-platform discipline — every `cfg(target_os)` and `cfg(unix)` in `platform/` | Task 3 + Task 11.4 grep. The single `cfg(unix)`/`cfg(not(unix))` in `crypto.rs::set_keyfile_permissions` is an audited delegate that calls `platform::set_owner_only_permissions`; documented in cross-cutting decision §16. |
| M3 worker-watcher panic-recovery pattern is preserved | Task 8 — `schedule_history_insert` mirrors the M3 watcher. The translation-worker panic path (Task 8.1's `TranslationOutcome` literal in the watcher) is updated to populate the new fields without breaking semantics. |
| M4 SIGHUP-glossary-snapshot pattern is unchanged | Task 7+8 don't touch the glossary path. The translator's read-snapshot of the glossary at dispatch time is preserved. |
| Reduced motion (M3) is cached on `App.reduced_motion` | History viewer doesn't animate per the design; no further work needed. |
| `_secrets: Box<dyn Secrets>` is still dead — M6 revives it | Task 7's `ClipApp::new` keeps the parameter unchanged. |

### 4. Placeholder scan

- No "TBD", "implement later", "etc.", "similar to Task N", or naked "add error handling" appearances.
- Every code step has the actual code; every command step has the actual command + expected output.
- The README block in Task 11.2 is the full user-facing text (not a "documents the encryption story" placeholder).

### 5. Type consistency

- `HistoryEntry { id, created_at, action, source_lang, target_lang, char_count, source: Option<Zeroizing<String>>, result: Option<Zeroizing<String>> }` — same shape across Tasks 5 (defined), 6 (consumed in `query`/`filter_matches`), 9 (consumed in viewer), 10 (consumed in App's outcome routing).
- `NewEntry { created_at, action, source_lang, target_lang, char_count, source: Option<String>, result: Option<String> }` — Task 5 declared, Task 6 consumed in `insert`, Task 8 consumed in `schedule_history_insert`.
- `HistoryModel { entries, query, selected, confirm_clear, show_corruption_banner }` — Task 9 declared; Task 10 consumed in `summon_history`/`update_showing_history`/`refresh_history_model`.
- `HistoryOutcome::{ Close, CopyResult(i64), CopySource(i64), Delete(i64), ClearAll }` — Task 9 declared; Task 10 consumed in `update_showing_history`'s match.
- `QueryFilter { query: Option<String> }` — Task 5 declared, Task 6 consumed, Task 10 consumed.
- `ClipApp` field set: `history: Option<Arc<History>>`, `history_disabled: Arc<AtomicBool>`, `history_warned: AtomicBool`, `prompt_hotkey_id: u32`, `history_hotkey_id: Option<u32>` — Task 7 declared; Tasks 8, 10, 11 consumed. Cloning `self.history.clone()` for spawn-into-worker bumps the inner `Arc` refcount when present and is `None.clone() == None` otherwise.
- `derive_key`, `encrypt`, `decrypt`, `load_or_create_keyfile`, `load_and_derive`, `default_keyfile_path` — Task 4 declared; Tasks 5/6 (encrypt_optional/decrypt_optional internal helpers), 7 (main.rs/lib.rs callers).
- `History::open`, `History::in_memory`, `History::insert`, `History::insert_with_cap`, `History::query`, `History::delete`, `History::clear_all`, `History::count` — Task 5 + 6 declared; Task 7 (open in main/lib), Task 8 (insert_with_cap), Task 10 (query/delete/clear_all in App), and Task 6 tests for everything.
- `set_owner_only_permissions(path: &Path) -> std::io::Result<()>` — Task 3 declared in `platform/unix.rs`; Task 4 (`crypto.rs::set_keyfile_permissions`) consumed via the `platform/mod.rs` re-export.
- `action_kind_str(&Action)`, `target_lang_for(&Action)` — Task 8 declared and tested.
- `ago_label(i64, Option<i64>)`, `pair_label(&HistoryEntry)`, `pair_color(&HistoryEntry)`, `filter_entries`, `truncate_for_row` — Task 9 declared and tested.
- `TranslationOutcome` adds `detected_source_lang`, `source_text`, `action` fields — Task 8 declared; consumed in `handle_translation_done` (M3-shipped) + `schedule_history_insert` (Task 8). All call sites updated in Task 8.1.

No drift. Plan is consistent end-to-end.

---

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-04-29-clipt9n-m5-encrypted-history.md`. The user has pre-confirmed subagent-driven execution; on completion of this plan, the orchestrator invokes **superpowers:subagent-driven-development** with this plan as input. Mirrors M1/M2/M3/M4 execution flow.
