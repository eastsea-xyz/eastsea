//! Old-rules golden for a synthetic history built from the shipped 7780 genesis.
//! This is deterministic execution/replay coverage, not an archived testnet replay.

use aether_crypto::{address_of, P256Signer, Signer as _};
use aether_execution::{sign_call_with, EvmCall};
use aether_node::block::{Block, Context, PublicKey, EPOCH};
use aether_node::chain::{build_payload, Chain, ChainConfig, Executed, Extras};
use aether_node::roster::NetworkFile;
use aether_types::{Address, Bytes, FeeVector, GasVector, B256, U256};
use commonware_codec::{Decode, DecodeExt, Encode};
use commonware_consensus::types::{Round, View};
use commonware_cryptography::Digestible;

const NETWORK: &[u8] = include_bytes!("fixtures/legacy-7780-network.json");
// Pin the reported hash while running against the pre-change implementation.
const EXPECTED_TRANSCRIPT: &str = "ba2adc97c77ab73f2865bf23bd8f3eb706a8225184f9eb6522a07abc43ba2ce7";

fn config(file: &NetworkFile) -> ChainConfig {
    // Mirror main.rs::chain_config with dev_alloc=false; do not substitute a
    // hand-written registrar, faucet allocation, or shortened registry epoch.
    let genesis = file.genesis().unwrap();
    ChainConfig {
        chain_id: file.chain_id,
        limits: GasVector {
            exec: 30_000_000,
            state: u64::MAX,
            prove: 200_000_000,
        },
        alloc: genesis
            .faucet
            .map(|f| vec![(f, U256::from(aether_node::faucet::SUPPLY))])
            .unwrap_or_default(),
        fees: true,
        registrar: genesis.registrar,
        epoch_blocks: genesis.epoch_blocks,
        min_streak: genesis.min_streak,
        draw_epochs: genesis.draw_epochs,
        history_v2: genesis.history >= 2,
        protocol: genesis.protocol,
        node_rewards: genesis.node_rewards,
        committee: genesis.committee,
        reserve: genesis.reserve,
        group: genesis.group,
        max_committee: genesis.max_committee,
    }
}

#[derive(Debug, PartialEq, Eq)]
struct Frame {
    encoded: Vec<u8>,
    state_root: B256,
    metadata: B256,
    payouts: Vec<(u64, Address, U256)>,
}

impl Frame {
    fn of(block: &Block, executed: &Executed) -> Self {
        Self {
            encoded: block.encode().to_vec(),
            state_root: executed.state.root(),
            metadata: executed.meta_digest(),
            payouts: executed.payouts.clone(),
        }
    }

    fn hash_into(&self, transcript: &mut blake3::Hasher) {
        transcript.update(&(self.encoded.len() as u64).to_be_bytes());
        transcript.update(&self.encoded);
        transcript.update(self.state_root.as_slice());
        transcript.update(self.metadata.as_slice());
        transcript.update(&(self.payouts.len() as u64).to_be_bytes());
        for (height, address, amount) in &self.payouts {
            transcript.update(&height.to_be_bytes());
            transcript.update(address.as_slice());
            transcript.update(&amount.to_be_bytes::<32>());
        }
    }
}

#[test]
fn legacy_7780_encoded_history_replays_byte_identically() {
    // The existing shipped-network SHA-256 pin also applies to this fixture.
    assert!(aether_node::mainnet::shipped_legacy_network(NETWORK));
    let file: NetworkFile = serde_json::from_slice(NETWORK).unwrap();
    replay_archived_7780(&file);
    assert_eq!(file.chain_id, 7_780);
    let cfg = config(&file);
    assert!(!cfg.node_rewards);
    assert!(!cfg.history_v2);
    assert!(cfg.reserve.is_none());
    assert_eq!(cfg.protocol, 1);
    assert_eq!(cfg.group, 0);
    let leaders: Vec<PublicKey> = file
        .validators
        .iter()
        .map(|m| PublicKey::decode(hex::decode(&m.key).unwrap().as_slice()).unwrap())
        .collect();
    let (source, genesis) = Chain::new(cfg.clone());
    let mut parent = source.lock().finalized.clone();
    let genesis_root = parent.state.root();
    let genesis_digest = genesis.digest();
    // The file's absent epoch_blocks is zero in ChainConfig, which installs
    // the real testnet default in registry state; do not shorten that epoch.
    let params = aether_execution::registry::params(&parent.state);
    let epoch_blocks = params.epoch_blocks;
    assert_eq!(epoch_blocks, 3_600);
    let last_height = epoch_blocks + 2;
    let mut frames = vec![Frame::of(&genesis, &parent)];
    source.finalize(&genesis).unwrap();

    // Real signatures on zero-value, zero-fee calls exercise nonce/state
    // changes without needing the actual faucet's private key or modifying
    // its genesis allocation. Exactly heights 1, 3 and 5 carry calls; all
    // remaining blocks are empty, through the first real epoch boundary.
    let signer = P256Signer::from_seed(&[0x37; 32]).unwrap();
    let sender = address_of(&signer.public_key()).unwrap();
    let mut last = genesis;
    let mut source_boundaries = 0;
    for height in 1..=last_height {
        if parent.height / epoch_blocks != height / epoch_blocks {
            source_boundaries += 1;
        }
        let context = Context {
            round: Round::new(EPOCH, View::new(height)),
            leader: leaders[(height as usize - 1) % leaders.len()].clone(),
            parent: (View::new(height - 1), last.digest()),
        };
        let timestamp = 1_791_417_600_000 + height * 1_000;
        let skeleton = Block::new(
            context.clone(),
            last.digest(),
            last.height.next(),
            timestamp,
            bytes::Bytes::new(),
        );
        let ctx = Chain::block_context(&cfg, &skeleton, &parent);
        let (pre, _) = source
            .pre_state(&parent, parent.next_protocol(), &[], None, false)
            .unwrap();
        let txs = if matches!(height, 1 | 3 | 5) {
            vec![sign_call_with(
                &signer,
                file.chain_id,
                height / 2,
                FeeVector::default(),
                0,
                &EvmCall {
                    to: Some(Address::repeat_byte(0xb1)),
                    value: U256::ZERO,
                    input: Bytes::new(),
                    gas_limit: 21_000,
                    delegate: None,
                },
            )
            .unwrap()]
        } else {
            vec![]
        };
        let expected_txs = txs.len();
        let (payload, _) = build_payload(&parent, &pre, &ctx, txs, Extras::default());
        assert_eq!(payload.txs.len(), expected_txs, "height {height}");
        assert!(payload.receipts_root.is_none(), "legacy receipt layout");
        drop(pre);
        let block = Block::new(
            context,
            last.digest(),
            last.height.next(),
            timestamp,
            payload.to_bytes(),
        );
        parent = source.execute(&block, &parent).unwrap();
        assert!(parent.payouts.is_empty(), "no protocol-1 rewards");
        assert_eq!(parent.archive_excess, 0, "no legacy archive debt");
        source.finalize(&block).unwrap();
        frames.push(Frame::of(&block, &parent));
        last = block;
    }
    assert_eq!(
        source_boundaries, 1,
        "exactly one real registry epoch boundary"
    );
    assert_eq!(source.finalized_height(), last_height);
    assert_eq!(parent.height, last_height);
    assert_eq!(parent.height / epoch_blocks, 1);
    assert_eq!(aether_execution::registry::params(&parent.state), params);
    assert_eq!(frames.len(), last_height as usize + 1);
    assert_eq!(parent.state.nonce(&sender), 3);
    assert_ne!(parent.state.root(), genesis_root);

    // Replay the encoded source blocks, not newly proposed equivalents. A
    // second Chain has its own state and execution cache from genesis onward.
    let (replay, replay_genesis) = Chain::new(cfg);
    let mut replay_parent = replay.lock().finalized.clone();
    assert_eq!(Frame::of(&replay_genesis, &replay_parent), frames[0]);
    replay.finalize(&replay_genesis).unwrap();
    let mut replay_boundaries = 0;
    for (height, expected) in frames.iter().enumerate().skip(1) {
        let block =
            Block::decode_cfg(expected.encoded.as_slice(), &Block::codec_config(8 << 20)).unwrap();
        assert_eq!(block.height.get(), height as u64);
        if replay_parent.height / epoch_blocks != block.height.get() / epoch_blocks {
            replay_boundaries += 1;
        }
        replay_parent = replay.execute(&block, &replay_parent).unwrap();
        replay.finalize(&block).unwrap();
        assert_eq!(
            Frame::of(&block, &replay_parent),
            *expected,
            "height {height}"
        );
    }
    assert_eq!(
        replay_boundaries, 1,
        "replay crosses the same real boundary"
    );
    assert_eq!(replay.finalized_height(), last_height);
    assert_eq!(replay_parent.height, last_height);
    assert_eq!(replay_parent.height / epoch_blocks, 1);
    assert_eq!(replay_parent.state.nonce(&sender), 3);
    assert_eq!(
        aether_execution::registry::params(&replay_parent.state),
        params
    );

    let mut transcript = blake3::Hasher::new();
    transcript.update(b"aether-reserve-floor-legacy7780-v1");
    transcript.update(&(frames.len() as u64).to_be_bytes());
    for frame in &frames {
        frame.hash_into(&mut transcript);
    }
    let actual = transcript.finalize().to_hex().to_string();
    assert_eq!(
        actual, EXPECTED_TRANSCRIPT,
        "legacy7780 pre-change transcript; genesis digest={genesis_digest}, root={genesis_root:#x}"
    );
}

// Optional, immutable RPC capture: replay it without launching or reading a node.
fn replay_archived_7780(file: &NetworkFile) {
    use std::io::BufRead as _;

    let Ok(path) = std::env::var("AETHER_7780_REPLAY") else {
        return;
    };
    let rows = std::io::BufReader::new(std::fs::File::open(path).unwrap());
    let (chain, genesis) = Chain::new(config(file));
    chain.lock().identity = Some(
        aether_light::Identity::decode(
            hex::decode(file.identity.as_ref().unwrap())
                .unwrap()
                .as_slice(),
        )
        .unwrap(),
    );
    chain.finalize(&genesis).unwrap();
    let mut transcript = blake3::Hasher::new();
    let mut count = 0u64;
    for line in rows.lines() {
        let row: serde_json::Value = serde_json::from_str(&line.unwrap()).unwrap();
        let height = row["height"].as_u64().unwrap();
        assert_eq!(height, count, "the archived range is contiguous");
        let block = if height == 0 {
            genesis.clone()
        } else {
            let bytes = hex::decode(
                row["finalized"]["block"]
                    .as_str()
                    .unwrap()
                    .trim_start_matches("0x"),
            )
            .unwrap();
            Block::decode_cfg(bytes.as_slice(), &Block::codec_config(8 << 20)).unwrap()
        };
        assert_eq!(block.height.get(), height);
        assert_eq!(
            block.digest().to_string(),
            row["summary"]["hash"].as_str().unwrap(),
            "archived block hash at {height}"
        );
        if height > 0 {
            chain
                .finalize(&block)
                .unwrap_or_else(|e| panic!("archived execution at {height}: {e:?}"));
        }
        let exec = chain.lock().finalized.clone();
        let root: B256 = serde_json::from_value(row["summary"]["state_root"].clone()).unwrap();
        assert_eq!(
            exec.state.root(),
            root,
            "archived 7780 state root at {height}"
        );
        Frame::of(&block, &exec).hash_into(&mut transcript);
        if height.is_multiple_of(10_000) {
            println!("archived7780 replay height={height} root={root:#x}");
        }
        count += 1;
    }
    assert_eq!(
        count, 79_878,
        "capture includes the protocol-2 and protocol-3 switches"
    );
    let last = chain.lock().finalized.clone();
    assert_eq!(
        aether_node::upgrade::protocol_at(&last.schedule, last.height),
        3
    );
    println!(
        "archived7780 PASS range=0..{} roots={count} transcript={}",
        last.height,
        transcript.finalize().to_hex()
    );
}
