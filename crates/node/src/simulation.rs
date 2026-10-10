//! Private wallet transaction previews over the finalized immutable state.
//! Kept in the node so client previews do not change the proving program.

use aether_execution::{BlockContext, Event, ProveGasMeter, StateError, WorldState};
use aether_types::{Address, Bytes, B256, U256};
use revm::context::result::{ExecutionResult, HaltReason, ResultAndState};
use revm::context::TxEnv;
use revm::database::{CacheDB, WrapDatabaseRef};
use revm::primitives::{TxKind, KECCAK_EMPTY};
use revm::{Context, Database, DatabaseCommit, InspectEvm, MainBuilder, MainContext};

/// Result of a read-only call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CallResult {
    pub success: bool,
    pub output: Bytes,
    pub gas_used: u64,
    pub failure_reason: Option<String>,
    pub native_changes: Vec<NativeBalanceChange>,
    pub token_changes: Vec<TokenBalanceChange>,
    /// Successfully checked token contracts, including unchanged balances.
    pub measured_tokens: Vec<Address>,
    /// False if candidate discovery/read limits leave possible changes unknown.
    pub token_coverage_complete: bool,
    pub events: Vec<Event>,
}

/// A balance change caused by execution, excluding gas fees.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NativeBalanceChange {
    pub address: Address,
    /// Signed decimal wei; using a string preserves all 256 bits.
    pub delta_wei: String,
}

/// The caller's ERC-20 balance difference from balanceOf before/after execution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TokenBalanceChange {
    pub token: Address,
    /// Signed decimal token base units, independent of reported Transfer logs.
    pub delta: String,
}

/// Run a call against `state` without changing it (eth_call): no fees, the
/// caller's current nonce, nothing committed.
pub(crate) fn call(state: &WorldState, ctx: &BlockContext, from: Address, to: Option<Address>, data: Bytes, value: U256, gas: u64) -> Result<CallResult, String> {
    let out = run_call(WrapDatabaseRef(state), ctx, from, to, data, value, gas)?;
    Ok(describe_call(state, &out))
}

/// Wallet preview: execute against the same immutable snapshot as eth_call,
/// then check the caller's token balances against a small in-memory overlay.
/// Only the overlay receives writes; the WorldState and its journal are untouched.
pub(crate) fn simulate(state: &WorldState, ctx: &BlockContext, from: Address, to: Option<Address>, data: Bytes, value: U256, gas: u64) -> Result<CallResult, String> {
    let out = run_call(WrapDatabaseRef(state), ctx, from, to, data, value, gas)?;
    let mut result = describe_call(state, &out);
    result.token_coverage_complete = true;
    if !result.success {
        return Ok(result);
    }
    // Destination first, then event emitters and observed storage writers.
    // Both count and gas are bounded:
    // an arbitrary contract cannot make its preview run unbounded balance reads.
    const MAX_TOKENS: usize = 16;
    const BALANCE_GAS: u64 = 100_000;
    let mut seen = std::collections::BTreeSet::new();
    let storage_writers: std::collections::BTreeSet<_> = out.state.iter()
        .filter(|(_, account)| account.storage.values().any(|slot| slot.is_changed()))
        .map(|(address, _)| *address).collect();
    let mut candidates: Vec<_> = to.into_iter().chain(result.events.iter().map(|event| event.address))
        .chain(storage_writers)
        .filter(|token| seen.insert(*token))
        .filter(|token| state.code_hash(token) != KECCAK_EMPTY || out.state.get(token)
            .is_some_and(|account| (account.info.code_hash != KECCAK_EMPTY && account.info.code_hash != B256::ZERO)
                || account.info.code.as_ref().is_some_and(|code| !code.is_empty())))
        .take(MAX_TOKENS + 1).collect();
    if candidates.len() > MAX_TOKENS {
        result.token_coverage_complete = false;
        candidates.truncate(MAX_TOKENS);
    }
    let mut overlay = CacheDB::new(state);
    overlay.commit(out.state);
    for token in candidates {
        let gas = BALANCE_GAS.min(ctx.limits.exec);
        let before = token_balance(WrapDatabaseRef(state), ctx, token, from, gas);
        let after = token_balance(WrapDatabaseRef(&overlay), ctx, token, from, gas);
        if let (Some(before), Some(after)) = (before, after) {
            result.measured_tokens.push(token);
            if let Some(delta) = balance_delta(before, after) {
                result.token_changes.push(TokenBalanceChange { token, delta });
            }
        } else {
            result.token_coverage_complete = false;
        }
    }
    result.token_changes.sort_by_key(|change| change.token);
    result.measured_tokens.sort();
    Ok(result)
}

fn token_balance(db: impl Database<Error = StateError>, ctx: &BlockContext, token: Address, owner: Address, gas: u64) -> Option<U256> {
    let mut input = vec![0x70, 0xa0, 0x82, 0x31]; // balanceOf(address)
    input.extend_from_slice(&[0; 12]);
    input.extend_from_slice(owner.as_slice());
    let out = run_call(db, ctx, Address::ZERO, Some(token), input.into(), U256::ZERO, gas).ok()?;
    match out.result {
        ExecutionResult::Success { output, .. } if output.data().len() == 32 => Some(U256::from_be_slice(output.data())),
        _ => None,
    }
}

fn run_call(mut db: impl Database<Error = StateError>, ctx: &BlockContext, from: Address, to: Option<Address>, data: Bytes, value: U256, gas: u64) -> Result<ResultAndState, String> {
    let nonce = db.basic(from).map_err(|e| e.to_string())?.map_or(0, |account| account.nonce);
    let tx_env = TxEnv::builder()
        .caller(from)
        .nonce(nonce)
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
        .with_db(db)
        .modify_cfg_chained(|c| c.chain_id = ctx.chain_id)
        .modify_block_chained(|b| {
            b.number = U256::from(ctx.number);
            b.timestamp = U256::from(ctx.timestamp);
            b.beneficiary = ctx.beneficiary;
            b.basefee = 0;
            b.gas_limit = ctx.limits.exec;
        })
        .build_mainnet_with_inspector(ProveGasMeter::default());
    evm.inspect_tx(tx_env).map_err(|e| e.to_string())
}

fn describe_call(state: &WorldState, out: &ResultAndState) -> CallResult {
    let mut result = match &out.result {
        ExecutionResult::Success { gas, output, logs, .. } => CallResult {
            success: true, output: output.data().clone(), gas_used: gas.tx_gas_used(), failure_reason: None,
            native_changes: vec![], token_changes: vec![],
            measured_tokens: vec![], token_coverage_complete: false,
            events: logs.iter().map(|log| Event { address: log.address, topics: log.data.topics().to_vec(), data: log.data.data.clone() }).collect(),
        },
        ExecutionResult::Revert { gas, output, .. } => CallResult {
            success: false, output: output.clone(), gas_used: gas.tx_gas_used(), failure_reason: Some(revert_reason(output)),
            native_changes: vec![], token_changes: vec![], events: vec![],
            measured_tokens: vec![], token_coverage_complete: false,
        },
        ExecutionResult::Halt { gas, reason, .. } => CallResult {
            success: false, output: Bytes::new(), gas_used: gas.tx_gas_used(),
            failure_reason: Some(match reason {
                HaltReason::OutOfGas(_) => "Not enough gas to run this transaction.".into(),
                _ => format!("Contract execution failed: {reason}."),
            }),
            native_changes: vec![], token_changes: vec![], events: vec![],
            measured_tokens: vec![], token_coverage_complete: false,
        },
    };
    if result.success {
        result.native_changes = out.state.iter().filter(|(_, account)| account.is_touched())
            .filter_map(|(address, account)| {
                let after = if account.is_selfdestructed() { U256::ZERO } else { account.info.balance };
                balance_delta(state.balance(address), after).map(|delta_wei| NativeBalanceChange { address: *address, delta_wei })
            }).collect();
        result.native_changes.sort_by_key(|change| change.address);
    }
    result
}

fn balance_delta(before: U256, after: U256) -> Option<String> {
    if before == after { None }
    else if after > before { Some((after - before).to_string()) }
    else { Some(format!("-{}", before - after)) }
}

/// Decode standard Solidity errors with checked offsets/lengths. Unknown or
/// malformed error data remains available in output, never presented as prose.
fn revert_reason(output: &[u8]) -> String {
    if output.starts_with(&[0x08, 0xc3, 0x79, 0xa0]) {
        if let Some(message) = solidity_error_message(output) {
            return format!("Execution reverted: {message}");
        }
    } else if output.starts_with(&[0x4e, 0x48, 0x7b, 0x71]) && output.len() == 36 {
        let code: Option<u64> = U256::from_be_slice(&output[4..]).try_into().ok();
        let reason = match code {
            Some(0x01) => "an assertion failed",
            Some(0x11) => "arithmetic overflow or underflow",
            Some(0x12) => "division by zero",
            Some(0x21) => "an invalid enum value",
            Some(0x22) => "invalid storage data",
            Some(0x31) => "removing an item from an empty array",
            Some(0x32) => "an array index is out of bounds",
            Some(0x41) => "not enough memory",
            Some(0x51) => "an uninitialized function was called",
            _ => "a contract safety check failed",
        };
        return format!("Execution reverted: {reason}.");
    }
    if output.is_empty() { "The contract rejected this transaction.".into() }
    else { "The contract rejected this transaction with an unrecognized error.".into() }
}

fn solidity_error_message(output: &[u8]) -> Option<String> {
    let offset: usize = U256::from_be_slice(output.get(4..36)?).try_into().ok()?;
    if offset < 32 || offset % 32 != 0 { return None; }
    let length_start = 4usize.checked_add(offset)?;
    let message_start = length_start.checked_add(32)?;
    let length: usize = U256::from_be_slice(output.get(length_start..message_start)?).try_into().ok()?;
    let message_end = message_start.checked_add(length)?;
    let message = std::str::from_utf8(output.get(message_start..message_end)?).ok()?;
    let message: String = message.chars().filter(|c| !c.is_control()).take(1024).collect();
    (!message.trim().is_empty()).then_some(message)
}
