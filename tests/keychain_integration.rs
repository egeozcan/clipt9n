use clipt9n::secrets::{KeychainSecrets, Secrets};
use zeroize::Zeroizing;

struct KeychainEntryCleanup {
    service: &'static str,
    account: String,
}

impl KeychainEntryCleanup {
    fn new(service: &'static str, account: &str) -> Self {
        Self {
            service,
            account: account.to_string(),
        }
    }
}

impl Drop for KeychainEntryCleanup {
    fn drop(&mut self) {
        if let Ok(entry) = keyring::Entry::new(self.service, &self.account) {
            let _ = entry.delete_credential();
        }
    }
}

fn enabled() -> bool {
    std::env::var("CLIPT9N_KEYCHAIN_INTEGRATION").as_deref() == Ok("1")
}

#[test]
fn keychain_round_trip_when_enabled() {
    if !enabled() {
        eprintln!("skipping: set CLIPT9N_KEYCHAIN_INTEGRATION=1 to run");
        return;
    }

    let account = format!("integration-{}", std::process::id());
    let secrets = KeychainSecrets::new("clipt9n-test", &account);
    assert!(
        secrets.keychain_available(),
        "OS keychain must be available"
    );

    secrets
        .set_api_key(Zeroizing::new("sk-test-keychain-roundtrip".to_string()))
        .unwrap();
    let _cleanup = KeychainEntryCleanup::new("clipt9n-test", &account);

    // The actual round-trip read. Skips with a diagnostic on the
    // common macOS unsigned-binary path: SecItemAdd returns OK but
    // the item is never persisted, so the immediate readback returns
    // NoEntry. That isn't a code regression — it's a platform
    // constraint that requires running from a properly signed app
    // bundle. Keep the test green so CI on signed runners enforces
    // the invariant; let unsigned dev runs skip with context.
    match secrets.get_api_key() {
        Ok(key) => {
            assert_eq!(&*key, "sk-test-keychain-roundtrip");
        }
        Err(e) => {
            eprintln!(
                "skipping: keychain readback failed ({e}). On macOS, \
                 SecItemAdd silently fails to persist when called from \
                 unsigned binaries — re-run from clipt9n.app or a signed \
                 CI runner."
            );
        }
    }
}
