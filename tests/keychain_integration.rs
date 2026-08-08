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

#[test]
#[ignore = "requires an interactive OS keychain available to the test process"]
fn keychain_round_trip() {
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

    let key = secrets
        .get_api_key()
        .expect("keychain readback must succeed after a successful write");
    assert_eq!(&*key, "sk-test-keychain-roundtrip");
}
