//! Shared result recorder (EastSea native plan, step 2 / deliverable A0).
//!
//! A *workflow* is the sequence of transactions one user task needs. Every
//! transaction the harness includes while a workflow is open becomes a
//! [`StepRecord`], measured from the executor's own outcome: exact state
//! units split into new accounts, newly occupied slots, persisted code bytes
//! and archive units (the split is asserted to reproduce the executor's
//! total), signed-envelope, receipt and BAL bytes, exec and prove gas, the fee
//! actually paid, the floor fee and congestion fees. [`WorkflowRecord`] adds
//! user signature counts, failures with their fee, and the B5 binding limit.
//! JSON output follows `crates/contracts-onchain/schema/workflow-record.v1.schema.json`.
use aether_execution::{
    block::{receipt_persistent_bytes, tx_persistent_bytes},
    fees,
    receipt::{receipt_canonical_bytes, receipt_proof, receipt_root, verify_receipt},
    BlockContext, BlockOutcome, WorldState, FEE_COLLECTOR, PROVER_ESCROW,
};
use aether_types::{Address, Canonical, TxEnvelope, B256};
use alloy_primitives::{keccak256, KECCAK256_EMPTY};
use serde_json::{json, Value};

/// Schema identifier written into every record.
pub const SCHEMA: &str = "eastsea.workflow-record/v1";
/// Path of the schema file, relative to the repository root.
pub const SCHEMA_PATH: &str = "crates/contracts-onchain/schema/workflow-record.v1.schema.json";
/// Node block transaction cap (`aether_node::chain::MAX_TXS_PER_BLOCK`).
pub const MAX_TXS_PER_BLOCK: u64 = 2_000;
/// Nominal one-second heights per day; refill follows finalized heights.
pub const HEIGHTS_PER_DAY: u64 = 86_400;
/// State-debt levels for the congestion fee cases (PRIMITIVES: ~2.72x/7.39x/54.46x floor).
pub const CONGESTION_DEBTS: [u64; 3] = [62_500, 75_000, 99_999];
/// Shares of the shared refill a workflow is assumed to get.
pub const REFILL_SHARES_PERCENT: [u64; 3] = [10, 50, 100];
/// The phase name excluded from the warm (repeat-use) totals.
pub const SETUP_PHASE: &str = "setup";

/// A code change persisted by one transaction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodeChange {
    pub address: Address,
    pub code_hash: B256,
    pub bytes: u64,
    /// For an EIP-7702 designator, the delegate and the delegate's code hash.
    pub delegate: Option<(Address, B256)>,
}

/// One included transaction, measured exactly.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StepRecord {
    pub label: String,
    pub phase: String,
    pub actor: u8,
    pub sender: Address,
    /// The sender is one of the workflow's users (otherwise a relayer).
    pub by_user: bool,
    pub expected_success: bool,
    pub success: bool,
    pub height: u64,
    /// Empty heights the harness waited because the B5 bucket was short.
    pub waited_heights: u64,
    pub tx_hash: B256,
    pub exec_gas: u64,
    pub prove_gas: u64,
    pub state_units: u64,
    pub new_accounts: u64,
    pub new_slots: u64,
    pub code_bytes: u64,
    pub archive_units: u64,
    pub envelope_bytes: u64,
    pub receipt_metered_bytes: u64,
    pub receipt_canonical_bytes: u64,
    pub bal_bytes: u64,
    pub output_bytes: u64,
    pub events: u64,
    pub event_topics: u64,
    pub event_data_bytes: u64,
    pub signed_state_budget: u64,
    pub state_price_wei: u128,
    pub fee_paid_wei: u128,
    pub fee_floor_wei: u128,
    pub fee_congestion_wei: Vec<(u64, u128)>,
    pub receipt_root: B256,
    pub receipt_proof_verified: bool,
    pub created: Option<Address>,
    pub code_changes: Vec<CodeChange>,
}

/// A typed-message (off-chain) signature the user produced for the workflow.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypedSignature {
    pub actor: u8,
    pub label: String,
    pub phase: String,
}

/// An open or finished workflow.
#[derive(Clone, Debug, Default)]
pub struct WorkflowRecord {
    pub name: String,
    pub notes: String,
    pub users: Vec<u8>,
    pub phase: String,
    pub steps: Vec<StepRecord>,
    pub typed_signatures: Vec<TypedSignature>,
    pub environment: Value,
}

fn ceil_div(a: u64, b: u64) -> u64 {
    a.div_ceil(b.max(1))
}

/// Measure one included single-transaction block. Panics if the independent
/// unit decomposition disagrees with the executor's state units.
#[allow(clippy::too_many_arguments)]
pub(crate) fn measure(
    before: &WorldState,
    out: &BlockOutcome,
    tx: &TxEnvelope,
    ctx: &BlockContext,
    label: &str,
    actor: u8,
    expected_success: bool,
    waited_heights: u64,
) -> StepRecord {
    let receipt = &out.receipts[0];
    let policy = ctx.fees.expect("the recorder needs a fee policy");
    let excluded = [FEE_COLLECTOR, PROVER_ESCROW, policy.proposer];
    let mut new_accounts = 0;
    let mut code_bytes = 0;
    let mut code_changes = Vec::new();
    for access in &out.bal.accounts {
        let a = access.address;
        if excluded.contains(&a) {
            continue;
        }
        if before.account(&a).is_none() && out.state.account(&a).is_some() {
            new_accounts += 1;
        }
        let hash = out.state.code_hash(&a);
        if hash != KECCAK256_EMPTY && hash != before.code_hash(&a) {
            let code = out.state.code(&a);
            code_bytes += code.len() as u64;
            let delegate = (code.len() == 23 && code[..3] == [0xef, 0x01, 0x00]).then(|| {
                let target = Address::from_slice(&code[3..]);
                (target, out.state.code_hash(&target))
            });
            code_changes.push(CodeChange { address: a, code_hash: hash, bytes: code.len() as u64, delegate });
        }
    }
    let envelope_bytes = tx_persistent_bytes(tx);
    let receipt_metered_bytes = receipt_persistent_bytes(receipt);
    let archive_units = ceil_div(envelope_bytes + receipt_metered_bytes, fees::RECEIPT_BYTES_PER_STATE_UNIT);
    let decomposed = fees::STATE_ACCOUNT_UNITS * new_accounts
        + fees::STATE_SLOT_UNITS * out.new_slots
        + code_bytes
        + archive_units;
    assert_eq!(
        decomposed, receipt.state_gas,
        "{label}: accounts {new_accounts}, slots {}, code {code_bytes}, archive {archive_units} do not reproduce the executor's state units",
        out.new_slots
    );
    let exec_price = policy.base.exec;
    let prove_price = policy.base.prove;
    let other = u128::from(receipt.gas_used) * exec_price + u128::from(receipt.prove_gas) * prove_price;
    let fee_at = |price: u128| u128::from(receipt.state_gas) * price + other;
    let fee_paid_wei = u128::try_from(receipt.state_fee).expect("fee fits u128") + other;
    let root = receipt_root(&out.receipts);
    let proof = receipt_proof(&out.receipts, 0).expect("single receipt proof");
    StepRecord {
        label: label.to_owned(),
        phase: String::new(),
        actor,
        sender: tx.header.sender,
        by_user: false,
        expected_success,
        success: receipt.success,
        height: ctx.number,
        waited_heights,
        tx_hash: receipt.tx_hash,
        exec_gas: receipt.gas_used,
        prove_gas: receipt.prove_gas,
        state_units: receipt.state_gas,
        new_accounts,
        new_slots: out.new_slots,
        code_bytes,
        archive_units,
        envelope_bytes,
        receipt_metered_bytes,
        receipt_canonical_bytes: receipt_canonical_bytes(receipt).len() as u64,
        bal_bytes: out.bal.to_canonical_bytes().len() as u64,
        output_bytes: receipt.output.len() as u64,
        events: receipt.events.len() as u64,
        event_topics: receipt.events.iter().map(|e| e.topics.len() as u64).sum(),
        event_data_bytes: receipt.events.iter().map(|e| e.data.len() as u64).sum(),
        signed_state_budget: tx.header.gas.state,
        state_price_wei: policy.base.state,
        fee_paid_wei,
        fee_floor_wei: fee_at(fees::STATE_UNIT_PRICE),
        fee_congestion_wei: CONGESTION_DEBTS.iter().map(|d| (*d, fee_at(fees::state_base_fee(*d)))).collect(),
        receipt_root: root,
        receipt_proof_verified: verify_receipt(root, 0, receipt, &proof),
        created: receipt.contract_address,
        code_changes,
    }
}

/// Sums over a set of steps.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Totals {
    pub transactions: u64,
    pub failed: u64,
    pub exec_gas: u64,
    pub prove_gas: u64,
    pub state_units: u64,
    pub new_accounts: u64,
    pub new_slots: u64,
    pub code_bytes: u64,
    pub archive_units: u64,
    pub envelope_bytes: u64,
    pub receipt_metered_bytes: u64,
    pub bal_bytes: u64,
    pub fee_paid_wei: u128,
    pub fee_floor_wei: u128,
    pub fee_congestion_wei: [u128; 3],
    pub user_transactions: u64,
    pub relayer_transactions: u64,
}

impl Totals {
    pub fn of<'a>(steps: impl IntoIterator<Item = &'a StepRecord>) -> Self {
        let mut t = Self::default();
        for s in steps {
            t.transactions += 1;
            t.failed += u64::from(!s.success);
            t.exec_gas += s.exec_gas;
            t.prove_gas += s.prove_gas;
            t.state_units += s.state_units;
            t.new_accounts += s.new_accounts;
            t.new_slots += s.new_slots;
            t.code_bytes += s.code_bytes;
            t.archive_units += s.archive_units;
            t.envelope_bytes += s.envelope_bytes;
            t.receipt_metered_bytes += s.receipt_metered_bytes;
            t.bal_bytes += s.bal_bytes;
            t.fee_paid_wei += s.fee_paid_wei;
            t.fee_floor_wei += s.fee_floor_wei;
            for (i, (_, fee)) in s.fee_congestion_wei.iter().enumerate() {
                t.fee_congestion_wei[i] += fee;
            }
            if s.by_user {
                t.user_transactions += 1;
            } else {
                t.relayer_transactions += 1;
            }
        }
        t
    }

    fn to_json(self) -> Value {
        json!({
            "transactions": self.transactions,
            "failed_transactions": self.failed,
            "exec_gas": self.exec_gas,
            "prove_gas": self.prove_gas,
            "state_units": self.state_units,
            "new_accounts": self.new_accounts,
            "new_slots": self.new_slots,
            "code_bytes": self.code_bytes,
            "archive_units": self.archive_units,
            "envelope_bytes": self.envelope_bytes,
            "receipt_metered_bytes": self.receipt_metered_bytes,
            "bal_bytes": self.bal_bytes,
            "fee_paid_wei": self.fee_paid_wei.to_string(),
            "fee_floor_wei": self.fee_floor_wei.to_string(),
            "fee_congestion_wei": CONGESTION_DEBTS.iter().zip(self.fee_congestion_wei).map(|(d, f)| json!({
                "state_debt": d, "state_price_wei": fees::state_base_fee(*d).to_string(), "fee_wei": f.to_string()
            })).collect::<Vec<_>>(),
            "user_transactions": self.user_transactions,
            "relayer_transactions": self.relayer_transactions,
        })
    }
}

/// One B5 / block-capacity dimension's workflows-per-day ceiling.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ceiling {
    pub share_percent: u64,
    pub per_day: u64,
    pub binding: &'static str,
    pub dimensions: Vec<(&'static str, u64)>,
}

/// Workflows per day at `share` percent of every shared per-height capacity,
/// and the dimension that binds first. Upper bounds, not achieved throughput.
pub fn sustained_ceiling(t: &Totals, ctx: &BlockContext, share_percent: u64) -> Ceiling {
    let per_day = |capacity_per_height: u64, used: u64| -> Option<u64> {
        (used > 0).then(|| {
            let capacity = u128::from(capacity_per_height) * u128::from(HEIGHTS_PER_DAY) * u128::from(share_percent) / 100;
            u64::try_from(capacity / u128::from(used)).unwrap_or(u64::MAX)
        })
    };
    let dims: Vec<(&'static str, u64)> = [
        ("state_refill", per_day(fees::STATE_UNITS_PER_BLOCK, t.state_units)),
        ("encoded_payload_refill", per_day(fees::ENCODED_PAYLOAD_BYTES_PER_BLOCK, t.envelope_bytes)),
        ("exec_gas", per_day(ctx.limits.exec, t.exec_gas)),
        ("prove_gas", per_day(ctx.limits.prove, t.prove_gas)),
        ("new_slots", per_day(fees::MAX_NEW_SLOTS_PER_BLOCK, t.new_slots)),
        ("persistent_bytes", per_day(fees::MAX_PERSISTENT_BYTES_PER_BLOCK, t.envelope_bytes + t.receipt_metered_bytes)),
        ("transactions", per_day(MAX_TXS_PER_BLOCK, t.transactions)),
    ]
    .into_iter()
    .filter_map(|(name, v)| v.map(|v| (name, v)))
    .collect();
    let (binding, per_day) = dims.iter().min_by_key(|(_, v)| *v).copied().unwrap_or(("none", u64::MAX));
    Ceiling { share_percent, per_day, binding, dimensions: dims }
}

fn throughput(t: &Totals, max_step_units: u64, ctx: &BlockContext) -> Value {
    let burst = fees::MAX_STATE_UNITS_PER_BLOCK / t.state_units.max(1);
    json!({
        "units_per_workflow": t.state_units,
        "max_transaction_units": max_step_units,
        "fits_in_one_transaction_budget": max_step_units <= fees::MAX_STATE_UNITS_PER_BLOCK,
        "burst": {
            "bucket_units": fees::MAX_STATE_UNITS_PER_BLOCK,
            "workflows_from_full_bucket": burst,
            "recovery_heights_to_full_bucket": ceil_div(burst * t.state_units, fees::STATE_UNITS_PER_BLOCK),
            "recovery_heights_per_workflow": ceil_div(t.state_units, fees::STATE_UNITS_PER_BLOCK),
        },
        "sustained_per_day": REFILL_SHARES_PERCENT.iter().map(|share| {
            let c = sustained_ceiling(t, ctx, *share);
            json!({
                "refill_share_percent": c.share_percent,
                "workflows_per_day": c.per_day,
                "binding_limit": c.binding,
                "ceilings": c.dimensions.iter().map(|(k, v)| (k.to_string(), json!(v))).collect::<serde_json::Map<_, _>>(),
            })
        }).collect::<Vec<_>>(),
        "heights_per_day": HEIGHTS_PER_DAY,
        "caveat": "upper bounds at nominal 1 s heights for this workflow alone; encoded payload excludes BAL, headers and protocol traffic",
    })
}

impl StepRecord {
    pub fn to_json(&self) -> Value {
        json!({
            "label": self.label,
            "phase": self.phase,
            "actor": self.actor,
            "sender": self.sender.to_string(),
            "signer_role": if self.by_user { "user" } else { "relayer" },
            "expected_success": self.expected_success,
            "success": self.success,
            "height": self.height,
            "waited_heights": self.waited_heights,
            "tx_hash": self.tx_hash.to_string(),
            "exec_gas": self.exec_gas,
            "prove_gas": self.prove_gas,
            "state_units": self.state_units,
            "units": {
                "new_accounts": self.new_accounts,
                "new_slots": self.new_slots,
                "code_bytes": self.code_bytes,
                "archive_units": self.archive_units,
            },
            "bytes": {
                "envelope": self.envelope_bytes,
                "receipt_metered": self.receipt_metered_bytes,
                "receipt_canonical": self.receipt_canonical_bytes,
                "bal": self.bal_bytes,
                "block_payload_lower_bound": self.envelope_bytes + self.bal_bytes,
                "output": self.output_bytes,
                "event_data": self.event_data_bytes,
            },
            "events": self.events,
            "event_topics": self.event_topics,
            "signed_state_budget": self.signed_state_budget,
            "fee": {
                "state_price_wei": self.state_price_wei.to_string(),
                "paid_wei": self.fee_paid_wei.to_string(),
                "floor_wei": self.fee_floor_wei.to_string(),
                "congestion_wei": self.fee_congestion_wei.iter().map(|(d, f)| json!({"state_debt": d, "fee_wei": f.to_string()})).collect::<Vec<_>>(),
            },
            "receipt_evidence": {
                "receipt_root": self.receipt_root.to_string(),
                "index": 0,
                "proof_verified": self.receipt_proof_verified,
            },
            "created": self.created.map(|a| a.to_string()),
            "code_changes": self.code_changes.iter().map(|c| json!({
                "address": c.address.to_string(),
                "code_hash": c.code_hash.to_string(),
                "bytes": c.bytes,
                "delegate": c.delegate.map(|(a, _)| a.to_string()),
                "delegate_code_hash": c.delegate.map(|(_, h)| h.to_string()),
            })).collect::<Vec<_>>(),
        })
    }
}

impl WorkflowRecord {
    pub fn totals(&self) -> Totals {
        Totals::of(&self.steps)
    }
    /// Totals without the setup phase (repeat use).
    pub fn warm_totals(&self) -> Totals {
        Totals::of(self.steps.iter().filter(|s| s.phase != SETUP_PHASE))
    }
    pub fn failures(&self) -> impl Iterator<Item = &StepRecord> {
        self.steps.iter().filter(|s| !s.success)
    }
    pub fn user_transaction_signatures(&self) -> u64 {
        self.steps.iter().filter(|s| s.by_user).count() as u64
    }
    pub fn user_typed_signatures(&self) -> u64 {
        self.typed_signatures.len() as u64
    }
    pub fn step(&self, label: &str) -> &StepRecord {
        self.steps
            .iter()
            .find(|s| s.label == label)
            .unwrap_or_else(|| panic!("no step {label}"))
    }

    pub fn to_json(&self, ctx: &BlockContext) -> Value {
        let cold = self.totals();
        let warm = self.warm_totals();
        let max_units = |setup: bool| {
            self.steps
                .iter()
                .filter(|s| setup || s.phase != SETUP_PHASE)
                .map(|s| s.state_units)
                .max()
                .unwrap_or(0)
        };
        let mut phases: Vec<&str> = Vec::new();
        for s in &self.steps {
            if !phases.contains(&s.phase.as_str()) {
                phases.push(&s.phase);
            }
        }
        let typed = self.user_typed_signatures();
        let warm_typed = self.typed_signatures.iter().filter(|t| t.phase != SETUP_PHASE).count() as u64;
        json!({
            "schema": SCHEMA,
            "workflow": self.name,
            "notes": self.notes,
            "environment": self.environment,
            "users": self.users,
            "signatures": {
                "user_transaction": cold.user_transactions,
                "user_typed_message": typed,
                "user_total": cold.user_transactions + typed,
                "relayer_transaction": cold.relayer_transactions,
                "warm_user_total": warm.user_transactions + warm_typed,
                "typed_messages": self.typed_signatures.iter().map(|t| json!({"actor": t.actor, "label": t.label, "phase": t.phase})).collect::<Vec<_>>(),
            },
            "phases": phases.iter().map(|p| {
                let t = Totals::of(self.steps.iter().filter(|s| s.phase == *p));
                json!({"phase": p, "totals": t.to_json()})
            }).collect::<Vec<_>>(),
            "totals": {"cold": cold.to_json(), "warm": warm.to_json()},
            "failures": self.failures().map(|s| json!({
                "label": s.label,
                "phase": s.phase,
                "expected": !s.expected_success,
                "payer_role": if s.by_user { "user" } else { "relayer" },
                "state_units": s.state_units,
                "exec_gas": s.exec_gas,
                "fee_paid_wei": s.fee_paid_wei.to_string(),
                "fee_floor_wei": s.fee_floor_wei.to_string(),
            })).collect::<Vec<_>>(),
            "b5": {"cold": throughput(&cold, max_units(true), ctx), "warm": throughput(&warm, max_units(false), ctx)},
            "steps": self.steps.iter().map(StepRecord::to_json).collect::<Vec<_>>(),
        })
    }
}

/// Recording environment: chain, fee vector, pinned runtime and fixture hashes.
pub(crate) fn environment(state: &WorldState, ctx: &BlockContext) -> Value {
    let policy = ctx.fees.expect("fee policy");
    let provenance: Value = serde_json::from_str(include_str!("../fixtures/provenance.json")).expect("provenance");
    let account = state.code(&aether_execution::AETHER_ACCOUNT);
    json!({
        "executor": "aether_execution::execute_block (parallel) + execute_block_sequential + build_block replay",
        "chain_id": ctx.chain_id,
        "genesis": "new-genesis paid state (B5 active), harness context",
        "fee_vector": {
            "exec_wei": policy.base.exec.to_string(),
            "prove_wei": policy.base.prove.to_string(),
            "state_floor_wei": fees::STATE_UNIT_PRICE.to_string(),
        },
        "block_limits": {"exec": ctx.limits.exec, "prove": ctx.limits.prove, "state_bucket": fees::MAX_STATE_UNITS_PER_BLOCK},
        "account_runtime_code_hash": keccak256(&account).to_string(),
        "artifacts_sha256": provenance["artifacts_sha256"],
    })
}
