//! The ReleaseLog predeploy (contracts/src/ReleaseLog.sol): the append-only
//! public record Mac app releases are approved against
//! (docs/design/19-release-approval.md).
//!
//! A new genesis (node rewards + history v2) installs the runtime code at a
//! fixed address, so the address and the runtime code hash are known before
//! the genesis ceremony and the shipped network.json can pin them (checklist
//! B6) — the ceremony record pins that file's exact bytes, so nothing can be
//! added to it after a post-launch deployment. The contract has no
//! constructor state, no owner and no upgrade path: installing the runtime
//! code is the whole deployment. 7780 never gets it.

use alloy_primitives::{address, keccak256, Address, Bytes, B256};
use std::sync::OnceLock;

/// Where the ReleaseLog lives on a new genesis.
pub const ADDRESS: Address = address!("0000000000000000000000000000000000007705");

/// Builder signatures a normal release needs (of the three pinned keys).
pub const THRESHOLD: u8 = 2;
/// Builder signatures an emergency release needs: all three. B4 changed only
/// the protocol-upgrade emergency quorum, never this app-release rule.
pub const EMERGENCY_THRESHOLD: u8 = 3;
/// Pinned builder keys.
pub const BUILDERS: usize = 3;

/// The runtime code (`forge inspect ReleaseLog deployedBytecode`, solc 0.8.19,
/// optimizer 200 runs; `scripts/release-approve.py contract-hash` recompiles
/// and compares).
pub fn code() -> Bytes {
    static CODE: OnceLock<Bytes> = OnceLock::new();
    CODE.get_or_init(|| {
        Bytes::from(alloy_primitives::hex::decode(include_str!("release_log.bin.hex").trim()).expect("valid ReleaseLog hex"))
    })
    .clone()
}

/// keccak256 of the runtime code: the value the wallet checks with an
/// EIP-7864 code-hash proof before it reads any entry.
pub fn code_hash() -> B256 {
    keccak256(code())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_code_hash_is_the_published_pin() {
        assert!(!code().is_empty());
        assert_eq!(
            format!("{:#x}", code_hash()),
            "0x4417ad7040420fe3547cdc3fdcd0fa0a690ba2f65e98af851a5e9fa5589db1ec",
            "a ReleaseLog code change moves every new-genesis release pin: re-run contract-hash and update docs/design/19"
        );
    }
}
