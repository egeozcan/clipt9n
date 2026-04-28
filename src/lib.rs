//! Public library surface for integration tests in `tests/`.
//!
//! The actual entry point is `src/main.rs`; this lib re-exports modules so
//! `tests/*.rs` can import them as `clipt9n::module`.

pub mod clipboard;
pub mod config;
pub mod error;
pub mod llm;
pub mod secrets;
pub mod translator;
