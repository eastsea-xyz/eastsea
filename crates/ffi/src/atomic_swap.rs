//! ABI calldata for the chain-agnostic AtomicSwap contract.

use crate::{WalletError, R};
use aether_types::{Address, U256};
use alloy_primitives::{hex, keccak256};

fn selector(signature: &str) -> [u8; 4] {
    keccak256(signature.as_bytes()).0[..4]
        .try_into()
        .expect("selector")
}

fn word(value: U256) -> [u8; 32] {
    value.to_be_bytes::<32>()
}

fn decode_hex(value: &str, field: &str) -> R<Vec<u8>> {
    hex::decode(value.strip_prefix("0x").unwrap_or(value))
        .map_err(|_| WalletError::Invalid(format!("{field}: hex")))
}

/// Encode `lock(address,bytes32,uint64,address,uint256)`. Native coin uses the
/// zero token address and must be sent with `value_wei == amount_wei`.
#[uniffi::export]
pub fn atomic_swap_lock_calldata(
    recipient: String,
    hashlock_hex: String,
    timelock: u64,
    token: String,
    amount_wei: String,
) -> R<String> {
    let recipient: Address = recipient
        .parse()
        .map_err(|_| WalletError::Invalid("recipient address".into()))?;
    let token: Address = token
        .parse()
        .map_err(|_| WalletError::Invalid("token address".into()))?;
    let hashlock: [u8; 32] = decode_hex(&hashlock_hex, "hashlock")?
        .try_into()
        .map_err(|_| WalletError::Invalid("hashlock: 32 bytes".into()))?;
    let amount: U256 = amount_wei
        .parse()
        .map_err(|_| WalletError::Invalid("amount".into()))?;
    if recipient == Address::ZERO || hashlock == [0; 32] || timelock == 0 || amount == U256::ZERO {
        return Err(WalletError::Invalid("empty swap field".into()));
    }
    let mut data = Vec::with_capacity(4 + 5 * 32);
    data.extend_from_slice(&selector("lock(address,bytes32,uint64,address,uint256)"));
    data.extend_from_slice(&word(U256::from_be_slice(recipient.as_slice())));
    data.extend_from_slice(&hashlock);
    data.extend_from_slice(&word(U256::from(timelock)));
    data.extend_from_slice(&word(U256::from_be_slice(token.as_slice())));
    data.extend_from_slice(&word(amount));
    Ok(hex::encode_prefixed(data))
}

/// Encode `claim(uint256,bytes)` with the original Bitcoin-compatible bytes.
#[uniffi::export]
pub fn atomic_swap_claim_calldata(id: u64, preimage_hex: String) -> R<String> {
    let preimage = decode_hex(&preimage_hex, "preimage")?;
    let padded_len = preimage
        .len()
        .checked_add(31)
        .ok_or_else(|| WalletError::Invalid("preimage too large".into()))?
        / 32
        * 32;
    let mut data = Vec::with_capacity(4 + 3 * 32 + padded_len);
    data.extend_from_slice(&selector("claim(uint256,bytes)"));
    data.extend_from_slice(&word(U256::from(id)));
    data.extend_from_slice(&word(U256::from(64)));
    data.extend_from_slice(&word(U256::from(preimage.len())));
    data.extend_from_slice(&preimage);
    data.resize(4 + 3 * 32 + padded_len, 0);
    Ok(hex::encode_prefixed(data))
}

/// Encode `refund(uint256)`; the contract pays its stored sender.
#[uniffi::export]
pub fn atomic_swap_refund_calldata(id: u64) -> String {
    let mut data = Vec::with_capacity(4 + 32);
    data.extend_from_slice(&selector("refund(uint256)"));
    data.extend_from_slice(&word(U256::from(id)));
    hex::encode_prefixed(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_matches_solidity_abi() {
        let data = atomic_swap_lock_calldata(
            "0x0000000000000000000000000000000000001234".into(),
            format!("0x{}", "ab".repeat(32)),
            1_700_000_000,
            "0x0000000000000000000000000000000000000000".into(),
            "1000000000000000000".into(),
        )
        .unwrap();
        assert_eq!(
            data,
            format!(
                "0x0dac9166{}{}{}{}{}",
                format!("{:064x}", 0x1234u64),
                "ab".repeat(32),
                format!("{:064x}", 1_700_000_000u64),
                "0".repeat(64),
                format!("{:064x}", 1_000_000_000_000_000_000u64)
            )
        );
        assert!(atomic_swap_lock_calldata(
            "bad".into(),
            "ab".repeat(32),
            1,
            Address::ZERO.to_string(),
            "1".into()
        )
        .is_err());
        assert!(atomic_swap_lock_calldata(
            "0x0000000000000000000000000000000000001234".into(),
            "ab".into(),
            1,
            Address::ZERO.to_string(),
            "1".into()
        )
        .is_err());
    }

    #[test]
    fn claim_and_refund_match_solidity_abi() {
        let claim = atomic_swap_claim_calldata(7, "0x010203".into()).unwrap();
        assert_eq!(
            claim,
            format!(
                "0x38926b6d{}{}{}01{}",
                format!("{:064x}", 7),
                format!("{:064x}", 64),
                format!("{:064x}", 3),
                "0203".to_owned() + &"0".repeat(58)
            )
        );
        assert_eq!(
            atomic_swap_refund_calldata(7),
            format!("0x278ecde1{:064x}", 7)
        );
        assert!(atomic_swap_claim_calldata(0, "xyz".into()).is_err());
    }
}
