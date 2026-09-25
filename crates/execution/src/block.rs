//! Block execution (docs/design/04-execution.md).
//!
//! `build_block` (proposer) executes candidates in order and keeps only valid
//! ones; `execute_block` (validator) re-executes a proposed block and fails on
//! any invalid tx. Both produce the canonical BAL from revm's touched state.
//! Execution is sequential here; the BAL-driven parallel scheduler replaces the
//! loop without changing outputs (checked by differential tests).

use crate::tx::{tx_hash, validate_stateless, EvmCall};
use crate::world::{StateError, WorldState};
use aether_types::{Address, BalBuilder, BlockAccessList, Bytes, GasVector, TxEnvelope, TxHash, B256, U256};
use revm::context::result::{ExecutionResult, Output};
use revm::context::TxEnv;
use revm::database::WrapDatabaseRef;
use revm::interpreter::{Interpreter, InterpreterTypes};
use revm::primitives::TxKind;
use revm::{Context, InspectEvm, Inspector, MainBuilder, MainContext};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug)]
pub struct BlockContext {
    pub chain_id: u64,
    pub number: u64,
    pub timestamp: u64,
    pub beneficiary: Address,
    pub limits: GasVector,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Receipt {
    pub tx_hash: TxHash,
    pub success: bool,
    pub gas_used: u64,
    pub prove_gas: u64,
    pub contract_address: Option<Address>,
    pub logs: u32,
    pub output: Bytes,
}

#[derive(Clone)]
pub struct BlockOutcome {
    pub state: WorldState,
    pub bal: BlockAccessList,
    pub receipts: Vec<Receipt>,
    pub gas: GasVector,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecError {
    InvalidTx { index: usize, reason: String },
    LimitExceeded { index: usize },
    State(StateError),
}

/// Proving-cost meter: one unit per interpreted instruction (design D4).
/// Replaced by per-opcode zkVM cycle weights once measured (spike S5).
#[derive(Default)]
pub struct ProveGasMeter {
    pub steps: u64,
}

impl<CTX, INTR: InterpreterTypes> Inspector<CTX, INTR> for ProveGasMeter {
    fn step(&mut self, _interp: &mut Interpreter<INTR>, _ctx: &mut CTX) {
        self.steps += 1;
    }
}

struct TxRun {
    receipt: Receipt,
    changes: revm::state::EvmState,
    gas: GasVector,
}

fn run_tx(state: &WorldState, ctx: &BlockContext, tx: &TxEnvelope) -> Result<TxRun, String> {
    let call: EvmCall = validate_stateless(tx, ctx.chain_id).map_err(|e| format!("{e:?}"))?;
    let tx_env = TxEnv::builder()
        .caller(tx.header.sender)
        .nonce(tx.header.nonce)
        .chain_id(Some(ctx.chain_id))
        .gas_limit(call.gas_limit)
        .gas_price(tx.header.max_fee.exec)
        .kind(match call.to {
            Some(a) => TxKind::Call(a),
            None => TxKind::Create,
        })
        .value(call.value)
        .data(call.input.clone())
        .build()
        .map_err(|e| format!("{e:?}"))?;

    let mut evm = Context::mainnet()
        .with_db(WrapDatabaseRef(state))
        .modify_cfg_chained(|c| c.chain_id = ctx.chain_id)
        .modify_block_chained(|b| {
            b.number = U256::from(ctx.number);
            b.timestamp = U256::from(ctx.timestamp);
            b.beneficiary = ctx.beneficiary;
            b.basefee = 0;
            b.gas_limit = ctx.limits.exec;
        })
        .build_mainnet_with_inspector(ProveGasMeter::default());
    let out = evm.inspect_tx(tx_env).map_err(|e| format!("{e:?}"))?;
    let prove_gas = evm.inspector.steps;

    let (success, gas_used, logs, output, contract_address) = match &out.result {
        ExecutionResult::Success { gas, logs, output, .. } => {
            let (bytes, created) = match output {
                Output::Call(b) => (b.clone(), None),
                Output::Create(b, a) => (b.clone(), *a),
            };
            (true, gas.tx_gas_used(), logs.len() as u32, bytes, created)
        }
        ExecutionResult::Revert { gas, output, .. } => (false, gas.tx_gas_used(), 0, output.clone(), None),
        ExecutionResult::Halt { gas, .. } => (false, gas.tx_gas_used(), 0, Bytes::new(), None),
    };
    Ok(TxRun {
        receipt: Receipt { tx_hash: tx_hash(tx), success, gas_used, prove_gas, contract_address, logs, output },
        changes: out.state,
        gas: GasVector { exec: gas_used, state: 0, prove: prove_gas },
    })
}

fn record_bal(bal: &mut BalBuilder, pre: &WorldState, index: u32, changes: &revm::state::EvmState) {
    for (addr, acc) in changes.iter() {
        if !acc.is_touched() {
            continue;
        }
        bal.touch_account(*addr);
        if acc.info.balance != pre.balance(addr) {
            bal.balance(*addr);
        }
        if acc.info.nonce != pre.nonce(addr) {
            bal.nonce(*addr);
        }
        if acc.is_created() {
            bal.code(*addr);
        }
        for (slot, v) in acc.storage.iter() {
            let key = B256::from(slot.to_be_bytes::<32>());
            if v.is_changed() {
                bal.write(*addr, key, index);
            } else {
                bal.read(*addr, key);
            }
        }
    }
}

fn apply(
    state: &mut WorldState,
    bal: &mut BalBuilder,
    receipts: &mut Vec<Receipt>,
    total: &mut GasVector,
    run: TxRun,
) -> Result<(), ExecError> {
    record_bal(bal, state, receipts.len() as u32, &run.changes);
    state.commit(&run.changes).map_err(ExecError::State)?;
    *total = total.checked_add(run.gas).expect("gas bounded by limits");
    receipts.push(run.receipt);
    Ok(())
}

/// Proposer: execute candidates in order, keep the valid ones that fit the limits.
pub fn build_block(pre: &WorldState, ctx: &BlockContext, candidates: Vec<TxEnvelope>) -> (Vec<TxEnvelope>, BlockOutcome) {
    let mut state = pre.clone();
    let (mut bal, mut receipts, mut total, mut included) = (BalBuilder::default(), Vec::new(), GasVector::default(), Vec::new());
    for tx in candidates {
        let Ok(run) = run_tx(&state, ctx, &tx) else { continue };
        match total.checked_add(run.gas) {
            Some(t) if t.fits(&ctx.limits) => {}
            _ => continue,
        }
        if apply(&mut state, &mut bal, &mut receipts, &mut total, run).is_err() {
            continue;
        }
        included.push(tx);
    }
    (included, BlockOutcome { state, bal: bal.build(), receipts, gas: total })
}

/// FOCIL append check: would `tx` be valid if appended to a block whose
/// post-state is `post` and that already used `used` gas? Inclusion-list txs
/// for which this holds must not be left out.
pub fn can_append(post: &WorldState, ctx: &BlockContext, used: GasVector, tx: &TxEnvelope) -> bool {
    let Ok(run) = run_tx(post, ctx, tx) else { return false };
    matches!(used.checked_add(run.gas), Some(t) if t.fits(&ctx.limits))
}

/// Validator: every tx must be valid and the whole block must fit the limits.
pub fn execute_block(pre: &WorldState, ctx: &BlockContext, txs: &[TxEnvelope]) -> Result<BlockOutcome, ExecError> {
    let mut state = pre.clone();
    let (mut bal, mut receipts, mut total) = (BalBuilder::default(), Vec::new(), GasVector::default());
    for (index, tx) in txs.iter().enumerate() {
        let run = run_tx(&state, ctx, tx).map_err(|reason| ExecError::InvalidTx { index, reason })?;
        match total.checked_add(run.gas) {
            Some(t) if t.fits(&ctx.limits) => {}
            _ => return Err(ExecError::LimitExceeded { index }),
        }
        apply(&mut state, &mut bal, &mut receipts, &mut total, run)?;
    }
    Ok(BlockOutcome { state, bal: bal.build(), receipts, gas: total })
}
