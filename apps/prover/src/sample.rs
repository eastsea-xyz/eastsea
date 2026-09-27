//! A sample block for `self-test`: `n` P-256-signed payments on a state that
//! also holds 1000 unrelated accounts (same shape as the Jolt spike's witness).

use aether_crypto::{P256Signer, Signer};
use aether_execution::{execute_block_sequential, sign_call, BlockContext, EvmCall, WorldState};
use aether_types::{Address, Bytes, GasVector, U256};

use crate::program::BoxError;

const CHAIN: u64 = 7777;

/// The postcard `BlockInput` and the statement commitment native execution gives.
pub fn block(n: usize) -> Result<(Vec<u8>, [u8; 32]), BoxError> {
    let signers = (0..n)
        .map(|i| {
            let mut seed = [0u8; 32];
            seed[0] = 0x11;
            seed[30] = (i >> 8) as u8;
            seed[31] = (i as u8).wrapping_add(1);
            P256Signer::from_seed(&seed)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut pre = WorldState::default();
    for s in &signers {
        pre.set_balance(aether_crypto::address_of(&s.public_key())?, U256::from(10u128.pow(21)))?;
    }
    for i in 0..1000u32 {
        let mut a = [0u8; 20];
        a[..4].copy_from_slice(&i.to_be_bytes());
        a[19] = 0xee;
        pre.set_balance(Address::from(a), U256::from(1u64))?;
    }
    let txs = signers
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let call = EvmCall {
                to: Some(Address::repeat_byte(0x40 + (i % 100) as u8)),
                value: U256::from(1000u64),
                input: Bytes::new(),
                gas_limit: 21_000,
                delegate: None,
            };
            sign_call(s, CHAIN, 0, 1, &call)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let ctx = BlockContext {
        chain_id: CHAIN,
        number: 1,
        timestamp: 1,
        beneficiary: Address::repeat_byte(0xbe),
        limits: GasVector { exec: 30_000_000, state: u64::MAX, prove: u64::MAX },
        fees: None,
    };
    let input = aether_proving::block::input(&pre, &ctx, &txs, &[], Address::repeat_byte(0x9a)).map_err(|e| format!("{e:?}"))?;
    let statement = aether_proving::block::execute(&input).map_err(|e| format!("{e:?}"))?;
    let native_root = execute_block_sequential(&pre, &ctx, &txs).map_err(|e| format!("{e:?}"))?.state.root();
    if statement.post_state_root != native_root {
        return Err("sample block: statement post-state root differs from native execution".into());
    }
    Ok((postcard::to_allocvec(&input)?, aether_proving::block::claim(statement.commitment(), input.prover)))
}
