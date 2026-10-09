//! Standard contracts installed by host-side new-genesis configuration.
//!
//! The released executor supplies CREATE2 and Multicall3. Permit2 belongs here
//! so adding a genesis contract does not change the proving guest's source.
//! Installation requires node rewards and history v2; the shipped 7780 genesis
//! receives none of these contracts.

use alloy_primitives::{address, b256, keccak256, Address, Bytes, B256};
use std::sync::OnceLock;

pub use aether_execution::predeploys::{
    create2_deployer_code, multicall3_code, CREATE2_DEPLOYER, CREATE2_DEPLOYER_CODE_HASH,
    MULTICALL3, MULTICALL3_CODE_HASH,
};

/// Uniswap Permit2 at its canonical Ethereum address.
pub const PERMIT2: Address = address!("000000000022D473030F116dDEE9F6B43aC78BA3");
/// Exact mainnet runtime, including its immutable domain cache (9152 bytes).
pub const PERMIT2_CODE_HASH: B256 = b256!("c67d1657868aa5146eaf24fb879fb1fdec3d2d493b3683a61c9c2f4fb2851131");

/// The cached EIP-712 separator is recomputed when the chain id differs.
pub fn permit2_code() -> Bytes {
    static CODE: OnceLock<Bytes> = OnceLock::new();
    CODE.get_or_init(|| {
        Bytes::from(alloy_primitives::hex::decode(include_str!("permit2.bin.hex").trim()).expect("valid Permit2 hex"))
    }).clone()
}

/// Every new-genesis predeploy: (address, runtime code, pinned code hash).
pub fn all() -> [(Address, Bytes, B256); 3] {
    let [deployer, multicall] = aether_execution::predeploys::all();
    [deployer, multicall, (PERMIT2, permit2_code(), PERMIT2_CODE_HASH)]
}

/// Require the released standard runtimes and the exact Permit2 runtime.
pub fn installed(code_at: impl Fn(&Address) -> Bytes) -> bool {
    aether_execution::predeploys::installed(&code_at)
        && keccak256(code_at(&PERMIT2)) == PERMIT2_CODE_HASH
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_code_is_the_pinned_mainnet_code() {
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
        assert!(!installed(|address| if *address == PERMIT2 { Bytes::new() } else { code_at(address) }));
        let mut altered = permit2_code().to_vec();
        altered[0] ^= 1;
        assert!(!installed(|address| if *address == PERMIT2 { altered.clone().into() } else { code_at(address) }));
    }
}
