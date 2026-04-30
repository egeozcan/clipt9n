#![no_main]

use clipt9n::history::crypto::{decrypt, derive_key};
use libfuzzer_sys::fuzz_target;
use zeroize::Zeroizing;

fuzz_target!(|data: &[u8]| {
    if data.len() < 12 {
        return;
    }
    let key = derive_key(&Zeroizing::new([7u8; 32])).unwrap();
    let mut nonce = [0u8; 12];
    nonce.copy_from_slice(&data[..12]);
    let ciphertext = &data[12..];
    let _ = decrypt(&key, ciphertext, &nonce).err();
});
