//! Signed transactions, paid-state accounting, rollback and three execution paths.
use aether_crypto::{P256Signer, Signer};
use aether_execution::{
    block::{receipt_persistent_bytes, tx_persistent_bytes},
    build_block, execute_block, execute_block_sequential, fees,
    receipt::receipt_root,
    recommended_state_budget, sign_call_with, BlockContext, BlockOutcome, EvmCall, FeePolicy,
    Receipt, WorldState, FEE_COLLECTOR,
};
use aether_state::layout::basic_data_key;
pub use aether_types::{Address, Bytes, B256, U256};
use alloy_primitives::hex;
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Write,
    sync::{Mutex, OnceLock},
};

pub const CHAIN: u64 = 7795;
pub const TX_GAS: u64 = 16_777_216;
const PROPOSER: Address = Address::repeat_byte(0xbe);
const START_TIME: u64 = 1_000_000;

fn manifest() -> &'static Value {
    static ARTIFACTS: OnceLock<Value> = OnceLock::new();
    ARTIFACTS.get_or_init(|| {
        serde_json::from_str(include_str!("../../fixtures/artifacts.json")).unwrap()
    })
}

pub fn bytecode(name: &str) -> Bytes {
    let raw = manifest()[name]["bytecode"]
        .as_str()
        .unwrap_or_else(|| panic!("missing fixture {name}"));
    Bytes::from(hex::decode(raw.strip_prefix("0x").unwrap_or(raw)).unwrap())
}
pub fn runtime(name: &str) -> Bytes {
    let raw = manifest()[name]["runtime"]
        .as_str()
        .unwrap_or_else(|| panic!("missing runtime {name}"));
    Bytes::from(hex::decode(raw.strip_prefix("0x").unwrap_or(raw)).unwrap())
}
pub fn artifacts() -> &'static serde_json::Map<String, Value> {
    manifest().as_object().unwrap()
}

fn metric(value: Value) {
    static LOCK: Mutex<()> = Mutex::new(());
    let _guard = LOCK.lock().unwrap();
    if let Ok(path) = std::env::var("CONTRACTS_ONCHAIN_METRICS") {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .unwrap();
        let path = std::path::PathBuf::from(path);
        assert!(
            path.is_absolute() && path.starts_with(root.join("tmp")),
            "metrics must stay under workspace/tmp"
        );
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap();
        writeln!(file, "{value}").unwrap();
    }
}

pub struct Harness {
    pub state: WorldState,
    pub excess: u64,
    pub archive_excess: u64,
    pub ctx: BlockContext,
    labels: BTreeMap<Address, String>,
}

impl Default for Harness {
    fn default() -> Self {
        Self::new()
    }
}
impl Harness {
    pub fn new() -> Self {
        let mut h = Self {
            state: WorldState::default(),
            excess: 0,
            archive_excess: 0,
            ctx: BlockContext {
                chain_id: CHAIN,
                number: 1,
                timestamp: START_TIME,
                beneficiary: FEE_COLLECTOR,
                limits: aether_types::GasVector {
                    exec: 30_000_000,
                    state: fees::MAX_STATE_UNITS_PER_BLOCK,
                    prove: 200_000_000,
                },
                fees: Some(FeePolicy {
                    base: aether_types::FeeVector {
                        exec: 0,
                        state: fees::STATE_UNIT_PRICE,
                        prove: 0,
                    },
                    proposer: PROPOSER,
                }),
            },
            labels: BTreeMap::new(),
        };
        for actor in 0..16 {
            let address = h.addr(actor);
            h.state
                .set_balance(address, U256::from(10u128.pow(27)))
                .unwrap();
            h.state.set_code(address, Bytes::new()).unwrap();
        }
        // Fee accounts start absent; zero-tip EVM execution may prune an
        // empty beneficiary. Only legitimate fee headers are excluded below.
        h.state
            .set_code(
                aether_execution::AETHER_ACCOUNT,
                aether_execution::aether_account_code_v2(),
            )
            .unwrap();
        h.state
            .set_code(
                aether_execution::release_log::ADDRESS,
                aether_execution::release_log::code(),
            )
            .unwrap();
        h.labels.insert(
            aether_execution::AETHER_ACCOUNT,
            "core/EastSeaAccount".into(),
        );
        h.labels.insert(
            aether_execution::release_log::ADDRESS,
            "core/ReleaseLog".into(),
        );
        h.state.clear_journal();
        h
    }
    pub fn signer(&self, actor: u8) -> P256Signer {
        let mut seed = [0x77; 32];
        seed[31] = actor + 1;
        P256Signer::from_seed(&seed).unwrap()
    }
    pub fn addr(&self, actor: u8) -> Address {
        aether_crypto::address_of(&self.signer(actor).public_key()).unwrap()
    }
    pub fn label(&mut self, address: Address, name: &str) {
        self.labels.insert(address, name.to_owned());
    }
    pub fn nonce(&self, actor: u8) -> u64 {
        self.state.nonce(&self.addr(actor))
    }
    pub fn timestamp(&self) -> u64 {
        self.ctx.timestamp
    }
    pub fn number(&self) -> u64 {
        self.ctx.number
    }
    pub fn chain_id(&self) -> u64 {
        self.ctx.chain_id
    }
    pub fn refresh(&mut self) {
        self.ctx.limits.state = fees::state_block_limit(self.excess);
        if let Some(policy) = self.ctx.fees.as_mut() {
            policy.base.state = fees::state_base_fee(self.excess);
        }
    }
    pub fn signed(
        &self,
        actor: u8,
        call: &EvmCall,
        budget: Option<u64>,
    ) -> aether_types::TxEnvelope {
        let signer = self.signer(actor);
        let base = self.ctx.fees.unwrap().base;
        let caps = aether_types::FeeVector {
            state: fees::state_base_fee(fees::MAX_STATE_UNITS_PER_BLOCK),
            ..base
        };
        let mut tx =
            sign_call_with(&signer, self.ctx.chain_id, self.nonce(actor), caps, 0, call).unwrap();
        tx.header.gas.state = budget.unwrap_or_else(|| {
            recommended_state_budget(
                call,
                Some(self.state.balance(&self.addr(actor))),
                caps.state,
            )
        });
        let mut sig = signer.sign(&tx.signing_bytes()).unwrap();
        sig.extend_from_slice(&signer.public_key().bytes);
        tx.signature = Bytes::from(sig);
        tx
    }
    pub fn advance(&mut self, blocks: u64) {
        for _ in 0..blocks {
            self.refresh();
            let out = execute_block(&self.state, &self.ctx, &[]).unwrap();
            assert_eq!(
                out.state.root(),
                self.state.root(),
                "empty block must preserve world state"
            );
            assert_eq!(receipt_root(&out.receipts), B256::ZERO);
            self.excess = fees::next_state_excess(self.excess, 0);
            self.archive_excess = fees::next_archive_excess(self.archive_excess, 0);
            self.ctx.number += 1;
            self.ctx.timestamp += 1;
        }
        self.refresh();
    }
    pub fn at(&mut self, timestamp: u64) {
        assert!(timestamp >= self.timestamp(), "time cannot go backward");
        // Run each needed refill height while debt exists; empty debt is a fixed
        // point so the remaining empty intervals can be skipped without simulation.
        let blocks = timestamp - self.timestamp();
        let active = self
            .excess
            .div_ceil(fees::STATE_UNITS_PER_BLOCK)
            .max(
                self.archive_excess
                    .div_ceil(fees::ENCODED_PAYLOAD_BYTES_PER_BLOCK),
            )
            .min(blocks);
        self.advance(active);
        self.ctx.number += blocks - active;
        self.ctx.timestamp = timestamp;
    }
    pub fn view(&self, actor: u8, to: Address, input: Vec<u8>) -> Bytes {
        let result = aether_execution::call(
            &self.state,
            &self.ctx,
            self.addr(actor),
            Some(to),
            input.into(),
            U256::ZERO,
            TX_GAS,
        )
        .unwrap();
        assert!(result.success, "view reverted: {:?}", result.output);
        result.output
    }
    pub fn deploy(&mut self, name: &str, args: Vec<u8>) -> Address {
        self.deploy_as(0, name, args)
    }
    pub fn deploy_as(&mut self, actor: u8, name: &str, args: Vec<u8>) -> Address {
        let mut input = bytecode(name).to_vec();
        input.extend(args);
        let call = EvmCall {
            to: None,
            value: U256::ZERO,
            input: input.into(),
            gas_limit: TX_GAS,
            delegate: None,
        };
        self.refresh();
        let small = self.signed(actor, &call, Some(0));
        let before = self.state.root();
        assert!(
            execute_block(&self.state, &self.ctx, std::slice::from_ref(&small)).is_err(),
            "{name}: zero state budget admitted"
        );
        let (included, refused) = build_block(&self.state, &self.ctx, vec![small]);
        assert!(included.is_empty());
        assert_eq!(refused.state.root(), before);
        metric(
            json!({"contract":name,"case":"deploy/undersized-budget","status":"pass","included":false}),
        );
        let receipt = self.run(
            actor,
            call,
            "deploy/wallet-recommended-budget",
            true,
            Some(name),
        );
        let address = receipt.contract_address.unwrap();
        assert!(
            !self.state.code(&address).is_empty(),
            "{name}: constructor returned no runtime"
        );
        self.labels.insert(address, name.into());
        address
    }
    pub fn deploy_revert(&mut self, name: &str, args: Vec<u8>, label: &str) -> Receipt {
        let mut input = bytecode(name).to_vec();
        input.extend(args);
        self.run(
            0,
            EvmCall {
                to: None,
                value: U256::ZERO,
                input: input.into(),
                gas_limit: TX_GAS,
                delegate: None,
            },
            label,
            false,
            Some(name),
        )
    }
    pub fn ok(
        &mut self,
        actor: u8,
        to: Address,
        input: Vec<u8>,
        value: U256,
        label: &str,
    ) -> Receipt {
        self.transact(
            actor,
            EvmCall {
                to: Some(to),
                value,
                input: input.into(),
                gas_limit: TX_GAS,
                delegate: None,
            },
            label,
            true,
        )
    }
    pub fn revert(
        &mut self,
        actor: u8,
        to: Address,
        input: Vec<u8>,
        value: U256,
        label: &str,
    ) -> Receipt {
        self.transact(
            actor,
            EvmCall {
                to: Some(to),
                value,
                input: input.into(),
                gas_limit: TX_GAS,
                delegate: None,
            },
            label,
            false,
        )
    }
    pub fn transact(
        &mut self,
        actor: u8,
        call: EvmCall,
        label: &str,
        expected_success: bool,
    ) -> Receipt {
        self.run(actor, call, label, expected_success, None)
    }
    fn run(
        &mut self,
        actor: u8,
        call: EvmCall,
        label: &str,
        expected: bool,
        deployment: Option<&str>,
    ) -> Receipt {
        self.refresh();
        let mut tx = self.signed(actor, &call, None);
        // Preview with a full bucket to determine whether a valid transaction
        // must wait. This uses exactly the executor, not an EVM-only estimate.
        let mut preview = self.ctx.clone();
        preview.limits.state = fees::MAX_STATE_UNITS_PER_BLOCK;
        let cost = aether_execution::check_admission_cost(&self.state, &preview, &tx)
            .unwrap_or_else(|e| panic!("{label}: admission {e:?}"));
        if cost.gas.state > self.ctx.limits.state {
            self.advance(
                (cost.gas.state - self.ctx.limits.state).div_ceil(fees::STATE_UNITS_PER_BLOCK),
            );
            tx = self.signed(actor, &call, None);
        }
        let before = self.state.clone();
        let (included, proposed) = build_block(&before, &self.ctx, vec![tx.clone()]);
        assert_eq!(included.len(), 1, "{label}: proposer refused");
        let validated = execute_block(&before, &self.ctx, &included).unwrap();
        let sequential = execute_block_sequential(&before, &self.ctx, &included).unwrap();
        for replay in [&validated, &sequential] {
            assert_eq!(
                proposed.state.root(),
                replay.state.root(),
                "{label}: state replay"
            );
            assert_eq!(
                receipt_root(&proposed.receipts),
                receipt_root(&replay.receipts),
                "{label}: receipt replay"
            );
            assert_eq!(proposed.receipts, replay.receipts);
            assert_eq!(proposed.gas, replay.gas);
            assert_eq!(proposed.persistent_bytes, replay.persistent_bytes);
        }
        let receipt = &validated.receipts[0];
        assert_eq!(
            receipt.success, expected,
            "{label}: unexpected execution result {receipt:?}"
        );
        let base = self.ctx.fees.unwrap().base;
        assert_eq!(
            receipt.state_fee,
            U256::from(receipt.state_gas) * U256::from(base.state)
        );
        assert_eq!(validated.settlement.burned_state, receipt.state_fee);
        assert_eq!(
            validated.persistent_bytes,
            tx_persistent_bytes(&tx) + receipt_persistent_bytes(receipt)
        );
        if !expected && call.delegate.is_none() {
            self.assert_rollback(&before, &validated, actor, receipt);
        }
        let delegated_label = call.to.and_then(|a| {
            let code = validated.state.code(&a);
            if code.len() == 23 && code[..3] == [0xef, 0x01, 0x00] {
                self.labels.get(&Address::from_slice(&code[3..])).cloned()
            } else {
                None
            }
        });
        let contract = deployment
            .map(str::to_owned)
            .or_else(|| call.to.and_then(|a| self.labels.get(&a).cloned()))
            .or(delegated_label)
            .unwrap_or_else(|| "system/account".into());
        metric(
            json!({"contract":contract,"case":label,"status":"pass","success":receipt.success,
            "exec_gas":receipt.gas_used,"prove_gas":receipt.prove_gas,"state_units":receipt.state_gas,
            "persisted_bytes":validated.persistent_bytes,"fee_wei":(receipt.state_fee+U256::from(receipt.gas_used)*U256::from(base.exec)+U256::from(receipt.prove_gas)*U256::from(base.prove)).to_string(),
            "state_fee_wei":receipt.state_fee.to_string(),
            "floor_fee_wei":(U256::from(receipt.state_gas)*U256::from(fees::STATE_UNIT_PRICE)).to_string(),
            "selector":hex::encode(call.input.get(..4).unwrap_or(&[])),"new_slots":validated.new_slots,"signed_budget":tx.header.gas.state,"height":self.number()}),
        );
        let result = receipt.clone();
        self.excess = fees::next_state_excess(self.excess, validated.gas.state);
        // Executor-only archive accounting tracks signed tx bytes. Full payload
        // encoding and archive certification are covered by node/state_budget.
        self.archive_excess =
            fees::next_archive_excess(self.archive_excess, tx_persistent_bytes(&tx));
        self.state = validated.state;
        self.state.clear_journal();
        self.ctx.number += 1;
        self.ctx.timestamp += 1;
        self.refresh();
        result
    }
    fn assert_rollback(
        &self,
        before: &WorldState,
        out: &BlockOutcome,
        actor: u8,
        receipt: &Receipt,
    ) {
        let sender = self.addr(actor);
        let base = self.ctx.fees.unwrap().base;
        let charge = receipt.state_fee
            + U256::from(receipt.gas_used) * U256::from(base.exec)
            + U256::from(receipt.prove_gas) * U256::from(base.prove);
        assert_eq!(
            out.state.balance(&sender),
            before.balance(&sender) - charge,
            "revert fee debit"
        );
        assert_eq!(
            out.state.nonce(&sender),
            before.nonce(&sender) + 1,
            "reverted nonce consumed"
        );
        assert_eq!(out.new_slots, 0);
        assert!(receipt.events.is_empty());
        assert_eq!(
            receipt.state_gas,
            out.persistent_bytes
                .div_ceil(fees::RECEIPT_BYTES_PER_STATE_UNIT),
            "only archive state charged on revert"
        );
        let excluded: BTreeSet<_> = [
            sender,
            FEE_COLLECTOR,
            PROPOSER,
            aether_execution::PROVER_ESCROW,
        ]
        .iter()
        .map(|address| basic_data_key(before.repo().hasher(), address))
        .collect();
        let rows = |state: &WorldState| {
            state
                .repo()
                .entries()
                .filter(|(key, _)| !excluded.contains(key))
                .collect::<BTreeMap<_, _>>()
        };
        assert_eq!(
            rows(before),
            rows(&out.state),
            "revert changed contract/account/storage/code outside fee headers"
        );
        assert_eq!(before.codes(), out.state.codes(), "revert persisted code");
    }
}
