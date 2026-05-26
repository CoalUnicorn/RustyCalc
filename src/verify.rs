use sha2::{Digest, Sha256};

use crate::storage::MAX_SHARE_BYTES;

#[derive(Debug, Clone, PartialEq, Eq)]
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

pub fn encode_with_version(word: Option<&str>, bitcode_bytes: &[u8]) -> Vec<u8> {
    match word {
        None => {
            let mut out = Vec::with_capacity(1 + bitcode_bytes.len());
            out.push(ShareVersion::V0 as u8);
            out.extend_from_slice(bitcode_bytes);
            out
        }
        Some(w) => {
            let hash = hash_word(w);
            let mut out = Vec::with_capacity(1 + 32 + bitcode_bytes.len());
            out.push(ShareVersion::V1 as u8);
            out.extend_from_slice(&hash);
            out.extend_from_slice(bitcode_bytes);
            out
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

pub fn verify_and_extract(hash: &[u8; 32], word: &str) -> Result<Vec<u8>, VerifyError> {
    let trimmed = validate_word(word)?;
    if &hash_word(trimmed) != hash {
        return Err(VerifyError::HashMismatch);
    }
    Ok(Vec::new())
}
