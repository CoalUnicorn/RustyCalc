use sha2::{Digest, Sha256};

use crate::storage::MAX_SHARE_BYTES;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifyError {
    TooShort,
    NonAlphabetic,
    HashMismatch,
}

impl std::fmt::Display for VerifyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooShort => f.write_str("Word must be at least 3 letters."),
            Self::NonAlphabetic => f.write_str("Word must contain only letters."),
            Self::HashMismatch => f.write_str("That word doesn't match. Try again."),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ShareVersion {
    V0 = 0x00,
    V1 = 0x01,
}

impl TryFrom<u8> for ShareVersion {
    type Error = ();
    fn try_from(byte: u8) -> Result<Self, Self::Error> {
        match byte {
            0x00 => Ok(Self::V0),
            0x01 => Ok(Self::V1),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone)]
pub enum SharePayload {
    V0(Vec<u8>),
    V1 { hash: [u8; 32], bytes: Vec<u8> },
}

pub fn validate_word(word: &str) -> Result<&str, VerifyError> {
    let trimmed = word.trim();
    if trimmed.chars().count() < 3 {
        return Err(VerifyError::TooShort);
    }
    if !trimmed.chars().all(char::is_alphabetic) {
        return Err(VerifyError::NonAlphabetic);
    }
    Ok(trimmed)
}

pub fn hash_word(word: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(word.trim().as_bytes());
    hasher.finalize().into()
}

/// Frame `bitcode_bytes` with a share-version header.
///
/// `None` -> a V0 (no-verification) payload. `Some(word)` -> a V1 payload carrying
/// a SHA-256 of the word, which the recipient must retype. The word is held to
/// the *same* grammar `decode_with_consent` enforces on the receiver, so a
/// sender can never mint a hash that no validate-passing word could match — i.e.
/// a permanently un-openable link. That is why this returns a `Result`: an
/// invalid word is refused here, at share-creation time.
pub fn encode_with_version(
    word: Option<&str>,
    bitcode_bytes: &[u8],
) -> Result<Vec<u8>, VerifyError> {
    match word {
        None => {
            let mut out = Vec::with_capacity(1 + bitcode_bytes.len());
            out.push(ShareVersion::V0 as u8);
            out.extend_from_slice(bitcode_bytes);
            Ok(out)
        }
        Some(w) => {
            // Mirror decode_with_consent: hold the word to the receiver's grammar
            // before hashing, so a word nobody could retype (an un-openable link)
            // is refused here at creation. validate_word trims, matching the hash.
            let word = validate_word(w)?;
            let hash = hash_word(word);
            let mut out = Vec::with_capacity(1 + 32 + bitcode_bytes.len());
            out.push(ShareVersion::V1 as u8);
            out.extend_from_slice(&hash);
            out.extend_from_slice(bitcode_bytes);
            Ok(out)
        }
    }
}

pub fn decode_payload(raw: &[u8]) -> Option<SharePayload> {
    let (version_byte, rest) = raw.split_first()?;
    let version = ShareVersion::try_from(*version_byte).ok()?;
    match version {
        ShareVersion::V0 => {
            if rest.len() > MAX_SHARE_BYTES {
                return None;
            }
            Some(SharePayload::V0(rest.to_vec()))
        }
        ShareVersion::V1 => {
            let (hash_slice, bytes) = rest.split_first_chunk::<32>()?;
            if bytes.len() > MAX_SHARE_BYTES {
                return None;
            }
            Some(SharePayload::V1 {
                hash: *hash_slice,
                bytes: bytes.to_vec(),
            })
        }
    }
}

pub fn decode_with_consent(hash: &[u8; 32], word: &str) -> Result<(), VerifyError> {
    let trimmed = validate_word(word)?;
    if &hash_word(trimmed) != hash {
        return Err(VerifyError::HashMismatch);
    }
    Ok(())
}
