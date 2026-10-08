//! The rolling paid-state budget follows the same certified excess on every
//! execution path, including restart and stateless proof replay.

use aether_crypto::{P256Signer, Signer as _};
use aether_execution::{
    build_block, check_admission_cost, execute_block, fees, sign_call_with, tx_hash, BlockContext,
    EvmCall,
};
use aether_node::block::{Block, Context, EPOCH};
use aether_node::chain::{
    build_payload, dev_accounts, dev_seed, Chain, ChainConfig, Executed, Extras,
};
use aether_node::inclusion::violations;
use aether_node::snapshot::Snapshot;
use aether_node::store::Store;
use aether_types::{Address, Bytes, FeeVector, GasVector, TxEnvelope, U256};
use commonware_consensus::types::{Round, View};
use commonware_cryptography::{ed25519, Digestible as _, Signer as _};
use std::path::PathBuf;
use std::sync::Arc;

const CHAIN: u64 = 7_799;

fn config() -> ChainConfig {
    ChainConfig {
        chain_id: CHAIN,
        limits: GasVector {
            exec: 30_000_000,
            state: u64::MAX,
            prove: 200_000_000,
        },
        alloc: dev_accounts(4)
            .into_iter()
            .map(|(_, a)| (a, U256::from(10u128.pow(24))))
            .collect(),
        fees: true,
        registrar: None,
        epoch_blocks: 10,
        min_streak: None,
        draw_epochs: None,
        history_v2: true,
        node_rewards: false,
        protocol: 3,
        committee: vec![],
        reserve: None,
        group: 0,
        max_committee: aether_node::rotation::GROW_UNTIL,
    }
}

fn signer() -> P256Signer {
    P256Signer::from_seed(&dev_seed(1)).unwrap()
}

fn sender() -> Address {
    aether_crypto::address_of(&signer().public_key()).unwrap()
}

fn paid(nonce: u64, call: &EvmCall) -> TxEnvelope {
    let s = signer();
    let mut tx = sign_call_with(
        &s,
        CHAIN,
        nonce,
        FeeVector {
            exec: 1_000_000_000_000_000,
            state: 1_000_000_000_000_000,
            prove: 1_000_000_000_000_000,
        },
        0,
        call,
    )
    .unwrap();
    // A wallet's original burst estimate remains usable after the bucket is
    // depleted: consensus compares the actual units with the current limit.
    tx.header.gas.state = fees::MAX_STATE_UNITS_PER_BLOCK;
    let mut sig = s.sign(&tx.signing_bytes()).unwrap();
    sig.extend_from_slice(&s.public_key().bytes);
    tx.signature = Bytes::from(sig);
    tx
}

fn vault_sized_deployment() -> EvmCall {
    sized_deployment(12_588)
}

fn sized_deployment(size: u16) -> EvmCall {
    let [hi, lo] = size.to_be_bytes();
    let mut init = vec![
        0x61, hi, lo, 0x60, 14, 0x60, 0, 0x39, 0x61, hi, lo, 0x60, 0, 0xf3,
    ];
    init.resize(14 + usize::from(size), 0);
    EvmCall {
        to: None,
        value: U256::ZERO,
        input: init.into(),
        gas_limit: 5_000_000,
        delegate: None,
    }
}

fn with_state_cap(mut tx: TxEnvelope, cap: u128) -> TxEnvelope {
    tx.header.max_fee.state = cap;
    let s = signer();
    let mut sig = s.sign(&tx.signing_bytes()).unwrap();
    sig.extend_from_slice(&s.public_key().bytes);
    tx.signature = Bytes::from(sig);
    tx
}

struct Node {
    chain: Chain,
    last: Block,
    parent: Arc<Executed>,
}

impl Node {
    fn open(dir: &PathBuf) -> Self {
        Self::open_with_config(dir, config())
    }

    fn open_with_config(dir: &PathBuf, cfg: ChainConfig) -> Self {
        let (chain, genesis) =
            Chain::open(cfg, Store::open(&dir.join("state.redb")).unwrap()).unwrap();
        let (_, sharing, _) = aether_light::devnet_threshold(4);
        chain.lock().identity = Some(*sharing.public());
        chain.finalize(&genesis).unwrap();
        let parent = chain.lock().finalized.clone();
        Self {
            chain,
            last: genesis,
            parent,
        }
    }

    fn skeleton(&self) -> Block {
        let height = self.last.height.next();
        let context = Context {
            round: Round::new(EPOCH, View::new(height.get())),
            leader: ed25519::PrivateKey::from_seed(1).public_key(),
            parent: (View::new(height.get() - 1), self.last.digest()),
        };
        Block::new(
            context,
            self.last.digest(),
            height,
            height.get() * 1000,
            bytes::Bytes::new(),
        )
    }

    fn ctx(&self) -> BlockContext {
        Chain::block_context(&self.chain.cfg(), &self.skeleton(), &self.parent)
    }

    fn build(&self, txs: Vec<TxEnvelope>) -> (Block, Arc<Executed>) {
        self.build_with_extras(txs, Extras::default())
    }

    fn build_with_extras(&self, txs: Vec<TxEnvelope>, extras: Extras) -> (Block, Arc<Executed>) {
        let skeleton = self.skeleton();
        let ctx = self.ctx();
        let (pre, _) = self
            .chain
            .pre_state(&self.parent, self.parent.next_protocol(), &[], None, false)
            .unwrap();
        let (payload, _) = build_payload(&self.parent, &pre, &ctx, txs, extras);
        let block = Block::new(
            skeleton.context,
            skeleton.parent,
            skeleton.height,
            skeleton.timestamp,
            payload.to_bytes(),
        );
        let exec = self.chain.execute(&block, &self.parent).unwrap();
        (block, exec)
    }

    fn step(&mut self, txs: Vec<TxEnvelope>) -> Arc<Executed> {
        self.step_with_extras(txs, Extras::default())
    }

    fn step_with_extras(&mut self, txs: Vec<TxEnvelope>, extras: Extras) -> Arc<Executed> {
        let before = self.parent.excess.state;
        let archive_before = self.parent.archive_excess;
        let (block, exec) = self.build_with_extras(txs, extras);
        assert_eq!(
            exec.excess.state,
            fees::next_state_excess(before, exec.gas.state)
        );
        assert_eq!(
            exec.archive_excess,
            fees::next_archive_excess(archive_before, block.data.len() as u64)
        );
        self.chain.finalize(&block).unwrap();
        self.last = block;
        self.parent = exec.clone();
        exec
    }

    fn nonce(&self) -> u64 {
        self.parent.state.nonce(&sender())
    }
}

fn tmp(name: &str) -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dir = root
        .join("tmp")
        .join(format!("state-budget-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn sustained_growth_exhausts_the_burst_and_every_execution_path_obeys_it() {
    let dir = tmp("paths");
    let mut n = Node::open(&dir);
    assert_eq!(n.ctx().limits.state, fees::MAX_STATE_UNITS_PER_BLOCK);
    assert_eq!(n.ctx().fees.unwrap().base.state, fees::STATE_UNIT_PRICE);

    // Deploy a writer through the actual chain; calldata's first word selects
    // the new storage slot. Extra zero calldata exercises paid archive bytes.
    let deploy = EvmCall {
        to: None,
        value: U256::ZERO,
        input: Bytes::from(hex::decode("6007600c60003960076000f360016000355500").unwrap()),
        gas_limit: 1_000_000,
        delegate: None,
    };
    let deployed = n.step(vec![paid(n.nonce(), &deploy)]);
    assert_eq!(deployed.tx_hashes.len(), 1);
    let writer = deployed.receipts[0].contract_address.unwrap();
    // Code consumes state units quickly without exhausting the independent
    // encoded-payload bucket first. Five ordinary-sized deployments fit.
    let nonce = n.nonce();
    let burst = sized_deployment(17_800);
    let out = n.step((0..5).map(|i| paid(nonce + i, &burst)).collect());
    assert_eq!(out.tx_hashes.len(), 5);
    assert!(n.parent.excess.state > fees::STATE_PRICE_FREE_BURST);
    assert!(n.ctx().fees.unwrap().base.state > fees::STATE_UNIT_PRICE);

    // Spend the exact remaining units on an archived call with no new slots.
    // Padding is metered in 32-byte units, so this consumes the entire burst.
    let remaining = n.ctx().limits.state;
    let make = |len: usize| {
        paid(
            n.nonce(),
            &EvmCall {
                to: Some(sender()),
                value: U256::ZERO,
                input: vec![0; len].into(),
                gas_limit: 10_000_000,
                delegate: None,
            },
        )
    };
    let base = check_admission_cost(&n.parent.state, &n.ctx(), &make(0))
        .unwrap()
        .gas
        .state;
    assert!(remaining >= base);
    // The call payload hex-encodes calldata before canonical envelope
    // accounting. Find the exact rounded unit boundary from actual costs.
    let mut full = n.ctx();
    full.limits.state = fees::MAX_STATE_UNITS_PER_BLOCK;
    let mut low = 0;
    let mut high = ((remaining - base) * fees::RECEIPT_BYTES_PER_STATE_UNIT) as usize;
    while low < high {
        let mid = low + (high - low) / 2;
        let units = check_admission_cost(&n.parent.state, &full, &make(mid))
            .unwrap()
            .gas
            .state;
        if units < remaining {
            low = mid + 1;
        } else {
            high = mid;
        }
    }
    let fill = make(low);
    assert_eq!(
        check_admission_cost(&n.parent.state, &full, &fill)
            .unwrap()
            .gas
            .state,
        remaining
    );
    assert_eq!(n.step(vec![fill]).gas.state, remaining);
    let limited = n.ctx();
    assert_eq!(limited.limits.state, fees::STATE_UNITS_PER_BLOCK);

    let tx = paid(
        n.nonce(),
        &EvmCall {
            to: Some(writer),
            value: U256::ZERO,
            input: U256::from(10_000u64).to_be_bytes::<32>().to_vec().into(),
            gas_limit: 100_000,
            delegate: None,
        },
    );
    let pre = &n.parent.state;
    let mut full = limited.clone();
    full.limits.state = fees::MAX_STATE_UNITS_PER_BLOCK;
    let cost = check_admission_cost(pre, &full, &tx).unwrap();
    assert!(cost.gas.state > limited.limits.state);
    assert!(
        n.chain.add_to_mempool(tx.clone()).is_err(),
        "admission refuses actual growth above the remaining bucket"
    );
    assert!(check_admission_cost(pre, &limited, &tx).is_err());
    assert!(
        build_block(pre, &limited, vec![tx.clone()]).0.is_empty(),
        "proposer skips it"
    );
    assert!(
        execute_block(pre, &limited, std::slice::from_ref(&tx)).is_err(),
        "validator refuses it"
    );
    assert!(
        violations(
            std::slice::from_ref(&tx),
            &[],
            false,
            pre,
            &limited,
            GasVector::default(),
            0,
            0
        )
        .is_empty(),
        "FOCIL cannot demand growth outside the bucket"
    );
    assert_eq!(
        violations(
            std::slice::from_ref(&tx),
            &[],
            false,
            pre,
            &full,
            GasVector::default(),
            0,
            0
        ),
        vec![tx_hash(&tx)]
    );
    let mut witness = aether_proving::block::input(
        pre,
        &full,
        std::slice::from_ref(&tx),
        &[],
        Address::repeat_byte(0xaa),
    )
    .unwrap();
    witness.ctx = limited.clone();
    assert!(
        aether_proving::block::execute(&witness).is_err(),
        "stateless prover refuses the same growth"
    );

    // Even while full, an ordinary small call can use its original signed
    // 100,000-unit estimate; only its actual usage is charged.
    let small = paid(
        n.nonce(),
        &EvmCall {
            to: Some(sender()),
            value: U256::ZERO,
            input: Bytes::new(),
            gas_limit: 21_000,
            delegate: None,
        },
    );
    assert!(
        check_admission_cost(pre, &limited, &small)
            .unwrap()
            .gas
            .state
            <= limited.limits.state
    );

    let snap = Snapshot::from_bytes(&Snapshot::of(&n.chain).to_bytes()).unwrap();
    let (next, _) = n.build(vec![]);
    let identity = n.chain.lock().identity.unwrap();
    let state = snap.check(&next, &n.chain.cfg(), &identity).unwrap();
    let (head, _) = snap.head(state);
    let restored = Chain::block_context(&n.chain.cfg(), &next, &head);
    assert_eq!(restored.limits, limited.limits);
    assert_eq!(restored.fees.unwrap().base, limited.fees.unwrap().base);
    let mut tampered = snap.clone();
    tampered.summary.excess.state = 0;
    assert!(
        tampered.check(&next, &n.chain.cfg(), &identity).is_err(),
        "snapshot debt is certified metadata"
    );

    let cfg = n.chain.cfg();
    let debt = n.parent.excess;
    drop(n);
    let (reopened, _) =
        Chain::open(cfg.clone(), Store::open(&dir.join("state.redb")).unwrap()).unwrap();
    let parent = reopened.lock().finalized.clone();
    assert_eq!(parent.excess, debt);
    let restored = Chain::block_context(&cfg, &next, &parent);
    assert_eq!(restored.limits, limited.limits);
    assert_eq!(restored.fees.unwrap().base, limited.fees.unwrap().base);
    reopened.execute(&next, &parent).unwrap();
    reopened.finalize(&next).unwrap();
    let parent = reopened.lock().finalized.clone();
    assert_eq!(
        fees::state_block_limit(parent.excess.state),
        2 * fees::STATE_UNITS_PER_BLOCK
    );
    let mut resumed = Node {
        chain: reopened,
        last: next,
        parent,
    };
    while resumed.ctx().limits.state < cost.gas.state {
        let before = resumed.ctx().limits.state;
        resumed.step(vec![]);
        assert_eq!(
            resumed.ctx().limits.state,
            before + fees::STATE_UNITS_PER_BLOCK
        );
    }
    assert!(
        resumed.chain.add_to_mempool(tx).unwrap(),
        "empty heights refill enough for the postponed write"
    );
    drop(resumed);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn ordinary_vault_sized_bursts_keep_the_original_price_and_legacy_context_is_unbounded() {
    let cfg = config();
    let (chain, genesis) = Chain::new(cfg.clone());
    let parent = chain.lock().finalized.clone();
    let ctx = Chain::block_context(&cfg, &genesis, &parent);
    let tx = with_state_cap(paid(0, &vault_sized_deployment()), fees::STATE_UNIT_PRICE);
    let deployed = execute_block(&parent.state, &ctx, std::slice::from_ref(&tx)).unwrap();
    assert!(deployed.receipts[0].success);
    assert_eq!(
        deployed
            .state
            .code(&deployed.receipts[0].contract_address.unwrap())
            .len(),
        12_588
    );
    assert!(deployed.gas.state >= 12_588 && deployed.gas.state < fees::MAX_STATE_UNITS_PER_BLOCK);
    assert_eq!(
        deployed.receipts[0].state_fee,
        U256::from(deployed.gas.state) * U256::from(fees::STATE_UNIT_PRICE)
    );
    let mut sampled = (*parent).clone();
    // The largest audited normal flow uses 12,588 units. Its burst debt
    // leaves ample capacity and keeps the original floor, even three times.
    for _ in 0..3 {
        assert!(fees::state_block_limit(sampled.excess.state) >= 12_588);
        assert_eq!(
            Chain::next_base_fee(&cfg, &sampled).state,
            fees::STATE_UNIT_PRICE
        );
        sampled.excess.state = fees::next_state_excess(sampled.excess.state, 12_588);
    }
    assert_eq!(
        Chain::next_base_fee(&cfg, &sampled).state,
        fees::STATE_UNIT_PRICE
    );
    sampled.excess.state = fees::STATE_PRICE_FREE_BURST + 1;
    assert!(Chain::next_base_fee(&cfg, &sampled).state > fees::STATE_UNIT_PRICE);

    let mut legacy = cfg;
    legacy.chain_id = 7_780;
    legacy.history_v2 = false;
    legacy.node_rewards = false;
    legacy.fees = false;
    let ctx = Chain::block_context(&legacy, &genesis, &sampled);
    assert_eq!(ctx.limits.state, u64::MAX);
    assert_eq!(
        Chain::next_base_fee(&legacy, &sampled),
        FeeVector::default()
    );
}

#[test]
fn state_growth_budget_and_congestion_price_also_apply_without_exec_fee_policy() {
    let dir = tmp("without-fees");
    let mut cfg = config();
    cfg.fees = false;
    let mut n = Node::open_with_config(&dir, cfg.clone());
    assert!(n.ctx().fees.is_none());
    assert_eq!(
        Chain::next_base_fee(&cfg, &n.parent).state,
        fees::STATE_UNIT_PRICE
    );
    let deploy = vault_sized_deployment();
    let txs = (0..4)
        .map(|nonce| with_state_cap(paid(nonce, &deploy), fees::STATE_UNIT_PRICE))
        .collect();
    let out = n.step(txs);
    assert_eq!(out.tx_hashes.len(), 4);
    assert!(out.receipts.iter().all(|r| r.success));
    assert!(out.excess.state > fees::STATE_PRICE_FREE_BURST);
    let ctx = n.ctx();
    assert_eq!(ctx.limits.state, fees::state_block_limit(out.excess.state));
    let price = Chain::next_base_fee(&cfg, &out).state;
    assert!(price > fees::STATE_UNIT_PRICE);

    let call = EvmCall {
        to: Some(sender()),
        value: U256::ZERO,
        input: Bytes::new(),
        gas_limit: 21_000,
        delegate: None,
    };
    let tx = paid(n.nonce(), &call);
    let underpriced = with_state_cap(tx.clone(), fees::STATE_UNIT_PRICE);
    assert!(n.chain.add_to_mempool(underpriced.clone()).is_err());
    assert!(execute_block(&out.state, &ctx, std::slice::from_ref(&underpriced)).is_err());
    let out = n.step(vec![tx]);
    assert_eq!(
        out.receipts[0].state_fee,
        U256::from(out.gas.state) * U256::from(price)
    );
    let before = n.ctx().limits.state;
    n.step(vec![]);
    assert_eq!(n.ctx().limits.state, before + fees::STATE_UNITS_PER_BLOCK);
    let debt = n.parent.excess.state;
    let price = Chain::next_base_fee(&cfg, &n.parent).state;
    drop(n);
    let (chain, _) =
        Chain::open(cfg.clone(), Store::open(&dir.join("state.redb")).unwrap()).unwrap();
    let parent = chain.lock().finalized.clone();
    assert_eq!(parent.excess.state, debt);
    assert_eq!(Chain::next_base_fee(&cfg, &parent).state, price);
    drop(chain);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn depleted_budget_enforces_cumulative_usage_exact_boundaries_and_congestion_price() {
    use aether_execution::{execute_block_sequential, FeePolicy, WorldState, FEE_COLLECTOR};

    let writer = Address::repeat_byte(0xc1);
    let mut pre = WorldState::default();
    pre.set_balance(sender(), U256::from(10u128.pow(24)))
        .unwrap();
    pre.set_code(
        writer,
        Bytes::from_static(&[0x60, 0x01, 0x60, 0x00, 0x35, 0x55, 0x00]),
    )
    .unwrap();
    let context = |limit| BlockContext {
        chain_id: CHAIN,
        number: 1,
        timestamp: 1,
        beneficiary: FEE_COLLECTOR,
        limits: GasVector {
            exec: 30_000_000,
            state: limit,
            prove: 200_000_000,
        },
        fees: Some(FeePolicy {
            base: FeeVector {
                exec: 0,
                state: fees::state_base_fee(fees::MAX_STATE_UNITS_PER_BLOCK - limit),
                prove: 0,
            },
            proposer: Address::repeat_byte(0xc2),
        }),
    };
    let txs: Vec<_> = (0..2)
        .map(|nonce| {
            paid(
                nonce,
                &EvmCall {
                    to: Some(writer),
                    value: U256::ZERO,
                    input: U256::from(nonce).to_be_bytes::<32>().to_vec().into(),
                    gas_limit: 100_000,
                    delegate: None,
                },
            )
        })
        .collect();
    let full = context(fees::MAX_STATE_UNITS_PER_BLOCK);
    let total = execute_block(&pre, &full, &txs).unwrap().gas.state;
    let limited = context(total - 1);
    assert!(limited.limits.state < 250);
    for tx in &txs {
        assert!(
            check_admission_cost(&pre, &limited, tx).unwrap().gas.state <= limited.limits.state
        );
    }
    let (included, proposed) = build_block(&pre, &limited, txs.clone());
    assert_eq!(
        included.len(),
        1,
        "each transaction fits alone but the pair exceeds the rolling budget"
    );
    assert!(execute_block(&pre, &limited, &txs).is_err());
    assert!(execute_block_sequential(&pre, &limited, &txs).is_err());
    assert!(
        violations(
            &txs[1..],
            &[tx_hash(&txs[0])],
            false,
            &proposed.state,
            &limited,
            proposed.gas,
            proposed.new_slots,
            proposed.persistent_bytes,
        )
        .is_empty(),
        "FOCIL respects the cumulative remaining budget"
    );
    let mut witness =
        aether_proving::block::input(&pre, &full, &txs, &[], Address::repeat_byte(0xc3)).unwrap();
    witness.ctx = limited;
    assert!(aether_proving::block::execute(&witness).is_err());

    let units = check_admission_cost(&pre, &full, &txs[0])
        .unwrap()
        .gas
        .state;
    let exact = context(units);
    assert!(exact.fees.unwrap().base.state > fees::STATE_UNIT_PRICE);
    assert_eq!(
        check_admission_cost(&pre, &exact, &txs[0])
            .unwrap()
            .gas
            .state,
        units
    );
    let (included, proposed) = build_block(&pre, &exact, vec![txs[0].clone()]);
    assert_eq!(included.len(), 1);
    let validated = execute_block(&pre, &exact, &included).unwrap();
    assert_eq!(
        validated.gas.state, exact.limits.state,
        "the exact boundary is accepted"
    );
    assert_eq!(validated.state.root(), proposed.state.root());
    let expected_burn = U256::from(units) * U256::from(exact.fees.unwrap().base.state);
    assert_eq!(validated.receipts[0].state_fee, expected_burn);
    assert_eq!(validated.settlement.burned_state, expected_burn);
    assert_eq!(
        pre.balance(&sender()) - validated.state.balance(&sender()),
        expected_burn
    );
    let witness =
        aether_proving::block::input(&pre, &exact, &included, &[], Address::repeat_byte(0xc3))
            .unwrap();
    let replay = aether_proving::block::execute(&witness).unwrap();
    assert_eq!(replay.gas, validated.gas);
    assert_eq!(replay.post_state_root, validated.state.root());
    let mut below = exact.clone();
    below.limits.state -= 1;
    assert!(check_admission_cost(&pre, &below, &txs[0]).is_err());
    assert!(execute_block(&pre, &below, &included).is_err());

    // The old floor cap is insufficient when the bucket is depleted, even
    // though the transaction's actual units fit this block exactly.
    let underpriced = with_state_cap(txs[0].clone(), fees::STATE_UNIT_PRICE);
    assert!(check_admission_cost(&pre, &exact, &underpriced).is_err());
    assert!(build_block(&pre, &exact, vec![underpriced.clone()])
        .0
        .is_empty());
    assert!(execute_block(&pre, &exact, std::slice::from_ref(&underpriced)).is_err());
    assert!(execute_block_sequential(&pre, &exact, std::slice::from_ref(&underpriced)).is_err());
    assert!(violations(
        std::slice::from_ref(&underpriced),
        &[],
        false,
        &pre,
        &exact,
        GasVector::default(),
        0,
        0
    )
    .is_empty());
    let mut witness = aether_proving::block::input(
        &pre,
        &full,
        std::slice::from_ref(&underpriced),
        &[],
        Address::repeat_byte(0xc3),
    )
    .unwrap();
    witness.ctx = exact;
    assert!(aether_proving::block::execute(&witness).is_err());
}

#[test]
fn encoded_budget_binds_all_payload_bytes_and_survives_certification_and_restart() {
    let dir = tmp("archive");
    let mut n = Node::open(&dir);
    let call = EvmCall {
        to: Some(sender()),
        value: U256::ZERO,
        input: vec![0; 256].into(),
        gas_limit: 100_000,
        delegate: None,
    };
    let tx = paid(0, &call);
    use aether_node::upgrade::{combine, sign_partial, Upgrade, MAINNET_NOTICE_BLOCKS};
    let (_, sharing, shares) = aether_light::devnet_threshold(4);
    let upgrade = Upgrade {
        chain_id: CHAIN,
        protocol: 4,
        activate_at: MAINNET_NOTICE_BLOCKS + 1,
        emergency: false,
        releases: vec![],
        registrar: None,
        notes: String::new(),
    };
    let approvals: Vec<_> = shares
        .iter()
        .map(|(_, s)| sign_partial(&upgrade, s))
        .collect();
    let signed = combine(&sharing, &approvals).unwrap();
    let extras = || Extras {
        upgrade: Some(signed.clone()),
        ..Default::default()
    };
    let (original, _) = n.build_with_extras(vec![tx.clone()], extras());
    let bytes = original.data.len() as u64;
    // Install an otherwise identical parent at the exact burst boundary.
    // Its child's parent_meta certifies this debt like any accumulated debt.
    let mut parent = (*n.parent).clone();
    parent.archive_excess = fees::MAX_ENCODED_PAYLOAD_BYTES - bytes;
    n.parent = Arc::new(parent);
    {
        let mut g = n.chain.lock();
        g.finalized = n.parent.clone();
    }
    let price = n.ctx().fees.unwrap().base.state;
    let (exact, _) = n.build_with_extras(vec![tx.clone()], extras());
    assert_eq!(exact.data.len() as u64, bytes);
    assert_eq!(exact.payload().unwrap().txs.len(), 1);

    let mut below = (*n.parent).clone();
    below.archive_excess += 1;
    let mut payload = exact.payload().unwrap();
    payload.parent_meta = below.meta_digest();
    let over = Block::new(
        exact.context.clone(),
        exact.parent,
        exact.height,
        exact.timestamp,
        payload.to_bytes(),
    );
    let err = n
        .chain
        .execute(&over, &below)
        .err()
        .expect("one byte over is refused");
    assert!(format!("{err:?}").contains("archive"), "{err:?}");
    let ctx = Chain::block_context(&n.chain.cfg(), &over, &below);
    let (trimmed, _) = build_payload(&below, &below.state, &ctx, vec![tx.clone()], extras());
    assert!(
        trimmed.txs.is_empty(),
        "proposer defers the archived transaction"
    );
    assert!(trimmed.to_bytes().len() as u64 <= fees::encoded_payload_limit(below.archive_excess));
    assert_eq!(
        ctx.fees.unwrap().base.state,
        price,
        "archive capacity never alters the state price"
    );

    let out = n.step_with_extras(vec![tx], extras());
    assert_eq!(
        out.archive_excess,
        fees::MAX_ENCODED_PAYLOAD_BYTES - fees::ENCODED_PAYLOAD_BYTES_PER_BLOCK
    );
    let snap = Snapshot::from_bytes(&Snapshot::of(&n.chain).to_bytes()).unwrap();
    assert_eq!(snap.summary.archive_excess, out.archive_excess);
    let (next, _) = n.build(vec![]);
    let identity = n.chain.lock().identity.unwrap();
    snap.check(&next, &n.chain.cfg(), &identity).unwrap();
    let mut tampered = snap.clone();
    tampered.summary.archive_excess = 0;
    assert!(tampered.check(&next, &n.chain.cfg(), &identity).is_err());

    // A system-only oversized proof cannot avoid the cap even with no paid
    // transactions. The archive gate runs before proof verification.
    let mut too_big = next.payload().unwrap();
    too_big.proofs.push(aether_light::block::ProofClaim {
        height: 1,
        prover: sender(),
        proof: "00".repeat(8192),
    });
    let oversized = Block::new(
        next.context.clone(),
        next.parent,
        next.height,
        next.timestamp,
        too_big.to_bytes(),
    );
    let err = n
        .chain
        .execute(&oversized, &out)
        .err()
        .expect("unpaid archive is bounded");
    assert!(format!("{err:?}").contains("archive"), "{err:?}");

    let cfg = n.chain.cfg();
    let debt = out.archive_excess;
    drop(n);
    let (reopened, _) = Chain::open(cfg, Store::open(&dir.join("state.redb")).unwrap()).unwrap();
    let parent = reopened.lock().finalized.clone();
    assert_eq!(parent.archive_excess, debt);
    let empty = reopened.execute(&next, &parent).unwrap();
    assert!(
        empty.archive_excess < debt,
        "empty heights replenish the bucket"
    );
    reopened.finalize(&next).unwrap();
    drop(reopened);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn archive_limited_selection_keeps_small_listed_transactions_and_focil_uses_exact_bytes() {
    use aether_node::inclusion::{InclusionList, FREEZE};
    use std::time::Instant;
    let cfg = config();
    let (chain, genesis) = Chain::new(cfg);
    let mut n = Node {
        parent: chain.lock().finalized.clone(),
        chain: chain.clone(),
        last: genesis,
    };
    let small = paid(
        0,
        &EvmCall {
            to: Some(sender()),
            value: U256::ZERO,
            input: Bytes::new(),
            gas_limit: 21_000,
            delegate: None,
        },
    );
    let key2 = P256Signer::from_seed(&dev_seed(2)).unwrap();
    let address2 = aether_crypto::address_of(&key2.public_key()).unwrap();
    let large = sign_call_with(
        &key2,
        CHAIN,
        0,
        FeeVector {
            exec: 1_000_000_000_000_000,
            state: 1_000_000_000_000_000,
            prove: 1_000_000_000_000_000,
        },
        0,
        &EvmCall {
            to: Some(address2),
            value: U256::ZERO,
            input: vec![0; 8192].into(),
            gas_limit: 100_000,
            delegate: None,
        },
    )
    .unwrap();
    let (sample, _) = n.build(vec![small.clone()]);
    let small_bytes = sample.data.len() as u64;
    let mut parent = (*n.parent).clone();
    parent.archive_excess =
        fees::MAX_ENCODED_PAYLOAD_BYTES - fees::CONTROL_ARCHIVE_RESERVE - small_bytes;
    n.parent = Arc::new(parent);
    {
        let mut g = n.chain.lock();
        g.finalized = n.parent.clone();
    }
    let first_seen = Instant::now();
    let list = InclusionList::sign(
        &aether_light::devnet_validator_key(1),
        1,
        1,
        vec![small.clone()],
    );
    assert!(n.chain.lock().inclusion.accept(&list, first_seen));
    let (selected, out) = n.build(vec![large, small.clone()]);
    assert_eq!(
        out.tx_hashes,
        vec![tx_hash(&small)],
        "a large earlier candidate cannot hide an appendable listed tail"
    );
    let payload = selected.payload().unwrap();
    assert_eq!(selected.data.len() as u64, small_bytes);
    assert!(n
        .chain
        .inclusion_violations_in_payload(&out, &n.ctx(), first_seen + FREEZE, &n.parent, &payload)
        .is_empty());

    let (empty, empty_out) = n.build(vec![]);
    assert_eq!(
        n.chain.inclusion_violations_in_payload(
            &empty_out,
            &n.ctx(),
            first_seen + FREEZE,
            &n.parent,
            &empty.payload().unwrap()
        ),
        vec![tx_hash(&small)],
        "exact append boundary is enforceable"
    );
    let mut below = (*n.parent).clone();
    below.archive_excess += 1;
    let ctx = Chain::block_context(&n.chain.cfg(), &empty, &below);
    let (below_payload, _) = build_payload(&below, &below.state, &ctx, vec![], Extras::default());
    assert!(
        n.chain
            .inclusion_violations_in_payload(
                &empty_out,
                &ctx,
                first_seen + FREEZE,
                &below,
                &below_payload
            )
            .is_empty(),
        "one byte beyond archive capacity is not censorship"
    );
}
