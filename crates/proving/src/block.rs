//! What a block proof proves (launch plan step 6).
//!
//! The prover (a Jolt guest on a Mac's GPU) runs [`execute`] on a
//! [`BlockInput`]: it rebuilds the pre-state from the stateless witness, checks
//! its root, applies the block's protocol activations, re-executes the
//! transactions and outputs the [`BlockStatement`]'s commitment. A verifier
//! recomputes the statement from the certified chain (this block's context and
//! transactions, its parent state root, the post-state root its child commits
//! to, its gas) and accepts the proof only if the commitments are equal. The
//! same function runs natively, so node and guest cannot disagree on it.

use aether_execution::{execute_block_sequential, BlockContext, StateWitness, WorldState};
use aether_types::{GasVector, TxEnvelope, B256};
use serde::{Deserialize, Serialize};

/// The prover's private input for one block.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BlockInput {
    pub ctx: BlockContext,
    pub txs: Vec<TxEnvelope>,
    /// Protocols that activate at this block, in order (their one-time state
    /// changes run before the transactions; usually empty).
    pub activate: Vec<u32>,
    /// The state root the block builds on (its parent's post-state).
    pub pre_state_root: B256,
    pub witness: StateWitness,
}

/// The public claim: this block, on this pre-state, gives this post-state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockStatement {
    pub ctx: BlockContext,
    /// BLAKE3 of the postcard-encoded transactions.
    pub txs_hash: [u8; 32],
    pub activate: Vec<u32>,
    pub pre_state_root: B256,
    pub post_state_root: B256,
    pub gas: GasVector,
}

impl BlockStatement {
    /// The 32 bytes a proof outputs and a verifier compares.
    pub fn commitment(&self) -> [u8; 32] {
        *blake3::hash(&postcard::to_allocvec(self).expect("statement encodes")).as_bytes()
    }
}

pub fn txs_hash(txs: &[TxEnvelope]) -> [u8; 32] {
    *blake3::hash(&postcard::to_allocvec(txs).expect("txs encode")).as_bytes()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockProofError {
    Witness(String),
    /// The witness does not give the claimed pre-state root.
    PreStateRoot,
    Activation(String),
    Execution(String),
}

/// Re-execute a block on its witness and state what it did.
pub fn execute(input: &BlockInput) -> Result<BlockStatement, BlockProofError> {
    let mut pre = WorldState::from_witness(&input.witness).map_err(|e| BlockProofError::Witness(format!("{e:?}")))?;
    if pre.root() != input.pre_state_root {
        return Err(BlockProofError::PreStateRoot);
    }
    for p in &input.activate {
        aether_execution::forks::activate(*p, &mut pre).map_err(|e| BlockProofError::Activation(e.to_string()))?;
    }
    let out = execute_block_sequential(&pre, &input.ctx, &input.txs).map_err(|e| BlockProofError::Execution(format!("{e:?}")))?;
    Ok(BlockStatement {
        ctx: input.ctx.clone(),
        txs_hash: txs_hash(&input.txs),
        activate: input.activate.clone(),
        pre_state_root: input.pre_state_root,
        post_state_root: out.state.root(),
        gas: out.gas,
    })
}

/// The prover's input for a block, from the full pre-state (a node that holds it).
pub fn input(pre: &WorldState, ctx: &BlockContext, txs: &[TxEnvelope], activate: &[u32]) -> Result<BlockInput, BlockProofError> {
    let mut recording = pre.clone();
    recording.record_access();
    for p in activate {
        aether_execution::forks::activate(*p, &mut recording).map_err(|e| BlockProofError::Activation(e.to_string()))?;
    }
    let out = aether_execution::execute_block(&recording, ctx, txs).map_err(|e| BlockProofError::Execution(format!("{e:?}")))?;
    Ok(BlockInput { ctx: ctx.clone(), txs: txs.to_vec(), activate: activate.to_vec(), pre_state_root: pre.root(), witness: pre.witness_for(&out.state) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use aether_crypto::{P256Signer, Signer};
    use aether_execution::{sign_call, EvmCall};
    use aether_types::{Address, Bytes, U256};

    fn setup() -> (WorldState, BlockContext, Vec<TxEnvelope>) {
        let signers: Vec<P256Signer> = (1..=5u8).map(|i| P256Signer::from_seed(&[i; 32]).unwrap()).collect();
        let mut pre = WorldState::default();
        for s in &signers {
            pre.set_balance(aether_crypto::address_of(&s.public_key()).unwrap(), U256::from(10u128.pow(21))).unwrap();
        }
        for i in 0..100u8 {
            pre.set_balance(Address::repeat_byte(i), U256::from(1u64)).unwrap();
        }
        let ctx = BlockContext {
            chain_id: 7,
            number: 1,
            timestamp: 1,
            beneficiary: Address::repeat_byte(0xbe),
            limits: GasVector { exec: 30_000_000, state: u64::MAX, prove: u64::MAX },
            fees: None,
        };
        let txs = signers
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let call = EvmCall {
                    to: Some(Address::repeat_byte(0x40 + i as u8)),
                    value: U256::from(1000u64),
                    input: Bytes::new(),
                    gas_limit: 21_000,
                    delegate: None,
                };
                sign_call(s, 7, 0, 1, &call).unwrap()
            })
            .collect();
        (pre, ctx, txs)
    }

    #[test]
    fn the_statement_matches_full_execution_and_binds_everything() {
        let (pre, ctx, txs) = setup();
        let input = input(&pre, &ctx, &txs, &[]).unwrap();
        let st = execute(&input).unwrap();
        let full = aether_execution::execute_block(&pre, &ctx, &txs).unwrap();
        assert_eq!(st.post_state_root, full.state.root());
        assert_eq!(st.gas, full.gas);
        assert_eq!(st.pre_state_root, pre.root());

        // A wrong pre-state root is refused; other txs or another context state something else.
        let mut wrong = input.clone();
        wrong.pre_state_root = B256::repeat_byte(1);
        assert_eq!(execute(&wrong), Err(BlockProofError::PreStateRoot));
        let mut fewer = input.clone();
        fewer.txs.pop();
        assert_ne!(execute(&fewer).unwrap().commitment(), st.commitment());
        let mut later = input.clone();
        later.ctx.timestamp += 1;
        assert_ne!(execute(&later).unwrap().commitment(), st.commitment());
    }
}
