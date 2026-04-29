//! Encrypted history persistence. `crypto.rs` owns the keyfile and the
//! AEAD layer; `store.rs` owns the SQLite schema and CRUD. The viewer
//! UI lives in `src/ui/history.rs` (egui paint + keyboard handling),
//! added in Task 9.
//!
//! Cross-cutting policy (spec §8 + §9):
//! - Failures are best-effort. A corrupt DB or missing key surfaces a
//!   one-shot toast and disables history for the session; clipboard
//!   writes are NEVER blocked by history-side errors.
//! - Decrypted source/result text is wrapped in `Zeroizing<String>` at
//!   every public boundary so it's wiped from memory when dropped.

pub mod crypto;
pub mod store;
