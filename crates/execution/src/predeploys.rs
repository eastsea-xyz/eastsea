//! Standard Ethereum predeploys a new genesis installs (clone catalog §0 B3):
//! the Arachnid deterministic CREATE2 deployer, Multicall3 and Permit2, at their
//! Ethereum addresses with their exact mainnet runtime code, so tooling that
//! assumes them (forge scripts' CREATE2 factory, viem/ethers multicall, Safe
//! deterministic deployments and Permit2 signatures) works on EastSea unchanged.
//!
//! Genesis writes the runtime code directly instead of replaying Ethereum's
//! secp256k1 deployment transactions. All three start with empty storage.
//! Permit2's runtime contains its mainnet EIP-712 domain cache and recomputes
//! the separator when the chain id differs from Ethereum's. Only a new genesis
//! with node rewards AND history v2 installs these contracts; 7780 gets none.
//!
//! The CREATE2 and Multicall3 code was read with `eth_getCode` from Ethereum
//! mainnet, Base and Arbitrum One (all identical). Multicall3 and Permit2 are
//! byte-identical to the toolbox's Ethereum mainnet cache fetched 2026-10-06.
//! The independently pinned hashes are asserted here and by the genesis tests.

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

/// Uniswap Permit2 (github.com/Uniswap/permit2).
pub const PERMIT2: Address = address!("000000000022D473030F116dDEE9F6B43aC78BA3");
/// keccak256 of its exact mainnet runtime code, including immutables (9152 bytes).
pub const PERMIT2_CODE_HASH: B256 = b256!("c67d1657868aa5146eaf24fb879fb1fdec3d2d493b3683a61c9c2f4fb2851131");

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

/// Permit2's runtime code, including its canonical mainnet domain cache.
pub fn permit2_code() -> Bytes {
    static CODE: OnceLock<Bytes> = OnceLock::new();
    CODE.get_or_init(|| decode(include_str!("permit2.bin.hex"))).clone()
}

/// Every standard predeploy: (address, runtime code, pinned code hash).
pub fn all() -> [(Address, Bytes, B256); 3] {
    [
        (CREATE2_DEPLOYER, create2_deployer_code(), CREATE2_DEPLOYER_CODE_HASH),
        (MULTICALL3, multicall3_code(), MULTICALL3_CODE_HASH),
        (PERMIT2, permit2_code(), PERMIT2_CODE_HASH),
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
        assert_eq!(permit2_code().len(), 9152);
        assert_eq!(keccak256(permit2_code()), PERMIT2_CODE_HASH);
    }

    #[test]
    fn installed_requires_permit2_and_its_exact_code() {
        let runtimes = all();
        let code_at = |address: &Address| {
            runtimes.iter().find(|(a, _, _)| a == address).map(|(_, code, _)| code.clone()).unwrap_or_default()
        };
        assert!(installed(code_at));
        // CREATE2 and Multicall3 alone used to satisfy the mainnet checklist.
        assert!(!installed(|address| if *address == PERMIT2 { Bytes::new() } else { code_at(address) }));
        let mut altered = permit2_code().to_vec();
        altered[0] ^= 1;
        assert!(!installed(|address| if *address == PERMIT2 { altered.clone().into() } else { code_at(address) }));
    }
}
