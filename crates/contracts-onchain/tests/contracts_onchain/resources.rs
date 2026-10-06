//! Capacity, refill, congestion and legacy execution of compiled contracts.
use super::harness::*;
use aether_execution::{
    block::{receipt_persistent_bytes, tx_persistent_bytes},
    build_block, execute_block, execute_block_sequential, fees,
    receipt::receipt_root,
    EvmCall,
};
use alloy_sol_types::{sol, SolCall, SolValue};

sol! {
    function mint(address to, uint8 color, uint8 shape, uint8 pattern, uint8 halo);
    function lock(address recipient, bytes32 hashlock, uint64 timelock, address token, uint256 amount) returns(uint256);
    function claim(uint256 id, bytes preimage);
}

#[test]
fn real_deploy_waits_for_bucket_refill_without_stalling_empty_blocks() {
    let mut h = Harness::new();
    let call = EvmCall {
        to: None,
        input: bytecode("core/AtomicSwap"),
        value: U256::ZERO,
        gas_limit: TX_GAS,
        delegate: None,
    };
    let tx = h.signed(0, &call, None);
    let full = execute_block(&h.state, &h.ctx, std::slice::from_ref(&tx)).unwrap();
    let units = full.gas.state;
    assert!(full.receipts[0].success && units > 32);
    h.excess = fees::MAX_STATE_UNITS_PER_BLOCK - units + 1;
    h.refresh();
    let scarce = h.ctx.clone();
    let root = h.state.root();
    assert!(execute_block(&h.state, &scarce, std::slice::from_ref(&tx)).is_err());
    let (included, proposed) = build_block(&h.state, &scarce, vec![tx.clone()]);
    assert!(included.is_empty());
    assert_eq!(proposed.state.root(), root);
    let empty = execute_block(&h.state, &scarce, &[]).unwrap();
    assert!(empty.receipts.is_empty());
    assert_eq!(empty.state.root(), root);
    h.advance(1);
    assert_eq!(
        h.ctx.limits.state,
        scarce.limits.state + fees::STATE_UNITS_PER_BLOCK
    );
    let paid = execute_block(&h.state, &h.ctx, std::slice::from_ref(&tx)).unwrap();
    assert!(paid.receipts[0].success);
    assert_eq!(paid.gas.state, units);
    assert!(
        paid.receipts[0].state_fee > full.receipts[0].state_fee,
        "depleted bucket adds congestion surcharge"
    );
    let (included, proposal) = build_block(&h.state, &h.ctx, vec![tx]);
    let replay = execute_block_sequential(&h.state, &h.ctx, &included).unwrap();
    assert_eq!(proposal.state.root(), replay.state.root());
    assert_eq!(
        receipt_root(&proposal.receipts),
        receipt_root(&replay.receipts)
    );
}

#[test]
fn depleted_block_accepts_exact_cost_and_refuses_one_unit_less() {
    let mut h = Harness::new();
    let to = h.deploy("core/AtomicSwap", vec![]);
    let input = lockCall {
        recipient: h.addr(1),
        hashlock: B256::repeat_byte(1),
        timelock: h.timestamp() + 100,
        token: Address::ZERO,
        amount: U256::from(1),
    }
    .abi_encode();
    let call = EvmCall {
        to: Some(to),
        value: U256::from(1),
        input: input.into(),
        gas_limit: TX_GAS,
        delegate: None,
    };
    let tx = h.signed(0, &call, None);
    let baseline = execute_block(&h.state, &h.ctx, std::slice::from_ref(&tx)).unwrap();
    let cost = baseline.gas.state;
    let mut exact = h.ctx.clone();
    exact.limits.state = cost;
    let mut below = exact.clone();
    below.limits.state -= 1;
    assert!(
        execute_block(&h.state, &exact, std::slice::from_ref(&tx))
            .unwrap()
            .receipts[0]
            .success
    );
    assert!(execute_block(&h.state, &below, std::slice::from_ref(&tx)).is_err());
    let (kept, refused) = build_block(&h.state, &below, vec![tx]);
    assert!(kept.is_empty());
    assert_eq!(refused.state.root(), h.state.root());
}

#[test]
fn nft_mints_to_fresh_users_measure_slot_and_paid_state_capacity() {
    let mut h = Harness::new();
    // Constructor is name, symbol, maxSupply, royaltyBps, guardian;
    // use compiler ABI below to ensure this fixture's arguments stay explicit.
    let abi = &artifacts()["toolbox/OnchainNFT"]["abi"];
    let constructor = abi
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["type"] == "constructor")
        .unwrap();
    assert_eq!(constructor["inputs"].as_array().unwrap().len(), 5);
    let args = (
        "Capacity".to_owned(),
        "CAP".to_owned(),
        U256::from(1000),
        alloy_primitives::aliases::U96::ZERO,
        h.addr(0),
    )
        .abi_encode_params();
    let nft = h.deploy("toolbox/OnchainNFT", args);
    // Begin a fully refilled burst, after paying for the deployment in its own
    // block. Unique recipients have no balance/account record in genesis.
    h.at(h.timestamp() + 4000);
    assert_eq!(h.excess, 0);
    let nonce = h.nonce(0);
    let txs: Vec<_> = (0..600u64)
        .map(|i| {
            let to = Address::from_word(B256::from(U256::from(10_000 + i).to_be_bytes::<32>()));
            assert!(h.state.account(&to).is_none());
            let call = EvmCall {
                to: Some(nft),
                value: U256::ZERO,
                input: mintCall {
                    to,
                    color: 0,
                    shape: 0,
                    pattern: 0,
                    halo: 0,
                }
                .abi_encode()
                .into(),
                gas_limit: 250_000,
                delegate: None,
            };
            let mut tx = h.signed(0, &call, None);
            tx.header.nonce = nonce + i;
            use aether_crypto::Signer;
            let signer = h.signer(0);
            let mut sig = signer.sign(&tx.signing_bytes()).unwrap();
            sig.extend_from_slice(&signer.public_key().bytes);
            tx.signature = sig.into();
            tx
        })
        .collect();
    let (included, first) = build_block(&h.state, &h.ctx, txs.clone());
    assert!(!included.is_empty() && included.len() < txs.len());
    assert!(first.receipts.iter().all(|r| r.success));
    assert_eq!(first.new_slots, 3 * included.len() as u64);
    assert_eq!(
        included.len(),
        170,
        "512-slot cap binds at three fresh NFT slots each"
    );
    assert!(first.new_slots <= fees::MAX_NEW_SLOTS_PER_BLOCK);
    assert!(first.gas.state <= fees::MAX_STATE_UNITS_PER_BLOCK);
    let replay = execute_block(&h.state, &h.ctx, &included).unwrap();
    assert_eq!(first.state.root(), replay.state.root());
    assert_eq!(
        receipt_root(&first.receipts),
        receipt_root(&replay.receipts)
    );
    assert!(
        execute_block(&h.state, &h.ctx, &txs).is_err(),
        "validator refuses overflowing burst"
    );
    let count = included.len() as u64;
    let day_units = fees::MAX_STATE_UNITS_PER_BLOCK + 86_400 * fees::STATE_UNITS_PER_BLOCK;
    // This is a paid-state upper bound, not a promise of end-to-end throughput:
    // BAL and certified payload encoding also consume node archive capacity.
    let amortized = first.gas.state.div_ceil(count);
    eprintln!(
        "NFT_CAPACITY burst={count} units={} slots={} bytes={} state_day_upper_bound={}",
        first.gas.state,
        first.new_slots,
        first.persistent_bytes,
        day_units / amortized
    );
    assert!(day_units / amortized >= count);
    for i in 0..included.len() {
        let recipient = Address::from_word(B256::from(U256::from(10_000 + i).to_be_bytes::<32>()));
        assert!(
            first.state.account(&recipient).is_none(),
            "NFT ownership is stored in contract slots, not recipient account headers"
        );
    }
    let mut next_ctx = h.ctx.clone();
    next_ctx.number += 1;
    next_ctx.timestamp += 1;
    let debt = fees::next_state_excess(0, first.gas.state);
    next_ctx.limits.state = fees::state_block_limit(debt);
    next_ctx.fees.as_mut().unwrap().base.state = fees::state_base_fee(debt);
    let next = execute_block(&first.state, &next_ctx, &[]).unwrap();
    assert!(next.receipts.is_empty());
    assert_eq!(next.state.root(), first.state.root());
}

#[test]
fn actual_contract_runs_on_legacy_7780_with_free_state_and_original_receipts() {
    let mut h = Harness::new();
    h.ctx.chain_id = 7780;
    h.ctx.limits.state = u64::MAX;
    h.ctx.fees.as_mut().unwrap().base.state = 0;
    let call = EvmCall {
        to: None,
        value: U256::ZERO,
        input: bytecode("core/AtomicSwap"),
        gas_limit: TX_GAS,
        delegate: None,
    };
    let tx = h.signed(0, &call, Some(0));
    let (included, out) = build_block(&h.state, &h.ctx, vec![tx]);
    assert_eq!(included.len(), 1);
    assert!(out.receipts[0].success);
    assert_eq!(out.gas.state, 0);
    assert_eq!(out.persistent_bytes, 0);
    assert_eq!(out.receipts[0].state_fee, U256::ZERO);
    let deployed = out.receipts[0].contract_address.unwrap();
    h.state = out.state;
    let preimage = b"legacy parity".to_vec();
    let hashlock =
        alloy_primitives::b256!("37363a46fda0b778ef6221b574c5d0f83d277d83338ed040b94f9ad21ca25dc9");
    let input = lockCall {
        recipient: h.addr(1),
        hashlock,
        timelock: h.timestamp() + 100,
        token: Address::ZERO,
        amount: U256::from(3),
    }
    .abi_encode();
    let lock_tx = h.signed(
        0,
        &EvmCall {
            to: Some(deployed),
            value: U256::from(3),
            input: input.into(),
            gas_limit: 500_000,
            delegate: None,
        },
        Some(0),
    );
    let locked = execute_block(&h.state, &h.ctx, &[lock_tx]).unwrap();
    assert!(locked.receipts[0].success);
    assert_eq!(locked.gas.state, 0);
    h.state = locked.state;
    let claim_tx = h.signed(
        1,
        &EvmCall {
            to: Some(deployed),
            value: U256::ZERO,
            input: claimCall {
                id: U256::ZERO,
                preimage: preimage.into(),
            }
            .abi_encode()
            .into(),
            gas_limit: 500_000,
            delegate: None,
        },
        Some(0),
    );
    let pre = h.state.balance(&h.addr(1));
    let claimed = execute_block(&h.state, &h.ctx, &[claim_tx]).unwrap();
    assert!(claimed.receipts[0].success);
    assert_eq!(claimed.state.balance(&h.addr(1)) - pre, U256::from(3));
    assert_eq!(claimed.gas.state, 0);
}

#[test]
fn payload_refill_and_receipt_bytes_remain_independently_bounded() {
    assert_eq!(fees::MAX_ENCODED_PAYLOAD_BYTES, 8 << 20);
    let mut debt = fees::MAX_ENCODED_PAYLOAD_BYTES - 1;
    assert_eq!(fees::encoded_payload_limit(debt), 1);
    debt = fees::next_archive_excess(debt, 0);
    assert_eq!(fees::encoded_payload_limit(debt), 1 + 4096);
    let mut h = Harness::new();
    let contract = h.deploy("core/AtomicSwap", vec![]);
    let receipt = h.revert(
        0,
        contract,
        vec![0xde, 0xad, 0xbe, 0xef],
        U256::ZERO,
        "AtomicSwap/unknown-selector-archive",
    );
    assert!(receipt_persistent_bytes(&receipt) >= fees::RECEIPT_BASE_BYTES);
    assert!(
        receipt.state_fee > U256::ZERO,
        "even reverted receipt is paid archive state"
    );
    let call = EvmCall {
        to: Some(contract),
        value: U256::ZERO,
        input: Bytes::new(),
        gas_limit: 250_000,
        delegate: None,
    };
    let tx = h.signed(0, &call, None);
    assert!(tx_persistent_bytes(&tx) > 0);
}

#[test]
fn exec_state_and_prove_fee_vector_charges_success_and_revert_as_designed() {
    let mut h = Harness::new();
    h.ctx.fees.as_mut().unwrap().base.exec = 7;
    h.ctx.fees.as_mut().unwrap().base.prove = 11;
    let contract = h.deploy("core/AtomicSwap", vec![]);
    let sender = h.addr(0);
    let escrow = aether_execution::PROVER_ESCROW;
    let before = h.state.balance(&sender);
    let escrow_before = h.state.balance(&escrow);
    let receipt = h.ok(
        0,
        contract,
        lockCall {
            recipient: h.addr(1),
            hashlock: B256::repeat_byte(1),
            timelock: h.timestamp() + 100,
            token: Address::ZERO,
            amount: U256::from(1),
        }
        .abi_encode(),
        U256::from(1),
        "AtomicSwap/three-dimensional-fees-success",
    );
    let exec = U256::from(receipt.gas_used) * U256::from(7);
    let prove = U256::from(receipt.prove_gas) * U256::from(11);
    assert!(receipt.state_fee > U256::ZERO && exec > U256::ZERO && prove > U256::ZERO);
    assert_eq!(
        before - h.state.balance(&sender),
        receipt.state_fee + exec + prove + U256::from(1)
    );
    assert_eq!(h.state.balance(&escrow) - escrow_before, prove);
    let escrow_before = h.state.balance(&escrow);
    let rejected = h.revert(
        0,
        contract,
        vec![0xde, 0xad, 0xbe, 0xef],
        U256::from(2),
        "AtomicSwap/three-dimensional-fees-revert",
    );
    assert_eq!(
        h.state.balance(&escrow) - escrow_before,
        U256::from(rejected.prove_gas) * U256::from(11)
    );
}

#[test]
fn first_sender_and_funded_recipient_records_pay_and_unfunded_sender_is_refused() {
    let mut h = Harness::new();
    let fresh = h.addr(200);
    assert!(h.state.account(&fresh).is_none());
    let call = EvmCall {
        to: Some(h.addr(0)),
        value: U256::ZERO,
        input: Bytes::new(),
        gas_limit: 21_000,
        delegate: None,
    };
    let free = h.signed(200, &call, Some(0));
    let root = h.state.root();
    assert!(execute_block(&h.state, &h.ctx, &[free.clone()]).is_err());
    let (included, out) = build_block(&h.state, &h.ctx, vec![free]);
    assert!(included.is_empty());
    assert_eq!(root, out.state.root());
    let funding = h.ok(
        0,
        fresh,
        vec![],
        U256::from(10u128.pow(20)),
        "FirstSender/funded-account-growth",
    );
    assert!(funding.state_gas >= fees::STATE_ACCOUNT_UNITS);
    let receipt = h.ok(
        200,
        h.addr(0),
        vec![],
        U256::ZERO,
        "FirstSender/first-nonce-record-and-receipt",
    );
    assert!(receipt.state_gas > 0);
    assert_eq!(h.state.nonce(&fresh), 1);
    let next = h.ok(
        200,
        h.addr(0),
        vec![],
        U256::ZERO,
        "FirstSender/subsequent-nonce-receipt",
    );
    assert_eq!(
        next.state_gas, receipt.state_gas,
        "same envelope size does not recharge existing account"
    );
}

sol! { interface AirdropCapacity { function claim(uint256 amount, bytes32[] proof) external; } }

#[test]
fn airdrop_fresh_claim_slots_fit_remaining_bucket_and_daily_state_bound() {
    let mut h = Harness::new();
    const USERS: usize = 128;
    let amount = U256::from(1);
    let mut leaves = Vec::new();
    for i in 0..USERS {
        let actor = (16 + i) as u8;
        let address = h.addr(actor);
        // Genesis fee allocations are explicit; native account growth is
        // tested separately by the first-sender funding regression above.
        h.state
            .set_balance(address, U256::from(10u128.pow(20)))
            .unwrap();
        h.state.set_code(address, Bytes::new()).unwrap();
        let mut packed = address.as_slice().to_vec();
        packed.extend_from_slice(&amount.to_be_bytes::<32>());
        leaves.push(alloy_primitives::keccak256(packed));
    }
    let mut levels = vec![leaves];
    while levels.last().unwrap().len() > 1 {
        let next = levels
            .last()
            .unwrap()
            .chunks_exact(2)
            .map(|pair| {
                let (a, b) = if pair[0] < pair[1] {
                    (pair[0], pair[1])
                } else {
                    (pair[1], pair[0])
                };
                let mut bytes = a.as_slice().to_vec();
                bytes.extend_from_slice(b.as_slice());
                alloy_primitives::keccak256(bytes)
            })
            .collect();
        levels.push(next);
    }
    let merkle_root = levels.last().unwrap()[0];
    let drop = h.deploy(
        "toolbox/MerkleAirdrop",
        (
            merkle_root,
            h.addr(0),
            alloy_primitives::aliases::U48::from(86_400u64),
        )
            .abi_encode_params(),
    );
    h.ok(
        0,
        drop,
        vec![],
        U256::from(USERS),
        "MerkleAirdrop/capacity-fund-pool",
    );
    h.at(h.timestamp() + 4000);
    let txs: Vec<_> = (0..USERS)
        .map(|i| {
            let mut index = i;
            let mut proof = Vec::new();
            for level in &levels[..levels.len() - 1] {
                proof.push(level[index ^ 1]);
                index /= 2;
            }
            let call = EvmCall {
                to: Some(drop),
                value: U256::ZERO,
                input: AirdropCapacity::claimCall { amount, proof }
                    .abi_encode()
                    .into(),
                gas_limit: 250_000,
                delegate: None,
            };
            h.signed((16 + i) as u8, &call, None)
        })
        .collect();
    let burst = execute_block(&h.state, &h.ctx, &txs).unwrap();
    assert!(burst.receipts.iter().all(|r| r.success));
    assert_eq!(
        burst.new_slots,
        USERS as u64 + 1,
        "one claimed bit/user and one totalClaimed slot"
    );
    let ten = execute_block(&h.state, &h.ctx, &txs[..10]).unwrap();
    h.excess = fees::MAX_STATE_UNITS_PER_BLOCK - ten.gas.state + 1;
    h.refresh();
    let (included, limited) = build_block(&h.state, &h.ctx, txs.clone());
    assert_eq!(
        included.len(),
        9,
        "nine claims fit one unit below the ten-claim boundary"
    );
    assert!(limited.receipts.iter().all(|r| r.success));
    let replay = execute_block(&h.state, &h.ctx, &included).unwrap();
    assert_eq!(limited.state.root(), replay.state.root());
    assert_eq!(
        receipt_root(&limited.receipts),
        receipt_root(&replay.receipts)
    );
    assert!(execute_block(&h.state, &h.ctx, &txs[..10]).is_err());
    h.advance(1);
    let (refilled, next) = build_block(&h.state, &h.ctx, txs[..10].to_vec());
    assert_eq!(refilled.len(), 10);
    assert!(next.receipts.iter().all(|r| r.success));
    let units_per_claim = burst.gas.state.div_ceil(USERS as u64);
    let day = fees::MAX_STATE_UNITS_PER_BLOCK + 86_400 * fees::STATE_UNITS_PER_BLOCK;
    eprintln!("AIRDROP_CAPACITY sampled_burst={USERS} units={} slots={} bytes={} state_day_upper_bound={}",burst.gas.state,burst.new_slots,burst.persistent_bytes,day/units_per_claim);
}

#[test]
fn every_constructor_free_artifact_uses_paid_wallet_deployment_and_refusal() {
    for (name, artifact) in artifacts() {
        let constructor = artifact["abi"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["type"] == "constructor");
        if constructor.is_some_and(|c| !c["inputs"].as_array().unwrap().is_empty()) {
            continue;
        }
        let mut h = Harness::new();
        h.deploy(name, vec![]);
    }
}

sol! { interface RandomnessView { function randomness(uint64 epoch) external view returns(uint256); } }
#[test]
fn toolbox_randomness_deploy_and_read_use_actual_executor_storage() {
    let mut h = Harness::new();
    let oracle = h.deploy("toolbox/Randomness", vec![]);
    let input = RandomnessView::randomnessCall { epoch: 7 }.abi_encode();
    assert_eq!(
        U256::from_be_slice(&h.view(0, oracle, input.clone())),
        U256::ZERO
    );
    let slot = (U256::from(9) << 200) | U256::from(7);
    // Represents the explicit system seed write made before user transactions.
    h.state.set_storage(oracle, slot, U256::from(777));
    assert_eq!(
        U256::from_be_slice(&h.view(0, oracle, input)),
        U256::from(777)
    );
    h.advance(1);
    assert_eq!(
        U256::from_be_slice(&h.view(
            0,
            oracle,
            RandomnessView::randomnessCall { epoch: 8 }.abi_encode()
        )),
        U256::ZERO
    );
}

fn instructions(code: &[u8]) -> &[u8] {
    assert!(code.len() >= 2);
    let n = u16::from_be_bytes(code[code.len() - 2..].try_into().unwrap()) as usize;
    &code[..code.len().checked_sub(n + 2).unwrap()]
}

#[test]
fn compiled_fixtures_and_actual_predeploys_have_expected_code_and_version_boundaries() {
    assert_eq!(
        runtime("core/ReleaseLog"),
        aether_execution::release_log::code()
    );
    assert_eq!(
        instructions(&runtime("core/EastSeaAccount")),
        instructions(&aether_execution::aether_account_code_v2())
    );
    assert_eq!(
        instructions(&runtime("core/CommitteeRegistry")),
        instructions(&aether_execution::registry::code_v2())
    );
    // V3 is a deployment candidate; installing it into the protocol predeploy
    // would change execution rules, so this suite never alters that rule.
    assert_ne!(
        instructions(&runtime("core/CommitteeRegistryV3")),
        instructions(&aether_execution::registry::code_v2())
    );
    let mut h = Harness::new();
    h.transact(
        0,
        EvmCall {
            to: Some(h.addr(0)),
            value: U256::ZERO,
            input: aether_execution::encode_execute(&[]),
            gas_limit: TX_GAS,
            delegate: Some(aether_execution::AETHER_ACCOUNT),
        },
        "EastSeaAccount/actual-pinned-predeploy-delegation",
        true,
    );
}

#[test]
fn p256_envelope_signature_chain_and_nonce_failures_leave_no_receipt_or_state() {
    use aether_crypto::Signer;
    let h = Harness::new();
    let call = EvmCall {
        to: None,
        value: U256::ZERO,
        input: bytecode("core/AtomicSwap"),
        gas_limit: TX_GAS,
        delegate: None,
    };
    let good = h.signed(0, &call, None);
    let mut corrupt = good.clone();
    let mut bytes = corrupt.signature.to_vec();
    bytes[0] ^= 1;
    corrupt.signature = bytes.into();
    let mut wrong_chain = good.clone();
    wrong_chain.header.chain_id = 7780;
    let mut future_nonce = good.clone();
    future_nonce.header.nonce += 1;
    for tx in [&mut wrong_chain, &mut future_nonce] {
        let signer = h.signer(0);
        let mut bytes = signer.sign(&tx.signing_bytes()).unwrap();
        bytes.extend_from_slice(&signer.public_key().bytes);
        tx.signature = bytes.into();
    }
    for tx in [corrupt, wrong_chain, future_nonce] {
        let root = h.state.root();
        assert!(execute_block(&h.state, &h.ctx, std::slice::from_ref(&tx)).is_err());
        let (kept, out) = build_block(&h.state, &h.ctx, vec![tx]);
        assert!(kept.is_empty());
        assert!(out.receipts.is_empty());
        assert_eq!(out.state.root(), root);
    }
}
