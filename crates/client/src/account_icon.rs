//! Archipelago v2, specified in `docs/design/46-account-icon.md`.
//! Identity depends on all 20 address bytes, never on names or chain metadata.

use sha2::{Digest, Sha256};

pub const ACCOUNT_ICON_VERSION: u8 = 2;
const DOMAIN: &[u8] = b"eastsea-account-icon-v2";

/// A versioned feature tuple; palette and geometry tables belong to the renderer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AccountIconSpec {
    pub version: u8,
    pub palette: u8,
    pub layout: u16,
    pub shape: u8,
    pub rotation: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccountIconError {
    InvalidAddress,
    UnsupportedVersion,
}

impl AccountIconSpec {
    /// Derive v2 from exactly 20 bytes, without text normalization or allocation.
    pub fn from_bytes(address: &[u8; 20]) -> Self {
        let mut hash = Sha256::new();
        hash.update(DOMAIN);
        hash.update(address);
        let seed = hash.finalize();
        Self {
            version: ACCOUNT_ICON_VERSION,
            palette: seed[0] & 15,
            layout: u16::from_be_bytes([seed[1], seed[2]]) & 0x3fff,
            shape: (seed[0] >> 4) & 3,
            rotation: (seed[0] >> 6) & 3,
        }
    }

    /// Forty ASCII hex digits, with optional `0x`/`0X`; reject every other input.
    pub fn from_address(address: &str) -> Result<Self, AccountIconError> {
        Self::from_address_version(address, ACCOUNT_ICON_VERSION)
    }

    /// An explicit version prevents old icons from being silently reinterpreted.
    pub fn from_address_version(address: &str, version: u8) -> Result<Self, AccountIconError> {
        if version != ACCOUNT_ICON_VERSION {
            return Err(AccountIconError::UnsupportedVersion);
        }
        let hex = address
            .strip_prefix("0x")
            .or_else(|| address.strip_prefix("0X"))
            .unwrap_or(address)
            .as_bytes();
        if hex.len() != 40 {
            return Err(AccountIconError::InvalidAddress);
        }
        let mut bytes = [0u8; 20];
        for (byte, pair) in bytes.iter_mut().zip(hex.as_chunks::<2>().0) {
            *byte = (nibble(pair[0])? << 4) | nibble(pair[1])?;
        }
        Ok(Self::from_bytes(&bytes))
    }

    /// One of 16 broad coastline classes; larger icons add two satellite islands.
    pub fn silhouette_class(&self) -> u8 {
        self.shape * 4 + (self.layout & 3) as u8
    }
}

fn nibble(byte: u8) -> Result<u8, AccountIconError> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(AccountIconError::InvalidAddress),
    }
}
