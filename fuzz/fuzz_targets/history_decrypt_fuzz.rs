#![no_main]

use clipt9n::history::crypto::{decrypt, derive_key, encrypt};
use libfuzzer_sys::fuzz_target;
use zeroize::Zeroizing;

fuzz_target!(|data: &[u8]| {
    let key = derive_key(&Zeroizing::new([7u8; 32])).unwrap();
    let (mut ciphertext, mut nonce) = encrypt(&key, b"authenticated history entry").unwrap();

    let selector = data.first().copied().unwrap_or(0) as usize;
    let bit = 1_u8 << (data.get(1).copied().unwrap_or(0) % 8);
    if selector % 2 == 0 {
        nonce[(selector / 2) % nonce.len()] ^= bit;
    } else {
        let index = data.get(2).copied().unwrap_or(0) as usize % ciphertext.len();
        ciphertext[index] ^= bit;
    }

    assert!(decrypt(&key, &ciphertext, &nonce).is_err());
});
