//! `encode_with_version` and `decode_with_consent` must agree on the word
//! grammar. A sender word that `validate_word` rejects (too short / non-alpha)
//! would hash to a value no validate-passing word could ever match — a
//! permanently un-openable link, reported to the recipient as a misleading
//! "too short" they cannot act on. Encode must refuse exactly what decode does.

use crate::verify::{self, SharePayload, ShareVersion, VerifyError};
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

// The regression: words the receiver would reject must be refused at encode
// time, surfaced as the *same* `VerifyError` the receiver reports.
#[wasm_bindgen_test]
fn encode_rejects_words_the_receiver_would_reject() {
    assert_eq!(
        verify::encode_with_version(Some("ab"), b"payload"),
        Err(VerifyError::TooShort),
    );
    assert_eq!(
        verify::encode_with_version(Some("pa55"), b"payload"),
        Err(VerifyError::NonAlphabetic),
    );
}

// A validate-passing word still round-trips: encode -> decode_payload -> the
// recipient's consent check accepts the same word.
#[wasm_bindgen_test]
fn valid_word_round_trips() {
    let Ok(wrapped) = verify::encode_with_version(Some("secret"), b"payload") else {
        panic!("a validate-passing word must encode");
    };
    let Some(SharePayload::V1 { hash, bytes }) = verify::decode_payload(&wrapped) else {
        panic!("a word share must decode as V1");
    };
    assert_eq!(bytes, b"payload".to_vec());
    assert!(verify::decode_with_consent(&hash, "secret").is_ok());
}

// No word -> V0, unchanged and infallible.
#[wasm_bindgen_test]
fn no_word_encodes_v0() {
    let Ok(wrapped) = verify::encode_with_version(None, b"payload") else {
        panic!("the no-word path never fails");
    };
    assert_eq!(wrapped.first().copied(), Some(ShareVersion::V0 as u8));
}
