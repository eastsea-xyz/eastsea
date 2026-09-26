//! Block execution (docs/design/04-execution.md).
//!
//! `build_block` (proposer) executes candidates in order and keeps only valid
//! ones; `execute_block` (validator) re-executes a proposed block and fails on
//! any invalid tx. Both produce the canonical BAL from revm's touched state.
//! Txs are first executed speculatively in parallel against the pre-state and
//! then committed in block order; a tx whose reads overlap earlier writes is
//! re-executed (see `parallel.rs`). Outputs equal sequential execution
//! (checked by differential tests).

use crate::fees::{settle, FeePolicy, Settlement, FEE_COLLECTOR, PROVER_ESCROW};
use crate::parallel::Scheduler;
use crate::tx::{tx_hash, validate_stateless, EvmCall};
use crate::world::{StateError, WorldState};
use aether_types::{Address, BalBuilder, BlockAccessList, Bytes, GasVector, TxEnvelope, TxHash, B256, U256};
use revm::context::result::{ExecutionResult, Output};
use revm::context::TxEnv;
use revm::context_interface::transaction::{Authorization, RecoveredAuthority, RecoveredAuthorization};
use revm::database::WrapDatabaseRef;
use revm::interpreter::interpreter_types::{Jumps, StackTr};
use revm::interpreter::{CallInputs, CallOutcome, Interpreter, InterpreterTypes};
use revm::primitives::TxKind;
use revm::{Context, InspectEvm, Inspector, MainBuilder, MainContext};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug)]
pub struct BlockContext {
    pub chain_id: u64,
    pub number: u64,
    pub timestamp: u64,
    /// Where revm credits priority fees ([`crate::fees::FEE_COLLECTOR`] when `fees` is set).
    pub beneficiary: Address,
    pub limits: GasVector,
    /// Fee policy (docs/research/tokenomics-2026.md); `None` = no base fee, tips to `beneficiary`.
    pub fees: Option<FeePolicy>,
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
    /// Where the fees went (zero without a fee policy).
    pub settlement: Settlement,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecError {
    InvalidTx { index: usize, reason: String },
    LimitExceeded { index: usize },
    State(StateError),
}

/// Proving-cost meter: one unit per interpreted instruction (design D4).
/// Replaced by per-opcode zkVM cycle weights once measured (spike S5).
///
/// Also reports whether execution looked at `watch` (the fee recipient) other
/// than through the fee credit: then a speculative result is not reusable.
#[derive(Default)]
pub struct ProveGasMeter {
    pub steps: u64,
    pub watch: Option<Address>,
    pub watched: bool,
}

const BALANCE: u8 = 0x31;
const EXTCODESIZE: u8 = 0x3b;
const EXTCODECOPY: u8 = 0x3c;
const EXTCODEHASH: u8 = 0x3f;
const SELFDESTRUCT: u8 = 0xff;

impl<CTX, INTR: InterpreterTypes> Inspector<CTX, INTR> for ProveGasMeter
where
    INTR::Bytecode: Jumps,
    INTR::Stack: StackTr,
{
    fn step(&mut self, interp: &mut Interpreter<INTR>, _ctx: &mut CTX) {
        self.steps += 1;
        let Some(w) = self.watch else { return };
        if matches!(interp.bytecode.opcode(), BALANCE | EXTCODESIZE | EXTCODECOPY | EXTCODEHASH | SELFDESTRUCT) {
            if let Some(top) = interp.stack.data().last() {
                if Address::from_word(B256::from(top.to_be_bytes::<32>())) == w {
                    self.watched = true;
                }
            }
        }
    }

    fn call(&mut self, _ctx: &mut CTX, inputs: &mut CallInputs) -> Option<CallOutcome> {
        if let Some(w) = self.watch {
            if inputs.target_address == w || inputs.bytecode_address == w || inputs.caller == w {
                self.watched = true;
            }
        }
        None
    }
}

pub(crate) struct TxRun {
    pub(crate) receipt: Receipt,
    pub(crate) changes: revm::state::EvmState,
    pub(crate) gas: GasVector,
    /// Execution observed the fee recipient beyond the fee credit.
    pub(crate) touched_beneficiary: bool,
    /// Base prove fee debited from the sender (goes to the prover escrow).
    pub(crate) prove_fee: U256,
}

/// A self-delegation as an EIP-7702 authorization whose authority is the tx
/// sender (already authenticated by its own signature, e.g. P-256). The sender's
/// nonce is bumped before authorizations are applied, hence `nonce + 1`.
fn delegation(tx: &TxEnvelope, ctx: &BlockContext, call: &EvmCall) -> Vec<RecoveredAuthorization> {
    call.delegate
        .map(|target| {
            let auth = Authorization { chain_id: U256::from(ctx.chain_id), address: target, nonce: tx.header.nonce + 1 };
            RecoveredAuthorization::new_unchecked(auth, RecoveredAuthority::Valid(tx.header.sender))
        })
        .into_iter()
        .collect()
}

pub(crate) fn run_tx(state: &WorldState, ctx: &BlockContext, tx: &TxEnvelope) -> Result<TxRun, String> {
    let call = validate_stateless(tx, ctx.chain_id).map_err(|e| format!("{e:?}"))?;
    run_validated(state, ctx, tx, &call)
}

/// Execute a tx whose signature and payload were already checked (`call` is its decoded payload).
pub(crate) fn run_validated(state: &WorldState, ctx: &BlockContext, tx: &TxEnvelope, call: &EvmCall) -> Result<TxRun, String> {
    // Everything that can reject a tx is checked before it runs: a tx that
    // executes always pays for its execution and proving.
    let reserve = match &ctx.fees {
        Some(f) => check_prove_budget(state, f, tx, call)?,
        None => U256::ZERO,
    };
    let tx_env = TxEnv::builder()
        .caller(tx.header.sender)
        .nonce(tx.header.nonce)
        .chain_id(Some(ctx.chain_id))
        .gas_limit(call.gas_limit)
        .gas_price(tx.header.max_fee.exec)
        // EIP-1559 pricing under a fee policy; legacy (price = cap) otherwise.
        .gas_priority_fee(ctx.fees.map(|_| tx.header.tip))
        .kind(match call.to {
            Some(a) => TxKind::Call(a),
            None => TxKind::Create,
        })
        .value(call.value)
        .data(call.input.clone())
        .authorization_list_recovered(delegation(tx, ctx, call))
        .build()
        .map_err(|e| format!("{e:?}"))?;

    let mut evm = Context::mainnet()
        .with_db(WrapDatabaseRef(Reserved { state, who: tx.header.sender, amount: reserve }))
        .modify_cfg_chained(|c| c.chain_id = ctx.chain_id)
        .modify_block_chained(|b| {
            b.number = U256::from(ctx.number);
            b.timestamp = U256::from(ctx.timestamp);
            b.beneficiary = ctx.beneficiary;
            b.basefee = ctx.fees.map_or(0, |f| u64::try_from(f.base.exec).unwrap_or(u64::MAX));
            b.gas_limit = ctx.limits.exec;
        })
        .build_mainnet_with_inspector(ProveGasMeter { watch: Some(ctx.beneficiary), ..Default::default() });
    let out = evm.inspect_tx(tx_env).map_err(|e| format!("{e:?}"))?;
    let prove_gas = evm.inspector.steps;
    let touched_beneficiary = evm.inspector.watched || tx.header.sender == ctx.beneficiary || call.to == Some(ctx.beneficiary);

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
    let mut changes = out.state;
    let prove_fee = match &ctx.fees {
        Some(f) => settle_prove(&mut changes, f, tx, prove_gas, reserve),
        None => U256::ZERO,
    };
    Ok(TxRun {
        receipt: Receipt { tx_hash: tx_hash(tx), success, gas_used, prove_gas, contract_address, logs, output },
        changes,
        gas: GasVector { exec: gas_used, state: 0, prove: prove_gas },
        touched_beneficiary,
        prove_fee,
    })
}

/// The sender must accept the base prove fee, declare a prove budget covering
/// the gas limit (each interpreted step costs at least one gas, so it cannot run
/// out), and hold value + max exec fee + that budget. Returns the budget, which
/// is set aside before execution so the tx cannot spend it.
fn check_prove_budget(state: &WorldState, f: &FeePolicy, tx: &TxEnvelope, call: &EvmCall) -> Result<U256, String> {
    if tx.header.max_fee.prove < f.base.prove {
        return Err(format!("max prove fee {} below base {}", tx.header.max_fee.prove, f.base.prove));
    }
    if tx.header.gas.prove < call.gas_limit {
        return Err(format!("prove budget {} below the gas limit {}", tx.header.gas.prove, call.gas_limit));
    }
    let reserve = U256::from(tx.header.gas.prove) * U256::from(f.base.prove);
    let need = U256::from(call.gas_limit) * U256::from(tx.header.max_fee.exec) + reserve;
    match need.checked_add(call.value) {
        Some(n) if state.balance(&tx.header.sender) >= n => Ok(reserve),
        _ => Err("insufficient funds for gas, prove budget and value".into()),
    }
}

/// Return the reserved prove budget to the sender minus what proving costs
/// (`prove_gas × base_prove`, capped at the budget). Never fails.
fn settle_prove(changes: &mut revm::state::EvmState, f: &FeePolicy, tx: &TxEnvelope, prove_gas: u64, reserve: U256) -> U256 {
    let fee = (U256::from(prove_gas) * U256::from(f.base.prove)).min(reserve);
    if let Some(acc) = changes.get_mut(&tx.header.sender) {
        acc.info.balance = acc.info.balance + reserve - fee;
        acc.mark_touch();
    }
    fee
}

/// The state as the EVM sees it while a tx runs: its prove budget is set aside.
struct Reserved<'a> {
    state: &'a WorldState,
    who: Address,
    amount: U256,
}

impl revm::database::DatabaseRef for Reserved<'_> {
    type Error = StateError;

    fn basic_ref(&self, address: Address) -> Result<Option<revm::state::AccountInfo>, Self::Error> {
        let mut info = self.state.basic_ref(address)?;
        if address == self.who {
            if let Some(i) = info.as_mut() {
                i.balance = i.balance.saturating_sub(self.amount);
            }
        }
        Ok(info)
    }

    fn code_by_hash_ref(&self, code_hash: B256) -> Result<revm::state::Bytecode, Self::Error> {
        self.state.code_by_hash_ref(code_hash)
    }

    fn storage_ref(&self, address: Address, index: U256) -> Result<U256, Self::Error> {
        self.state.storage_ref(address, index)
    }

    fn block_hash_ref(&self, number: u64) -> Result<B256, Self::Error> {
        self.state.block_hash_ref(number)
    }
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
        if acc.is_created() || acc.info.code_hash != pre.code_hash(addr) {
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

#[derive(Default)]
struct Acc {
    bal: BalBuilder,
    receipts: Vec<Receipt>,
    total: GasVector,
    prove_fees: U256,
}

impl Acc {
    fn apply(&mut self, state: &mut WorldState, run: TxRun) -> Result<(), ExecError> {
        record_bal(&mut self.bal, state, self.receipts.len() as u32, &run.changes);
        state.commit(&run.changes).map_err(ExecError::State)?;
        self.total = self.total.checked_add(run.gas).expect("gas bounded by limits");
        self.prove_fees += run.prove_fee;
        self.receipts.push(run.receipt);
        Ok(())
    }

    /// Settle fees (if any) and seal the block.
    fn finish(mut self, mut state: WorldState, ctx: &BlockContext) -> BlockOutcome {
        let settlement = match &ctx.fees {
            Some(f) => {
                let pre = [FEE_COLLECTOR, f.proposer, PROVER_ESCROW].map(|a| (a, state.balance(&a)));
                let s = settle(&mut state, f, self.prove_fees);
                for (a, before) in pre {
                    if state.balance(&a) != before {
                        self.bal.touch_account(a);
                        self.bal.balance(a);
                    }
                }
                s
            }
            None => Settlement::default(),
        };
        BlockOutcome { state, bal: self.bal.build(), receipts: self.receipts, gas: self.total, settlement }
    }
}

/// Proposer: execute candidates in order, keep the valid ones that fit the limits.
pub fn build_block(pre: &WorldState, ctx: &BlockContext, candidates: Vec<TxEnvelope>) -> (Vec<TxEnvelope>, BlockOutcome) {
    build_block_with(pre, ctx, candidates, true)
}

/// `build_block` without speculation (reference for differential tests).
pub fn build_block_sequential(pre: &WorldState, ctx: &BlockContext, candidates: Vec<TxEnvelope>) -> (Vec<TxEnvelope>, BlockOutcome) {
    build_block_with(pre, ctx, candidates, false)
}

fn build_block_with(pre: &WorldState, ctx: &BlockContext, candidates: Vec<TxEnvelope>, parallel: bool) -> (Vec<TxEnvelope>, BlockOutcome) {
    let mut state = pre.clone();
    state.clear_journal();
    let (mut acc, mut included) = (Acc::default(), Vec::new());
    let mut sched = Scheduler::new(pre, ctx, &candidates, parallel);
    for (i, tx) in candidates.into_iter().enumerate() {
        let Ok(run) = sched.run(i, &state, ctx, &tx) else { continue };
        match acc.total.checked_add(run.gas) {
            Some(t) if t.fits(&ctx.limits) => {}
            _ => continue,
        }
        sched.committing(&state, &run.changes);
        if acc.apply(&mut state, run).is_err() {
            continue;
        }
        included.push(tx);
    }
    (included, acc.finish(state, ctx))
}

/// FOCIL append check: would `tx` be valid if appended to a block whose
/// post-state is `post` and that already used `used` gas? Inclusion-list txs
/// for which this holds must not be left out.
pub fn can_append(post: &WorldState, ctx: &BlockContext, used: GasVector, tx: &TxEnvelope) -> bool {
    // `post` already includes the end-of-block fee settlement; a tx from an account
    // it credited may only have become affordable then, so it is never required.
    if let Some(f) = &ctx.fees {
        if [f.proposer, PROVER_ESCROW, FEE_COLLECTOR].contains(&tx.header.sender) {
            return false;
        }
    }
    let Ok(run) = run_tx(post, ctx, tx) else { return false };
    matches!(used.checked_add(run.gas), Some(t) if t.fits(&ctx.limits))
}

/// Validator: every tx must be valid and the whole block must fit the limits.
pub fn execute_block(pre: &WorldState, ctx: &BlockContext, txs: &[TxEnvelope]) -> Result<BlockOutcome, ExecError> {
    execute_block_with(pre, ctx, txs, true)
}

/// `execute_block` without speculation (reference for differential tests).
pub fn execute_block_sequential(pre: &WorldState, ctx: &BlockContext, txs: &[TxEnvelope]) -> Result<BlockOutcome, ExecError> {
    execute_block_with(pre, ctx, txs, false)
}

fn execute_block_with(pre: &WorldState, ctx: &BlockContext, txs: &[TxEnvelope], parallel: bool) -> Result<BlockOutcome, ExecError> {
    let mut state = pre.clone();
    state.clear_journal();
    let mut acc = Acc::default();
    let mut sched = Scheduler::new(pre, ctx, txs, parallel);
    for (index, tx) in txs.iter().enumerate() {
        let run = sched.run(index, &state, ctx, tx).map_err(|reason| ExecError::InvalidTx { index, reason })?;
        match acc.total.checked_add(run.gas) {
            Some(t) if t.fits(&ctx.limits) => {}
            _ => return Err(ExecError::LimitExceeded { index }),
        }
        sched.committing(&state, &run.changes);
        acc.apply(&mut state, run)?;
    }
    Ok(acc.finish(state, ctx))
}
