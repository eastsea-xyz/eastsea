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
    settle, FeePolicy, Settlement, EVENT_BASE_BYTES, FEE_COLLECTOR, MAX_NEW_SLOTS_PER_BLOCK,
    MAX_PERSISTENT_BYTES_PER_BLOCK, MAX_STATE_UNITS_PER_BLOCK, PROVER_ESCROW,
    RECEIPT_BASE_BYTES, RECEIPT_BYTES_PER_STATE_UNIT, STATE_ACCOUNT_UNITS, STATE_SLOT_UNITS,
};
use crate::parallel::Scheduler;
use crate::tx::{tx_hash, validate_stateless, EvmCall};
use crate::world::{StateError, WorldState};
use aether_types::{Address, BalBuilder, BlockAccessList, Bytes, Canonical, GasVector, TxEnvelope, TxHash, B256, U256};
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

/// Consensus accounting for logical receipt bytes. The block limit reserves
/// 16 stored key/value bytes per metered byte for JSON hex expansion and
/// activity indexes; fixed event overhead bounds empty LOG0 rows too.
pub fn receipt_persistent_bytes(receipt: &Receipt) -> u64 {
    receipt.events.iter().fold(RECEIPT_BASE_BYTES + receipt.output.len() as u64, |bytes, event| {
        bytes.saturating_add(EVENT_BASE_BYTES)
            .saturating_add((event.topics.len() as u64).saturating_mul(32))
            .saturating_add(event.data.len() as u64)
    })
}

/// Signed transaction bytes are archived with the block. This excludes the
/// block's shared header and capped protocol system records.
pub fn tx_persistent_bytes(tx: &TxEnvelope) -> u64 { tx.to_canonical_bytes().len() as u64 }

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

/// Non-consensus call targets observed during one transaction's EVM execution.
/// Addresses are unique and sorted; `complete` is false if the capture cap was
/// exceeded. A transaction that ultimately fails exposes no addresses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallTargets {
    pub addresses: Vec<Address>,
    pub complete: bool,
}

const MAX_CALL_TARGETS: usize = 4_096;

impl Default for CallTargets {
    fn default() -> Self { Self { addresses: Vec::new(), complete: true } }
}

impl CallTargets {
    fn record(&mut self, address: Address) {
        if let Err(index) = self.addresses.binary_search(&address) {
            if self.addresses.len() < MAX_CALL_TARGETS {
                self.addresses.insert(index, address);
            } else {
                self.complete = false;
            }
        }
    }
}

#[derive(Clone)]
pub struct BlockOutcome {
    pub state: WorldState,
    pub bal: BlockAccessList,
    pub receipts: Vec<Receipt>,
    /// Local execution metadata, aligned with `receipts`; never serialized or
    /// included in receipt bytes, state roots, gas, fees or commitments.
    pub call_targets: Vec<CallTargets>,
    pub gas: GasVector,
    /// Metered signed transaction and receipt bytes. Zero on the legacy chain.
    pub persistent_bytes: u64,
    /// Net new storage slots, used by append checks on the finalized block.
    pub new_slots: u64,
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
    pub call_targets: CallTargets,
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
        self.call_targets.record(inputs.target_address);
        self.call_targets.record(inputs.bytecode_address);
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
    pub(crate) call_targets: CallTargets,
    pub(crate) changes: revm::state::EvmState,
    pub(crate) gas: GasVector,
    /// Execution observed the fee recipient beyond the fee credit.
    pub(crate) touched_beneficiary: bool,
    /// Base prove fee debited from the sender (goes to the prover escrow).
    pub(crate) prove_fee: U256,
    pub(crate) state_fee: U256,
    pub(crate) new_slots: u64,
    pub(crate) persistent_bytes: u64,
}

/// The legacy state limit was unused and always unlimited. The node replaces
/// it with this finite consensus limit only for a new-genesis configuration.
fn state_growth_enabled(ctx: &BlockContext) -> bool { ctx.limits.state <= MAX_STATE_UNITS_PER_BLOCK }

fn state_price(ctx: &BlockContext) -> u128 {
    // A new-genesis context carries remaining burst capacity even when the
    // execution/proving fee switch is off. Never waive the state surcharge.
    let floor = crate::fees::state_base_fee(MAX_STATE_UNITS_PER_BLOCK.saturating_sub(ctx.limits.state));
    ctx.fees.map_or(floor, |fees| fees.base.state.max(floor))
}

/// Count the committed difference, including constructor writes, rather than
/// SSTORE opcodes. A set-then-clear, revert, or destroyed account adds nothing.
fn state_growth(
    pre: &WorldState,
    changes: &revm::state::EvmState,
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
            // revm may create the block's fee recipient just to credit tips;
            // that protocol credit is not state created by the user's call.
            let fee_only = *addr == beneficiary
                && acc.info.balance == beneficiary_credit
                && acc.info.nonce == 0
                && acc.info.code_hash == KECCAK_EMPTY
                && acc.storage.values().all(|v| v.present_value.is_zero());
            if !fee_only {
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
    check_admission_cost(state, ctx, tx).map(|_| ())
}

/// Admission's exact transaction cost from the block executor. A caller that
/// knows the proposed block's current totals can apply the same cumulative
/// limits as `execute_block`.
/// The wallet-facing reason a transaction does not fit the next block. Only
/// the text differs by dimension; the fit rule itself is `GasVector::fits`.
/// A spent B5 state budget is temporary (it refills per height), unlike an
/// exec/prove overrun, so it must not read like "too big, never".
fn admission_limit_error(gas: &GasVector, limits: &GasVector) -> String {
    if gas.exec <= limits.exec && gas.prove <= limits.prove && gas.state > limits.state {
        if gas.state > MAX_STATE_UNITS_PER_BLOCK {
            return format!(
                "state budget: this transaction needs {} state units, more than any block can take ({MAX_STATE_UNITS_PER_BLOCK}); split it",
                gas.state
            );
        }
        let blocks = (gas.state - limits.state).div_ceil(crate::fees::STATE_UNITS_PER_BLOCK);
        return format!(
            "state budget: this transaction needs {} state units and {} are available right now; the budget refills {} per block, retry in about {blocks} blocks",
            gas.state,
            limits.state,
            crate::fees::STATE_UNITS_PER_BLOCK
        );
    }
    "transaction exceeds block gas limit".into()
}

pub fn check_admission_cost(state: &WorldState, ctx: &BlockContext, tx: &TxEnvelope) -> Result<AdmissionCost, String> {
    let run = if tx.header.nonce > state.nonce(&tx.header.sender) {
        let simulated = state.with_sender_nonce(tx.header.sender, tx.header.nonce);
        run_tx(&simulated, ctx, tx)?
    } else {
        run_tx(state, ctx, tx)?
    };
    if !run.gas.fits(&ctx.limits) {
        return Err(admission_limit_error(&run.gas, &ctx.limits));
    }
    Ok(AdmissionCost { gas: run.gas, persistent_bytes: run.persistent_bytes, new_slots: run.new_slots })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AdmissionCost {
    pub gas: GasVector,
    pub persistent_bytes: u64,
    pub new_slots: u64,
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
    let mut call_targets = std::mem::take(&mut evm.inspector.call_targets);
    if !success { call_targets.addresses.clear(); }
    let mut changes = out.state;
    let beneficiary_price = ctx.fees.map_or(tx.header.max_fee.exec, |f| tx.header.tip.min(tx.header.max_fee.exec.saturating_sub(f.base.exec)));
    let beneficiary_credit = U256::from(gas_used) * U256::from(beneficiary_price);
    let mut receipt = Receipt { tx_hash: tx_hash(tx), success, gas_used, prove_gas, state_gas: 0, state_fee: U256::ZERO, contract_address, logs, output, events };
    let persistent_bytes = if state_enabled {
        receipt_persistent_bytes(&receipt).checked_add(tx_persistent_bytes(tx)).ok_or("persistent byte count overflow")?
    } else { 0 };
    if persistent_bytes > MAX_PERSISTENT_BYTES_PER_BLOCK {
        return Err(format!("persistent bytes {persistent_bytes} exceed transaction limit {MAX_PERSISTENT_BYTES_PER_BLOCK}"));
    }
    let (growth_gas, new_slots) = if state_enabled {
        state_growth(state, &changes, ctx.beneficiary, beneficiary_credit)?
    } else { (0, 0) };
    let receipt_gas = persistent_bytes.div_ceil(RECEIPT_BYTES_PER_STATE_UNIT);
    let state_gas = growth_gas.checked_add(receipt_gas).ok_or("state units overflow")?;
    if state_gas > tx.header.gas.state {
        return Err(format!("state growth {state_gas} exceeds transaction budget {}", tx.header.gas.state));
    }
    if new_slots > MAX_NEW_SLOTS_PER_BLOCK {
        return Err(format!("new storage slots {new_slots} exceed transaction limit {MAX_NEW_SLOTS_PER_BLOCK}"));
    }
    let state_fee = U256::from(state_gas) * U256::from(if state_enabled { state_price(ctx) } else { 0 });
    let prove_fee = match &ctx.fees {
        Some(f) => settle_prove(&mut changes, f, tx, prove_gas, prove_reserve),
        None => U256::ZERO,
    };
    if state_enabled {
        let acc = changes.get_mut(&tx.header.sender).ok_or("missing sender after execution")?;
        acc.info.balance = acc.info.balance + state_reserve - state_fee;
        acc.mark_touch();
    }
    receipt.state_gas = state_gas;
    receipt.state_fee = state_fee;
    Ok(TxRun {
        receipt,
        call_targets,
        changes,
        gas: GasVector { exec: gas_used, state: state_gas, prove: prove_gas },
        touched_beneficiary,
        prove_fee,
        state_fee,
        new_slots,
        persistent_bytes,
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
        // The signed budget can exceed today's remaining capacity: only the
        // actual committed usage consumes it. Wallet estimates remain valid.
        if tx.header.gas.state > MAX_STATE_UNITS_PER_BLOCK {
            return Err("transaction state budget exceeds block limit".into());
        }
        let price = state_price(ctx);
        if tx.header.gas.state > 0 && tx.header.max_fee.state < price {
            return Err("state fee cap below the state base price".into());
        }
        U256::from(tx.header.gas.state) * U256::from(price)
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

#[derive(Clone, Default)]
struct Acc {
    bal: BalBuilder,
    receipts: Vec<Receipt>,
    call_targets: Vec<CallTargets>,
    total: GasVector,
    prove_fees: U256,
    state_fees: U256,
    new_slots: u64,
    persistent_bytes: u64,
}

impl Acc {
    fn apply(&mut self, state: &mut WorldState, run: TxRun) -> Result<(), ExecError> {
        record_bal(&mut self.bal, state, self.receipts.len() as u32, &run.changes);
        state.commit(&run.changes).map_err(ExecError::State)?;
        self.total = self.total.checked_add(run.gas).expect("gas bounded by limits");
        self.prove_fees += run.prove_fee;
        self.state_fees += run.state_fee;
        self.new_slots += run.new_slots;
        self.persistent_bytes += run.persistent_bytes;
        self.receipts.push(run.receipt);
        self.call_targets.push(run.call_targets);
        Ok(())
    }

    /// Settle fees (if any) and seal the block.
    fn finish(mut self, mut state: WorldState, ctx: &BlockContext) -> BlockOutcome {
        let mut settlement = match &ctx.fees {
            Some(f) => {
                let pre = [FEE_COLLECTOR, f.proposer, PROVER_ESCROW].map(|a| (a, state.balance(&a)));
                let fee_bal_before = [FEE_COLLECTOR, f.proposer, PROVER_ESCROW]
                    .map(|a| self.bal.account_accesses(a));
                let mut s = settle(&mut state, f, self.prove_fees);
                s.fee_bal_before = fee_bal_before;
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
        BlockOutcome { state, bal: self.bal.build(), receipts: self.receipts, call_targets: self.call_targets, gas: self.total, persistent_bytes: self.persistent_bytes, new_slots: self.new_slots, settlement }
    }
}

/// Proposer: execute candidates in order, keep the valid ones that fit the limits.
pub fn build_block(pre: &WorldState, ctx: &BlockContext, candidates: Vec<TxEnvelope>) -> (Vec<TxEnvelope>, BlockOutcome) {
    build_block_with(pre, ctx, candidates, true)
}

/// Proposer selection with an additional budget check on each tentative block.
/// The callback sees the exact settled outcome, while accepted candidates keep
/// their unsettled state for the next transaction. Rejected candidates cannot
/// advance nonces, spend balances or contribute accesses to the final block.
pub fn build_block_filtered(
    pre: &WorldState,
    ctx: &BlockContext,
    candidates: Vec<TxEnvelope>,
    mut accept: impl FnMut(&[TxEnvelope], &BlockOutcome) -> bool,
) -> (Vec<TxEnvelope>, BlockOutcome) {
    let mut state = pre.clone();
    state.clear_journal();
    let (mut acc, mut included) = (Acc::default(), Vec::new());
    let mut sched = Scheduler::new(pre, ctx, &candidates, true);
    for (i, tx) in candidates.into_iter().enumerate() {
        let Ok(run) = sched.run(i, &state, ctx, &tx) else { continue };
        if !matches!(acc.total.checked_add(run.gas), Some(t) if t.fits(&ctx.limits)) { continue; }
        if state_growth_enabled(ctx) && (
            acc.new_slots.checked_add(run.new_slots).is_none_or(|n| n > MAX_NEW_SLOTS_PER_BLOCK)
            || acc.persistent_bytes.checked_add(run.persistent_bytes).is_none_or(|n| n > MAX_PERSISTENT_BYTES_PER_BLOCK)
        ) { continue; }

        // Invalidate speculative reads before consuming `run`. A rejected
        // candidate only causes conservative re-execution of later candidates.
        sched.committing(&state, &run.changes);
        let mut next_state = state.clone();
        let mut next_acc = acc.clone();
        if next_acc.apply(&mut next_state, run).is_err() { continue; }
        let preview = next_acc.clone().finish(next_state.clone(), ctx);
        included.push(tx);
        if accept(&included, &preview) {
            state = next_state;
            acc = next_acc;
        } else {
            included.pop();
        }
    }
    (included, acc.finish(state, ctx))
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
        if acc.persistent_bytes + run.persistent_bytes > MAX_PERSISTENT_BYTES_PER_BLOCK && state_growth_enabled(ctx) { continue; }
        sched.committing(&state, &run.changes);
        if acc.apply(&mut state, run).is_err() {
            continue;
        }
        included.push(tx);
    }
    (included, acc.finish(state, ctx))
}

/// FOCIL append check: would `tx` be valid if appended to a block whose
/// post-state is `post` and that already used the supplied limits? Inclusion-list txs
/// for which this holds must not be left out.
pub fn can_append(post: &WorldState, ctx: &BlockContext, used: GasVector, used_new_slots: u64, used_persistent_bytes: u64, tx: &TxEnvelope) -> bool {
    // `post` already includes the end-of-block fee settlement; a tx from an account
    // it credited may only have become affordable then, so it is never required.
    if let Some(f) = &ctx.fees {
        if [f.proposer, PROVER_ESCROW, FEE_COLLECTOR].contains(&tx.header.sender) {
            return false;
        }
    }
    let Ok(run) = run_tx(post, ctx, tx) else { return false };
    matches!(used.checked_add(run.gas), Some(t) if t.fits(&ctx.limits))
        && (!state_growth_enabled(ctx) || (
            used_new_slots.checked_add(run.new_slots).is_some_and(|n| n <= MAX_NEW_SLOTS_PER_BLOCK)
            && used_persistent_bytes.checked_add(run.persistent_bytes).is_some_and(|n| n <= MAX_PERSISTENT_BYTES_PER_BLOCK)
        ))
}

/// Incremental FOCIL archive-budget preview, executing only the appended tx.
/// `post` is the settled state; its preceding settlement is reversed before
/// execution so contracts observe the prospective block's unsettled balances.
/// Fee-recipient senders remain excluded exactly as in `can_append`.
/// `previous.settlement` must be the actual execution settlement. The preview
/// extends its gas, receipts and BAL without replaying the preceding txs.
pub fn append_block_preview(
    post: &WorldState,
    ctx: &BlockContext,
    previous: &BlockOutcome,
    tx: &TxEnvelope,
) -> Option<BlockOutcome> {
    let mut state = post.clone();
    if let Some(f) = &ctx.fees {
        if [f.proposer, PROVER_ESCROW, FEE_COLLECTOR].contains(&tx.header.sender) { return None; }
        // Restore both balances and existence, including aliased recipients.
        // Balance-only reversal would leave a newly created empty escrow and
        // undercharge a subsequent transfer that creates that account.
        for (address, before) in [FEE_COLLECTOR, f.proposer, PROVER_ESCROW]
            .into_iter().zip(previous.settlement.fee_accounts_before)
        {
            state.restore_account(address, before).ok()?;
        }
    }
    let run = run_tx(&state, ctx, tx).ok()?;
    if !matches!(previous.gas.checked_add(run.gas), Some(t) if t.fits(&ctx.limits)) { return None; }
    if state_growth_enabled(ctx) && (
        previous.new_slots.checked_add(run.new_slots).is_none_or(|n| n > MAX_NEW_SLOTS_PER_BLOCK)
        || previous.persistent_bytes.checked_add(run.persistent_bytes).is_none_or(|n| n > MAX_PERSISTENT_BYTES_PER_BLOCK)
    ) { return None; }

    let mut bal = BalBuilder::default();
    for account in &previous.bal.accounts {
        let (present, balance_touched) = ctx.fees.and_then(|f| {
            [FEE_COLLECTOR, f.proposer, PROVER_ESCROW].into_iter()
                .position(|a| a == account.address)
                .map(|i| previous.settlement.fee_bal_before[i])
        }).unwrap_or((true, account.balance_touched));
        if !present { continue; }
        bal.touch_account(account.address);
        for key in &account.reads { bal.read(account.address, *key); }
        for (key, index) in &account.writes { bal.write(account.address, *key, *index); }
        if balance_touched { bal.balance(account.address); }
        if account.nonce_touched { bal.nonce(account.address); }
        if account.code_touched { bal.code(account.address); }
    }
    let mut acc = Acc {
        bal,
        receipts: previous.receipts.clone(),
        call_targets: previous.call_targets.clone(),
        total: previous.gas,
        prove_fees: previous.settlement.prove_fees,
        state_fees: previous.settlement.burned_state,
        new_slots: previous.new_slots,
        persistent_bytes: previous.persistent_bytes,
    };
    acc.apply(&mut state, run).ok()?;
    Some(acc.finish(state, ctx))
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
        if acc.persistent_bytes + run.persistent_bytes > MAX_PERSISTENT_BYTES_PER_BLOCK && state_growth_enabled(ctx) {
            return Err(ExecError::LimitExceeded { index });
        }
        sched.committing(&state, &run.changes);
        acc.apply(&mut state, run)?;
    }
    Ok(acc.finish(state, ctx))
}

#[cfg(test)]
mod tests {
    use super::*;
    use aether_crypto::{P256Signer, Signer};
    use aether_types::FeeVector;

    fn preview_context(proposer: Address) -> BlockContext {
        BlockContext {
            chain_id: 7_781,
            number: 1,
            timestamp: 1,
            beneficiary: FEE_COLLECTOR,
            limits: GasVector { exec: 30_000_000, state: MAX_STATE_UNITS_PER_BLOCK, prove: 200_000_000 },
            fees: Some(FeePolicy {
                base: FeeVector { exec: 3, state: crate::fees::STATE_UNIT_PRICE, prove: 5 },
                proposer,
            }),
        }
    }

    fn preview_tx(signer: &P256Signer, ctx: &BlockContext, nonce: u64, to: Address) -> TxEnvelope {
        let call = EvmCall {
            to: Some(to), value: U256::from(11), input: Bytes::new(), gas_limit: 100_000, delegate: None,
        };
        let mut tx = crate::tx::sign_call_with(
            signer, ctx.chain_id, nonce,
            FeeVector { exec: 5, state: crate::fees::STATE_UNIT_PRICE, prove: 5 }, 2, &call,
        ).unwrap();
        tx.header.gas.state = 10_000;
        let mut signature = signer.sign(&tx.signing_bytes()).unwrap();
        signature.extend_from_slice(&signer.public_key().bytes);
        tx.signature = Bytes::from(signature);
        tx
    }

    fn assert_same_outcome(actual: &BlockOutcome, expected: &BlockOutcome) {
        assert_eq!(actual.state.root(), expected.state.root());
        assert_eq!(actual.bal, expected.bal);
        assert_eq!(actual.receipts, expected.receipts);
        assert_eq!(actual.call_targets, expected.call_targets);
        assert_eq!(actual.gas, expected.gas);
        assert_eq!(actual.new_slots, expected.new_slots);
        assert_eq!(actual.persistent_bytes, expected.persistent_bytes);
        assert_eq!(actual.settlement, expected.settlement);
    }

    fn trace_context() -> BlockContext {
        BlockContext {
            chain_id: 7_777,
            number: 1,
            timestamp: 1,
            beneficiary: Address::repeat_byte(0xbe),
            limits: GasVector { exec: 30_000_000, state: u64::MAX, prove: 200_000_000 },
            fees: None,
        }
    }

    fn trace_tx(signer: &P256Signer, nonce: u64, target: Address) -> TxEnvelope {
        crate::tx::sign_call(signer, trace_context().chain_id, nonce, 1, &EvmCall {
            to: Some(target), value: U256::ZERO, input: Bytes::new(), gas_limit: 200_000, delegate: None,
        }).unwrap()
    }

    fn silent_call_code(target: Address, opcode: u8, ending: &[u8]) -> Bytes {
        let mut code = Vec::new();
        // Empty input/output, plus zero value for CALL. DELEGATECALL has no
        // value operand and runs target's code using the caller's storage.
        for _ in 0..if opcode == 0xf1 { 5 } else { 4 } { code.extend_from_slice(&[0x60, 0x00]); }
        code.push(0x73); // PUSH20 target.
        code.extend_from_slice(target.as_slice());
        code.extend_from_slice(&[0x5a, opcode, 0x50]); // GAS; CALL/DELEGATECALL; POP.
        code.extend_from_slice(ending);
        Bytes::from(code)
    }

    fn trace_fixture() -> (P256Signer, WorldState, Address, Address) {
        let signer = P256Signer::from_seed(&[46; 32]).unwrap();
        let sender = aether_crypto::address_of(&signer.public_key()).unwrap();
        let (router, leaf) = (Address::repeat_byte(0x50), Address::repeat_byte(0x51));
        let mut pre = WorldState::default();
        pre.set_balance(sender, U256::from(10u128.pow(21))).unwrap();
        pre.set_code(router, silent_call_code(leaf, 0xf1, &[0x00])).unwrap();
        pre.set_code(leaf, Bytes::from_static(&[0x00])).unwrap();
        (signer, pre, router, leaf)
    }

    #[test]
    fn silent_internal_calls_are_captured_in_parallel_and_sequential_outcomes() {
        let (signer, pre, router, leaf) = trace_fixture();
        // Four candidates enable speculation; subsequent nonces require
        // replay on the current state, retaining the trace of that execution.
        let txs: Vec<_> = (0..4).map(|nonce| trace_tx(&signer, nonce, router)).collect();
        let ctx = trace_context();
        let parallel = execute_block(&pre, &ctx, &txs).unwrap();
        assert_same_outcome(&parallel, &execute_block_sequential(&pre, &ctx, &txs).unwrap());
        assert_eq!(parallel.call_targets.len(), parallel.receipts.len());
        for (receipt, targets) in parallel.receipts.iter().zip(&parallel.call_targets) {
            assert!(receipt.success);
            assert!(receipt.events.is_empty());
            assert_eq!(receipt.logs, 0);
            assert_eq!(targets, &CallTargets { addresses: vec![router, leaf], complete: true });
        }
        assert_same_outcome(&parallel, &build_block(&pre, &ctx, txs.clone()).1);
        assert_same_outcome(&parallel, &build_block_sequential(&pre, &ctx, txs).1);
        assert_eq!(parallel.clone().call_targets, parallel.call_targets);
    }

    #[test]
    fn delegatecall_captures_the_storage_target_and_executed_code_address() {
        let (signer, mut pre, router, leaf) = trace_fixture();
        pre.set_code(router, silent_call_code(leaf, 0xf4, &[0x00])).unwrap();
        let out = execute_block(&pre, &trace_context(), &[trace_tx(&signer, 0, router)]).unwrap();
        assert!(out.receipts[0].success);
        assert_eq!(out.call_targets[0], CallTargets { addresses: vec![router, leaf], complete: true });
    }

    #[test]
    fn delegated_owner_batches_capture_actual_internal_recipients() {
        let (signer, mut pre, router, leaf) = trace_fixture();
        let sender = aether_crypto::address_of(&signer.public_key()).unwrap();
        pre.set_code(crate::AETHER_ACCOUNT, crate::aether_account_code()).unwrap();
        let tx = crate::tx::sign_call(&signer, trace_context().chain_id, 0, 1, &EvmCall {
            to: Some(sender), value: U256::ZERO,
            input: crate::encode_execute(&[(router, U256::ZERO, Bytes::new()), (leaf, U256::ZERO, Bytes::new())]),
            gas_limit: 300_000, delegate: Some(crate::AETHER_ACCOUNT),
        }).unwrap();
        let out = execute_block(&pre, &trace_context(), &[tx]).unwrap();
        assert!(out.receipts[0].success);
        let mut expected = vec![sender, router, leaf];
        expected.sort_unstable();
        assert_eq!(out.call_targets[0], CallTargets { addresses: expected, complete: true });
    }

    #[test]
    fn reverting_and_halting_top_level_transactions_expose_no_targets() {
        let (signer, mut pre, router, leaf) = trace_fixture();
        for ending in [&[0x60, 0x00, 0x60, 0x00, 0xfd][..], &[0xfe][..]] {
            // The leaf runs successfully before the outer frame fails.
            pre.set_code(router, silent_call_code(leaf, 0xf1, ending)).unwrap();
            let out = execute_block(&pre, &trace_context(), &[trace_tx(&signer, 0, router)]).unwrap();
            assert!(!out.receipts[0].success);
            assert_eq!(out.call_targets[0], CallTargets::default());
        }
    }

    #[test]
    fn target_capture_does_not_change_evm_results_state_or_prove_steps() {
        let (signer, pre, router, leaf) = trace_fixture();
        let ctx = trace_context();
        let sender = aether_crypto::address_of(&signer.public_key()).unwrap();
        let tx_env = TxEnv::builder().caller(sender).nonce(0).chain_id(Some(ctx.chain_id))
            .gas_limit(200_000).gas_price(1).kind(TxKind::Call(router)).build().unwrap();
        let make_context = || Context::mainnet().with_db(WrapDatabaseRef(&pre))
            .modify_cfg_chained(|c| c.chain_id = ctx.chain_id)
            .modify_block_chained(|b| {
                b.number = U256::from(ctx.number);
                b.timestamp = U256::from(ctx.timestamp);
                b.beneficiary = ctx.beneficiary;
                b.basefee = 0;
                b.gas_limit = ctx.limits.exec;
            });
        let mut traced = make_context().build_mainnet_with_inspector(ProveGasMeter::default());
        let mut plain = make_context().build_mainnet_with_inspector(revm::inspector::NoOpInspector);
        let observed = traced.inspect_tx(tx_env.clone()).unwrap();
        let unobserved = plain.inspect_tx(tx_env).unwrap();
        assert_eq!(observed.result, unobserved.result);
        assert_eq!(observed.state, unobserved.state);
        assert_eq!(traced.inspector.steps, 11);
        assert_eq!(traced.inspector.call_targets.addresses, vec![router, leaf]);
        let run = run_tx(&pre, &ctx, &trace_tx(&signer, 0, router)).unwrap();
        assert_eq!(run.receipt.prove_gas, traced.inspector.steps);
        assert_eq!(run.receipt.gas_used, observed.result.gas().tx_gas_used());
        assert_eq!(run.changes, observed.state);
        assert_eq!(receipt_persistent_bytes(&run.receipt), RECEIPT_BASE_BYTES);
    }

    #[test]
    fn unique_call_target_capture_is_sorted_bounded_and_repeats_do_not_truncate() {
        let address = |n: usize| Address::from_word(B256::from(U256::from(n).to_be_bytes::<32>()));
        let mut targets = CallTargets::default();
        for n in (0..MAX_CALL_TARGETS).rev() { targets.record(address(n)); }
        assert!(targets.complete);
        assert!(targets.addresses.windows(2).all(|pair| pair[0] < pair[1]));
        for _ in 0..100_000 { targets.record(address(17)); }
        assert!(targets.complete, "repeated calls do not use extra capture capacity");
        for n in MAX_CALL_TARGETS..100_000 { targets.record(address(n)); }
        assert!(!targets.complete);
        assert_eq!(targets.addresses.len(), MAX_CALL_TARGETS);
        assert!(targets.addresses.capacity() <= MAX_CALL_TARGETS);
        assert_eq!(targets.addresses.first(), Some(&address(0)));
        assert_eq!(targets.addresses.last(), Some(&address(MAX_CALL_TARGETS - 1)));
    }

    #[test]
    fn filtered_previews_match_execution_and_rejection_preserves_nonce() {
        let signer = P256Signer::from_seed(&[42; 32]).unwrap();
        let sender = aether_crypto::address_of(&signer.public_key()).unwrap();
        let proposer = Address::repeat_byte(0xbe);
        let receiver = Address::repeat_byte(0x44);
        let writer = Address::repeat_byte(0x77);
        let ctx = preview_context(proposer);
        let mut pre = WorldState::default();
        pre.set_balance(sender, U256::from(10u128.pow(21))).unwrap();
        // Rejected candidate would add a contract account and a storage key.
        pre.set_code(writer, Bytes::from_static(&[0x60, 0x01, 0x60, 0x00, 0x55, 0x00])).unwrap();
        let first = preview_tx(&signer, &ctx, 0, receiver);
        let rejected = preview_tx(&signer, &ctx, 1, writer);
        let replacement = preview_tx(&signer, &ctx, 1, receiver);
        let last = preview_tx(&signer, &ctx, 2, receiver);
        let bal_cap = execute_block(&pre, &ctx, std::slice::from_ref(&first)).unwrap().bal.to_canonical_bytes().len();
        let candidates = vec![first.clone(), rejected, replacement.clone(), last.clone()];
        let mut previews = 0;
        let (included, outcome) = build_block_filtered(&pre, &ctx, candidates, |txs, preview| {
            previews += 1;
            assert_same_outcome(preview, &execute_block(&pre, &ctx, txs).unwrap());
            preview.bal.to_canonical_bytes().len() <= bal_cap
        });
        assert_eq!(previews, 4);
        assert_eq!(included, vec![first, replacement, last]);
        assert_eq!(outcome.state.nonce(&sender), 3);
        assert_same_outcome(&outcome, &execute_block(&pre, &ctx, &included).unwrap());
        assert_same_outcome(&outcome, &build_block(&pre, &ctx, included).1);
    }

    #[test]
    fn append_preview_matches_full_block_with_fee_balance_reads_and_aliases() {
        let signer = P256Signer::from_seed(&[43; 32]).unwrap();
        let sender = aether_crypto::address_of(&signer.public_key()).unwrap();
        let reader = Address::repeat_byte(0x78);
        for proposer in [Address::repeat_byte(0xbe), PROVER_ESCROW, FEE_COLLECTOR] {
            let ctx = preview_context(proposer);
            let mut pre = WorldState::default();
            pre.set_balance(sender, U256::from(10u128.pow(21))).unwrap();
            pre.set_balance(proposer, U256::from(17)).unwrap();
            // Store the proposer's BALANCE; observing settled credits here
            // instead of the in-block balance would produce a different root.
            let mut code = vec![0x73];
            code.extend_from_slice(proposer.as_slice());
            code.extend_from_slice(&[0x31, 0x60, 0x00, 0x55, 0x00]);
            pre.set_code(reader, Bytes::from(code)).unwrap();
            // A direct payment to the collector is part of the settlement's
            // actual tips, independently of receipt-derived priority fees.
            let first = preview_tx(&signer, &ctx, 0, FEE_COLLECTOR);
            let second = preview_tx(&signer, &ctx, 1, reader);
            let previous = execute_block(&pre, &ctx, std::slice::from_ref(&first)).unwrap();
            let preview = append_block_preview(&previous.state, &ctx, &previous, &second).unwrap();
            assert_same_outcome(&preview, &execute_block(&pre, &ctx, &[first.clone(), second]).unwrap());
            let escrow_payment = preview_tx(&signer, &ctx, 1, PROVER_ESCROW);
            let preview = append_block_preview(&previous.state, &ctx, &previous, &escrow_payment).unwrap();
            assert_same_outcome(&preview, &execute_block(&pre, &ctx, &[first, escrow_payment]).unwrap());
        }
    }

    #[test]
    fn append_preview_removes_settlement_only_bal_touches_when_tips_are_drained() {
        let signer = P256Signer::from_seed(&[45; 32]).unwrap();
        let sender = aether_crypto::address_of(&signer.public_key()).unwrap();
        let proposer = Address::repeat_byte(0xbe);
        let receiver = Address::repeat_byte(0x46);
        let drain_receiver = Address::repeat_byte(0x47);
        let ctx = preview_context(proposer);
        let mut pre = WorldState::default();
        pre.set_balance(sender, U256::from(10u128.pow(21))).unwrap();
        let mut drain = vec![0x73];
        drain.extend_from_slice(drain_receiver.as_slice());
        drain.push(0xff); // SELFDESTRUCT transfers the collector's whole balance.
        pre.set_code(FEE_COLLECTOR, Bytes::from(drain)).unwrap();
        let first = preview_tx(&signer, &ctx, 0, receiver);
        let previous = execute_block(&pre, &ctx, std::slice::from_ref(&first)).unwrap();
        assert!(previous.bal.accounts.iter().any(|a| a.address == proposer));
        assert_eq!(previous.settlement.fee_bal_before[1], (false, false));

        let mut appended = preview_tx(&signer, &ctx, 1, FEE_COLLECTOR);
        appended.header.max_fee.exec = ctx.fees.unwrap().base.exec;
        appended.header.tip = 0;
        let mut signature = signer.sign(&appended.signing_bytes()).unwrap();
        signature.extend_from_slice(&signer.public_key().bytes);
        appended.signature = Bytes::from(signature);
        let preview = append_block_preview(&previous.state, &ctx, &previous, &appended).unwrap();
        let expected = execute_block(&pre, &ctx, &[first, appended]).unwrap();
        assert!(!expected.bal.accounts.iter().any(|a| a.address == proposer));
        assert!(expected.settlement.tips.is_zero());
        assert_same_outcome(&preview, &expected);
    }

    #[test]
    fn append_preview_enforces_cumulative_limits_and_system_sender_exclusion() {
        let signer = P256Signer::from_seed(&[44; 32]).unwrap();
        let sender = aether_crypto::address_of(&signer.public_key()).unwrap();
        let ctx = preview_context(sender);
        let mut pre = WorldState::default();
        pre.set_balance(sender, U256::from(10u128.pow(21))).unwrap();
        let tx = preview_tx(&signer, &ctx, 0, Address::repeat_byte(0x45));
        let previous = execute_block(&pre, &ctx, &[]).unwrap();
        assert!(append_block_preview(&previous.state, &ctx, &previous, &tx).is_none());

        let ctx = preview_context(Address::repeat_byte(0xbe));
        let previous = execute_block(&pre, &ctx, &[]).unwrap();
        assert!(append_block_preview(&previous.state, &ctx, &previous, &tx).is_some());
        let mut bounded = previous.clone();
        bounded.gas.exec = ctx.limits.exec;
        assert!(append_block_preview(&bounded.state, &ctx, &bounded, &tx).is_none());
        bounded = previous.clone();
        bounded.gas.state = ctx.limits.state;
        assert!(append_block_preview(&bounded.state, &ctx, &bounded, &tx).is_none());
        bounded = previous.clone();
        bounded.new_slots = MAX_NEW_SLOTS_PER_BLOCK + 1;
        assert!(append_block_preview(&bounded.state, &ctx, &bounded, &tx).is_none());
        bounded = previous;
        bounded.persistent_bytes = MAX_PERSISTENT_BYTES_PER_BLOCK;
        assert!(append_block_preview(&bounded.state, &ctx, &bounded, &tx).is_none());
    }

    #[test]
    fn new_nonce_only_sender_is_charged_as_an_account() {
        let sender = Address::repeat_byte(0x12);
        let mut account = revm::state::Account::default();
        account.info.nonce = 1;
        account.mark_touch();
        let mut changes = revm::state::EvmState::default();
        changes.insert(sender, account);
        let (units, slots) = state_growth(&WorldState::default(), &changes, FEE_COLLECTOR, U256::ZERO).unwrap();
        assert_eq!((units, slots), (STATE_ACCOUNT_UNITS, 0));
    }
}
