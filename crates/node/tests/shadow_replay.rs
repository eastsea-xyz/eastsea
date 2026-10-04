//! Shadow replay of a short history-v2 dev chain, including a paid transfer.

use aether_crypto::{P256Signer, Signer as AetherSigner};
use aether_execution::{EvmCall, recommended_state_budget, sign_call_with};
use aether_node::block::{Block, Context, EPOCH};
use aether_node::chain::{Chain, ChainConfig, Extras, build_payload, dev_accounts, dev_seed};
use aether_node::shadow::{self, Source};
use aether_node::store::Store;
use aether_types::{Address, Bytes, FeeVector, GasVector, U256};
use commonware_consensus::types::{Round, View};
use commonware_cryptography::{Digestible, Signer, ed25519};
use std::path::Path;
use std::process::Command;

fn config() -> ChainConfig {
    ChainConfig {
        chain_id: 7792,
        limits: GasVector {
            exec: 30_000_000,
            state: u64::MAX,
            prove: 200_000_000,
        },
        alloc: vec![(dev_accounts(4)[0].1, U256::from(aether_node::faucet::SUPPLY))],
        fees: true,
        registrar: None,
        epoch_blocks: 10,
        min_streak: None,
        draw_epochs: None,
        history_v2: true,
        protocol: 1,
        node_rewards: false,
        committee: vec![],
        reserve: None,
        group: 0,
        max_committee: aether_node::rotation::GROW_UNTIL,
    }
}

fn make_chain(dir: &Path) {
    let (chain, genesis) =
        Chain::open(config(), Store::open(&dir.join("state.redb")).unwrap()).unwrap();
    chain.finalize(&genesis).unwrap();
    let mut parent = chain.lock().finalized.clone();
    let mut previous = genesis;
    for h in 1..=2 {
        let height = previous.height.next();
        let leader = ed25519::PrivateKey::from_seed(h % 4).public_key();
        let context = Context {
            round: Round::new(EPOCH, View::new(h)),
            leader,
            parent: (View::new(h - 1), previous.digest()),
        };
        let timestamp = 1_790_000_000_000 + h * 1000;
        let skeleton = Block::new(
            context.clone(),
            previous.digest(),
            height,
            timestamp,
            bytes::Bytes::new(),
        );
        let ctx = Chain::block_context(&chain.cfg(), &skeleton, &parent);
        let (pre, _) = chain
            .pre_state(&parent, parent.next_protocol(), &[], None, false)
            .unwrap();
        let txs = if h == 1 {
            let signer = P256Signer::from_seed(&dev_seed(1)).unwrap();
            let call = EvmCall {
                to: Some(Address::repeat_byte(0xb0)),
                value: U256::from(1000u64),
                input: Bytes::new(),
                gas_limit: 21_000,
                delegate: None,
            };
            let fees = FeeVector {
                exec: 100_000_000_000,
                state: aether_execution::fees::STATE_UNIT_PRICE,
                prove: 100_000_000_000,
            };
            let mut tx = sign_call_with(&signer, 7792, 0, fees, 1_000_000_000, &call).unwrap();
            // The faucet is funded: pay for the new account the transfer
            // creates, so the chain really carries a paid transfer whose fee
            // rule the changed replay below can flip (state growth 100 units,
            // burned at the fixed unit price).
            tx.header.gas.state = recommended_state_budget(&call, Some(U256::from(aether_node::faucet::SUPPLY)), fees.state);
            let mut signature = AetherSigner::sign(&signer, &tx.signing_bytes()).unwrap();
            signature.extend_from_slice(&AetherSigner::public_key(&signer).bytes);
            tx.signature = Bytes::from(signature);
            vec![tx]
        } else {
            vec![]
        };
        let (payload, _) = build_payload(&parent, &pre, &ctx, txs, Extras::default());
        drop(pre);
        let block = Block::new(
            context,
            previous.digest(),
            height,
            timestamp,
            payload.to_bytes(),
        );
        parent = chain.execute(&block, &parent).unwrap();
        chain.finalize(&block).unwrap();
        previous = block;
    }
}

#[test]
fn dev_chain_matches_and_changed_fee_rule_is_detected() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .unwrap()
        .join("tmp")
        .join(format!("shadow-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    let source_dir = base.join("source");
    make_chain(&source_dir);
    let source = Source::open(source_dir.to_str().unwrap(), &base.join("source-copy")).unwrap();
    shadow::replay(config(), &source, 2, &base.join("matching")).unwrap();
    let network = base.join("genesis.json");
    std::fs::write(&network, serde_json::json!({
        "chain_id": 7792,
        "validators": [],
        "faucet": dev_accounts(4)[0].1,
        "epoch_blocks": 10,
        "history": 2,
    }).to_string()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_aether"))
        .args(["shadow", "--from", source_dir.to_str().unwrap(), "--to", "2", "--network", network.to_str().unwrap()])
        .current_dir(&base)
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(String::from_utf8_lossy(&output.stdout).contains("shadow PASS through finalized height 2"));
    let mut changed = config();
    changed.fees = false;
    let error = shadow::replay(changed, &source, 2, &base.join("changed")).unwrap_err();
    assert!(error.contains("SHADOW MISMATCH height 1"), "{error}");
    drop(source);
    std::fs::remove_dir_all(base).unwrap();
}
