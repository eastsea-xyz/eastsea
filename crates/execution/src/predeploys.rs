//! Standard Ethereum predeploys a new genesis installs (clone catalog §0 B3):
//! the Arachnid deterministic CREATE2 deployer and Multicall3, at their
//! Ethereum addresses with their exact mainnet runtime code, so tooling that
//! assumes them (forge scripts' CREATE2 factory, viem/ethers multicall, Safe
//! and Permit2 deterministic deployments) works on EastSea unchanged.
//!
//! Both are ownerless, stateless at deployment, and need no constructor: the
//! runtime code is the whole deployment. Ethereum got them by a keyless
//! presigned transaction; EastSea cannot replay that (P-256 envelopes, other
//! chain id), so the genesis writes the code directly. 7780 never gets them.
//!
//! The code was read with `eth_getCode` from Ethereum mainnet, Base and
//! Arbitrum One (all identical); the hashes below are the published mainnet
//! code hashes, asserted by the tests here and by the genesis tests.

use alloy_primitives::{address, b256, keccak256, Address, Bytes, B256};
use std::sync::OnceLock;

/// The Arachnid deterministic deployment proxy
/// (github.com/Arachnid/deterministic-deployment-proxy): calldata is
/// `salt ‖ initcode`, it CREATE2s and returns the new address.
pub const CREATE2_DEPLOYER: Address = address!("4e59b44847b379578588920cA78FbF26c0B4956C");
/// keccak256 of its mainnet runtime code (69 bytes).
pub const CREATE2_DEPLOYER_CODE_HASH: B256 = b256!("2fa86add0aed31f33a762c9d88e807c475bd51d0f52bd0955754b2608f7e4989");

/// Multicall3 (github.com/mds1/multicall3).
pub const MULTICALL3: Address = address!("cA11bde05977b3631167028862bE2a173976CA11");
/// keccak256 of its mainnet runtime code (3808 bytes).
pub const MULTICALL3_CODE_HASH: B256 = b256!("d5c15df687b16f2ff992fc8d767b4216323184a2bbc6ee2f9c398c318e770891");

fn decode(hex: &str) -> Bytes {
    Bytes::from(alloy_primitives::hex::decode(hex.trim()).expect("valid predeploy hex"))
}

/// The CREATE2 deployer's runtime code.
pub fn create2_deployer_code() -> Bytes {
    static CODE: OnceLock<Bytes> = OnceLock::new();
    CODE.get_or_init(|| decode(include_str!("create2_deployer.bin.hex"))).clone()
}

/// Multicall3's runtime code.
pub fn multicall3_code() -> Bytes {
    static CODE: OnceLock<Bytes> = OnceLock::new();
    CODE.get_or_init(|| decode(include_str!("multicall3.bin.hex"))).clone()
}

/// Every standard predeploy: (address, runtime code, pinned code hash).
pub fn all() -> [(Address, Bytes, B256); 2] {
    [
        (CREATE2_DEPLOYER, create2_deployer_code(), CREATE2_DEPLOYER_CODE_HASH),
        (MULTICALL3, multicall3_code(), MULTICALL3_CODE_HASH),
    ]
}

/// Whether `code_at` answers every standard predeploy with its pinned code.
pub fn installed(code_at: impl Fn(&Address) -> Bytes) -> bool {
    all().iter().all(|(address, _, hash)| keccak256(code_at(address)) == *hash)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_code_is_the_mainnet_code() {
        assert_eq!(create2_deployer_code().len(), 69);
        assert_eq!(keccak256(create2_deployer_code()), CREATE2_DEPLOYER_CODE_HASH);
        assert_eq!(multicall3_code().len(), 3808);
        assert_eq!(keccak256(multicall3_code()), MULTICALL3_CODE_HASH);
    }
}
