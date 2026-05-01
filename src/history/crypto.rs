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
            TranslateError::History(format!("creating parent dir {}: {e}", parent.display()))
        })?;
    }
    let mut secret = Zeroizing::new([0u8; 32]);
    OsRng.fill_bytes(secret.as_mut());
    std::fs::write(path, secret.as_slice())
        .map_err(|e| TranslateError::History(format!("writing keyfile {}: {e}", path.display())))?;
    set_keyfile_permissions(path)?;
    tracing::warn!(
        path = %path.display(),
        "history-key created at {}; M6 will migrate this to OS keychain",
        path.display()
    );
    Ok(secret)
}

fn set_keyfile_permissions(path: &Path) -> Result<(), TranslateError> {
    crate::platform::set_owner_only_permissions(path)
        .map_err(|e| TranslateError::History(format!("chmod 0o600 on {}: {e}", path.display())))
}

/// Convenience: load (or create) the keyfile and immediately derive
/// the AEAD key. Used by `History::open` so callers don't have to
/// chain the two calls.
pub fn load_and_derive(keyfile_path: &Path) -> Result<Zeroizing<[u8; 32]>, TranslateError> {
    let secret = load_or_create_keyfile(keyfile_path)?;
    derive_key(&secret)
}

/// Load the migrated history secret from the OS keychain when present,
/// falling back to the legacy keyfile path used by M5.
pub fn load_and_derive_with_keychain_fallback(
    keyfile_path: &Path,
    service: &str,
    account: &str,
) -> Result<Zeroizing<[u8; 32]>, TranslateError> {
    match crate::secrets::history_secret_from_keychain(service, account) {
        Ok(Some(secret)) => derive_key(&secret),
        Ok(None) => load_and_derive(keyfile_path),
        Err(e @ TranslateError::History(_)) => Err(e),
        Err(e) => {
            tracing::warn!(
                error = %e,
                "history keychain read failed; falling back to legacy keyfile"
            );
            load_and_derive(keyfile_path)
        }
    }
}

/// Compute a default keyfile path: `<config_dir>/.history-key`.
/// Exposed because `lib.rs::run` and `main.rs` both compute it; keeping
/// the construction in one place avoids drift.
pub fn default_keyfile_path(config_dir: &Path) -> PathBuf {
    config_dir.join(".history-key")
}

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
    fn decrypt_rejects_tampered_nonce() {
        let secret = Zeroizing::new([9u8; 32]);
        let key = derive_key(&secret).unwrap();
        let (ct, mut nonce) = encrypt(&key, b"nonce protected").unwrap();
        nonce[0] ^= 0x01;
        let err = decrypt(&key, &ct, &nonce).unwrap_err();
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
        assert!(
            crate::platform::owner_only_permissions_are_enforced_for_test(&kf).unwrap(),
            "keyfile permissions must be owner-only on platforms that support POSIX modes"
        );
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
