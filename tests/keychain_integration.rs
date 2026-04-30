use clipt9n::secrets::{KeychainSecrets, Secrets};
use zeroize::Zeroizing;

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
    let key = secrets.get_api_key().unwrap();
    assert_eq!(&*key, "sk-test-keychain-roundtrip");
}
