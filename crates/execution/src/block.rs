//! Block execution (docs/design/04-execution.md).
//!
//! `build_block` (proposer) executes candidates in order and keeps only valid
//! ones; `execute_block` (validator) re-executes a proposed block and fails on
//! any invalid tx. Both produce the canonical BAL from revm's touched state.
//! Txs are first executed speculatively in parallel against the pre-state and
//! then committed in block order; a tx whose reads overlap earlier writes is
//! re-executed (see `parallel.rs`). Outputs equal sequential execution
//! (checked by differential tests).

use crate::fees::{
    settle, FeePolicy, Settlement, FEE_COLLECTOR, MAX_NEW_SLOTS_PER_BLOCK, MAX_STATE_UNITS_PER_BLOCK,
    PROVER_ESCROW, STATE_ACCOUNT_UNITS, STATE_SLOT_UNITS, STATE_UNIT_PRICE,
};
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
use revm::primitives::KECCAK_EMPTY;
use revm::{Context, InspectEvm, Inspector, MainBuilder, MainContext};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub state_gas: u64,
    #[serde(default, skip_serializing_if = "U256::is_zero")]
    pub state_fee: U256,
    pub contract_address: Option<Address>,
    pub logs: u32,
    pub output: Bytes,
    /// The events the tx emitted (for apps: trades, transfers).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<Event>,
}

fn is_zero_u64(n: &u64) -> bool { *n == 0 }

/// An EVM log.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    pub address: Address,
    pub topics: Vec<aether_types::B256>,
    pub data: Bytes,
}

/// Result of a read-only call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallResult {
    pub success: bool,
    pub output: Bytes,
    pub gas_used: u64,
}

/// Run a call against `state` without changing it (eth_call): no fees, the
/// caller's current nonce, nothing committed.
pub fn call(state: &WorldState, ctx: &BlockContext, from: Address, to: Option<Address>, data: Bytes, value: U256, gas: u64) -> Result<CallResult, String> {
    let tx_env = TxEnv::builder()
        .caller(from)
        .nonce(state.nonce(&from))
        .chain_id(Some(ctx.chain_id))
        .gas_limit(gas)
        .gas_price(0)
        .kind(match to {
            Some(a) => TxKind::Call(a),
            None => TxKind::Create,
        })
        .value(value)
        .data(data)
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
    Ok(match out.result {
        ExecutionResult::Success { gas, output, .. } => CallResult { success: true, output: output.data().clone(), gas_used: gas.tx_gas_used() },
        ExecutionResult::Revert { gas, output, .. } => CallResult { success: false, output, gas_used: gas.tx_gas_used() },
        ExecutionResult::Halt { gas, .. } => CallResult { success: false, output: Bytes::new(), gas_used: gas.tx_gas_used() },
    })
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
    pub(crate) state_fee: U256,
    pub(crate) new_slots: u64,
}

/// The legacy state limit was unused and always unlimited. The node replaces
/// it with this finite consensus limit only for a new-genesis configuration.
fn state_growth_enabled(ctx: &BlockContext) -> bool { ctx.limits.state == MAX_STATE_UNITS_PER_BLOCK }

/// Count the committed difference, including constructor writes, rather than
/// SSTORE opcodes. A set-then-clear, revert, or destroyed account adds nothing.
fn state_growth(
    pre: &WorldState,
    changes: &revm::state::EvmState,
    sender: Address,
    beneficiary: Address,
    beneficiary_credit: U256,
) -> Result<(u64, u64), String> {
    let mut units = 0u64;
    let mut slots = 0u64;
    for (addr, acc) in changes {
        if !acc.is_touched() || acc.is_selfdestructed() || (acc.is_empty() && !acc.is_created()) { continue; }
        for (slot, value) in &acc.storage {
            if value.is_changed() && !value.present_value.is_zero() && pre.storage(addr, *slot).is_zero() {
                slots = slots.checked_add(1).ok_or("state slot count overflow")?;
                units = units.checked_add(STATE_SLOT_UNITS).ok_or("state units overflow")?;
            }
        }
        let old_code_hash = pre.code_hash(addr);
        if acc.info.code_hash != KECCAK_EMPTY && acc.info.code_hash != old_code_hash {
            let bytes = acc.info.code.as_ref().map_or(0, |c| c.original_bytes().len());
            units = units.checked_add(u64::try_from(bytes).map_err(|_| "code size overflow")?).ok_or("state units overflow")?;
        }
        if pre.account(addr).is_none() {
            // A sender's first free transaction creates only its nonce record.
            let bare_sender = *addr == sender && acc.info.balance.is_zero() && acc.info.code_hash == KECCAK_EMPTY && acc.storage.values().all(|v| v.present_value.is_zero());
            // revm may create the block's fee recipient just to credit tips;
            // that protocol credit is not state created by the user's call.
            let fee_only = *addr == beneficiary
                && acc.info.balance == beneficiary_credit
                && acc.info.nonce == 0
                && acc.info.code_hash == KECCAK_EMPTY
                && acc.storage.values().all(|v| v.present_value.is_zero());
            if !bare_sender && !fee_only {
                units = units.checked_add(STATE_ACCOUNT_UNITS).ok_or("state units overflow")?;
            }
        }
    }
    Ok((units, slots))
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

/// Admission uses the same state-diff and affordability rule as block replay.
/// A queued future nonce is simulated on a private state clone; the signed tx
/// still runs through the same state-diff and affordability checks.
pub fn check_admission(state: &WorldState, ctx: &BlockContext, tx: &TxEnvelope) -> Result<(), String> {
    if tx.header.nonce > state.nonce(&tx.header.sender) {
        let simulated = state.with_sender_nonce(tx.header.sender, tx.header.nonce);
        run_tx(&simulated, ctx, tx).map(|_| ())
    } else {
        run_tx(state, ctx, tx).map(|_| ())
    }
}

/// Execute a tx whose signature and payload were already checked (`call` is its decoded payload).
pub(crate) fn run_validated(state: &WorldState, ctx: &BlockContext, tx: &TxEnvelope, call: &EvmCall) -> Result<TxRun, String> {
    // Reserve the declared budgets before EVM execution. The exact state
    // charge is checked against the final diff before it can be committed.
    let state_enabled = state_growth_enabled(ctx);
    let (prove_reserve, state_reserve) = check_budget(state, ctx, tx, call)?;
    let reserve = prove_reserve + state_reserve;
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

    let (success, gas_used, logs, output, contract_address, events) = match &out.result {
        ExecutionResult::Success { gas, logs, output, .. } => {
            let (bytes, created) = match output {
                Output::Call(b) => (b.clone(), None),
                Output::Create(b, a) => (b.clone(), *a),
            };
            let events = logs.iter().map(|l| Event { address: l.address, topics: l.data.topics().to_vec(), data: l.data.data.clone() }).collect();
            (true, gas.tx_gas_used(), logs.len() as u32, bytes, created, events)
        }
        ExecutionResult::Revert { gas, output, .. } => (false, gas.tx_gas_used(), 0, output.clone(), None, vec![]),
        ExecutionResult::Halt { gas, .. } => (false, gas.tx_gas_used(), 0, Bytes::new(), None, vec![]),
    };
    let mut changes = out.state;
    let beneficiary_price = ctx.fees.map_or(tx.header.max_fee.exec, |f| tx.header.tip.min(tx.header.max_fee.exec.saturating_sub(f.base.exec)));
    let beneficiary_credit = U256::from(gas_used) * U256::from(beneficiary_price);
    let (state_gas, new_slots) = if state_enabled {
        state_growth(state, &changes, tx.header.sender, ctx.beneficiary, beneficiary_credit)?
    } else { (0, 0) };
    if state_gas > tx.header.gas.state {
        return Err(format!("state growth {state_gas} exceeds transaction budget {}", tx.header.gas.state));
    }
    if new_slots > MAX_NEW_SLOTS_PER_BLOCK {
        return Err(format!("new storage slots {new_slots} exceed transaction limit {MAX_NEW_SLOTS_PER_BLOCK}"));
    }
    let state_fee = U256::from(state_gas) * U256::from(if state_enabled { STATE_UNIT_PRICE } else { 0 });
    let prove_fee = match &ctx.fees {
        Some(f) => settle_prove(&mut changes, f, tx, prove_gas, prove_reserve),
        None => U256::ZERO,
    };
    if state_enabled {
        let acc = changes.get_mut(&tx.header.sender).ok_or("missing sender after execution")?;
        acc.info.balance = acc.info.balance + state_reserve - state_fee;
        acc.mark_touch();
    }
    Ok(TxRun {
        receipt: Receipt { tx_hash: tx_hash(tx), success, gas_used, prove_gas, state_gas, state_fee, contract_address, logs, output, events },
        changes,
        gas: GasVector { exec: gas_used, state: state_gas, prove: prove_gas },
        touched_beneficiary,
        prove_fee,
        state_fee,
        new_slots,
    })
}

/// The sender must accept the base prove fee, declare a prove budget covering
/// the gas limit (each interpreted step costs at least one gas, so it cannot run
/// out), and hold value + max exec fee + that budget. Returns the budget, which
/// is set aside before execution so the tx cannot spend it.
fn check_budget(state: &WorldState, ctx: &BlockContext, tx: &TxEnvelope, call: &EvmCall) -> Result<(U256, U256), String> {
    if (ctx.fees.is_some() || state_growth_enabled(ctx)) && tx.header.gas.prove < call.gas_limit {
        return Err(format!("prove budget {} below the gas limit {}", tx.header.gas.prove, call.gas_limit));
    }
    let prove = if let Some(f) = &ctx.fees {
        if tx.header.max_fee.prove < f.base.prove {
            return Err(format!("max prove fee {} below base {}", tx.header.max_fee.prove, f.base.prove));
        }
        U256::from(tx.header.gas.prove) * U256::from(f.base.prove)
    } else { U256::ZERO };
    let growth = if state_growth_enabled(ctx) {
        if tx.header.gas.state > ctx.limits.state {
            return Err("transaction state budget exceeds block limit".into());
        }
        if tx.header.gas.state > 0 && tx.header.max_fee.state < STATE_UNIT_PRICE {
            return Err("state fee cap below the fixed state price".into());
        }
        U256::from(tx.header.gas.state) * U256::from(STATE_UNIT_PRICE)
    } else { U256::ZERO };
    let need = U256::from(call.gas_limit) * U256::from(tx.header.max_fee.exec) + prove + growth;
    match need.checked_add(call.value) {
        Some(n) if state.balance(&tx.header.sender) >= n => Ok((prove, growth)),
        _ => Err("insufficient funds for gas, prove budget, state budget and value".into()),
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
    state_fees: U256,
    new_slots: u64,
}

impl Acc {
    fn apply(&mut self, state: &mut WorldState, run: TxRun) -> Result<(), ExecError> {
        record_bal(&mut self.bal, state, self.receipts.len() as u32, &run.changes);
        state.commit(&run.changes).map_err(ExecError::State)?;
        self.total = self.total.checked_add(run.gas).expect("gas bounded by limits");
        self.prove_fees += run.prove_fee;
        self.state_fees += run.state_fee;
        self.new_slots += run.new_slots;
        self.receipts.push(run.receipt);
        Ok(())
    }

    /// Settle fees (if any) and seal the block.
    fn finish(mut self, mut state: WorldState, ctx: &BlockContext) -> BlockOutcome {
        let mut settlement = match &ctx.fees {
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
        settlement.burned_state = self.state_fees;
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
        if acc.new_slots + run.new_slots > MAX_NEW_SLOTS_PER_BLOCK && state_growth_enabled(ctx) { continue; }
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
        if acc.new_slots + run.new_slots > MAX_NEW_SLOTS_PER_BLOCK && state_growth_enabled(ctx) {
            return Err(ExecError::LimitExceeded { index });
        }
        sched.committing(&state, &run.changes);
        acc.apply(&mut state, run)?;
    }
    Ok(acc.finish(state, ctx))
}
