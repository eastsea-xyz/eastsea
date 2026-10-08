//! Archive node and era export (roadmap B6): a node that keeps everything
//! writes every sealed era as a static, torrent-ready file set (era file,
//! Manifest, .torrent with webseeds, index), serves the files over plain
//! HTTP (`GET /era/<file>`) alongside the era RPCs, and a pruned peer fetches
//! an old era from it, verified against its own certified history root.
//!
//! Blocks are built and finalized the way a proposer and marshal do, without
//! consensus (as in `tests/prune.rs`).

use aether_crypto::{P256Signer, Signer as AetherSigner};
use aether_execution::{recommended_state_budget, sign_call_with, EvmCall};
use aether_node::block::{Block, Context, EPOCH};
use aether_node::chain::{build_payload, dev_accounts, dev_seed, Chain, ChainConfig, Executed, Extras};
use aether_node::era;
use aether_node::era_net;
use aether_node::export::{self, ExportArgs};
use aether_node::follow::{FinalityArchive, Upstream};
use aether_node::prune::{self, Retention};
use aether_node::rpc::{self, RpcState};
use aether_node::store::Store;
use aether_test_support::Port;
use aether_state::mmr::ERA_LEN;
use aether_types::{Address, Bytes, FeeVector, GasVector, TxEnvelope, U256};
use commonware_consensus::types::{Round, View};
use commonware_cryptography::{ed25519, Digestible, Signer};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[path = "common/rpc.rs"]
mod test_rpc;

const CHAIN: u64 = 7_793;
const GH_WEBSEED: &str = "https://github.com/pipln/eastsea-releases/releases/download/eras/~name~";

fn config() -> ChainConfig {
    ChainConfig {
        chain_id: CHAIN,
        limits: GasVector { exec: 30_000_000, state: u64::MAX, prove: 200_000_000 },
        alloc: dev_accounts(4).into_iter().map(|(_, a)| (a, U256::from(10u128.pow(24)))).collect(),
        fees: true,
        registrar: Some(([1; 32], [2; 32])),
        epoch_blocks: 10,
        min_streak: None,
        draw_epochs: None,
        history_v2: true,
        protocol: 1,
        node_rewards: false,
        group: 0,
        max_committee: aether_node::rotation::GROW_UNTIL,
        committee: vec![],
        reserve: None,
    }
}

struct Node {
    chain: Chain,
    blocks: Vec<Block>,
    parent: Arc<Executed>,
    nonce: u64,
}

impl Node {
    fn start(dir: &Path) -> Node {
        let (chain, genesis) = Chain::open(config(), Store::open(&dir.join("state.redb")).unwrap()).unwrap();
        let (_, sharing, _) = aether_light::devnet_threshold(4);
        chain.lock().identity = Some(*sharing.public());
        chain.finalize(&genesis).unwrap();
        let parent = chain.lock().finalized.clone();
        Node { chain, blocks: vec![genesis], parent, nonce: 0 }
    }

    /// The same node after a restart (blocks and nonce carried over by the test).
    fn reopen(dir: &Path, blocks: Vec<Block>, nonce: u64) -> Node {
        let (chain, _) = Chain::open(config(), Store::open(&dir.join("state.redb")).unwrap()).unwrap();
        let (_, sharing, _) = aether_light::devnet_threshold(4);
        chain.lock().identity = Some(*sharing.public());
        let parent = chain.lock().finalized.clone();
        Node { chain, blocks, parent, nonce }
    }

    fn step(&mut self, txs: Vec<TxEnvelope>) -> Arc<Executed> {
        let prev = self.blocks.last().unwrap();
        let height = prev.height.next();
        let h = height.get();
        let leader = ed25519::PrivateKey::from_seed(h % 4).public_key();
        let context = Context { round: Round::new(EPOCH, View::new(h)), leader, parent: (View::new(h - 1), prev.digest()) };
        let ts = 1_790_000_000_000 + h * 1000;
        let skeleton = Block::new(context.clone(), prev.digest(), height, ts, bytes::Bytes::new());
        let ctx = Chain::block_context(&self.chain.cfg(), &skeleton, &self.parent);
        let (pre, _) = self.chain.pre_state(&self.parent, self.parent.next_protocol(), &[], None, false).unwrap();
        let (payload, _) = build_payload(&self.parent, &pre, &ctx, txs, Extras::default());
        drop(pre);
        let block = Block::new(context, prev.digest(), height, ts, payload.to_bytes());
        let exec = self.chain.execute(&block, &self.parent).unwrap();
        self.chain.finalize(&block).unwrap();
        self.blocks.push(block);
        self.parent = exec.clone();
        exec
    }

    fn transfer(&mut self) -> TxEnvelope {
        let signer = P256Signer::from_seed(&dev_seed(1)).unwrap();
        let call = EvmCall { to: Some(Address::repeat_byte(0xb0)), value: U256::from(1_000u64), input: Bytes::new(), gas_limit: 21_000, delegate: None };
        let fees = FeeVector { exec: 100_000_000_000, state: aether_execution::fees::STATE_UNIT_PRICE, prove: 100_000_000_000 };
        self.nonce += 1;
        let mut tx = sign_call_with(&signer, CHAIN, self.nonce - 1, fees, 1_000_000_000, &call).unwrap();
        tx.header.gas.state = recommended_state_budget(&call, Some(U256::from(10u128.pow(24))), fees.state);
        let mut signature = AetherSigner::sign(&signer, &tx.signing_bytes()).unwrap();
        signature.extend_from_slice(&AetherSigner::public_key(&signer).bytes);
        tx.signature = Bytes::from(signature);
        tx
    }

    fn run_to(&mut self, height: u64, with_tx: impl Fn(u64) -> bool) {
        while self.parent.height < height {
            let t = if with_tx(self.parent.height + 1) { vec![self.transfer()] } else { vec![] };
            self.step(t);
        }
    }
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("aether-archive-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let target = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_dir(&e.path(), &target);
        } else {
            std::fs::copy(e.path(), target).unwrap();
        }
    }
}

fn wait_until(what: &str, mut ok: impl FnMut() -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(180);
    while !ok() {
        assert!(std::time::Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

fn rpc_state(chain: Chain, upstream: Option<Arc<Upstream>>) -> RpcState {
    RpcState {
        chain,
        finality: rpc::Finality::Archive(Arc::new(FinalityArchive::new(None))),
        gossip: tokio::sync::mpsc::unbounded_channel().0,
        faucet: None,
        registrar: None,
        network: None,
        upstream,
        handoff: None,
        snapshot: Default::default(),
        prover: None,
        shards: None,
        presence: None,
        public_read_only: false,
    }
}

/// The info hash a magnet names (its `xt=urn:btih:` parameter — the first
/// parameter sits right after `magnet:?`, before any `&`).
fn btih_of(magnet: &str) -> [u8; 20] {
    let xt = magnet.split('&').find(|p| p.contains("urn:btih:")).expect("an xt parameter");
    let hash = xt.split("urn:btih:").nth(1).expect("a hash after the urn");
    hex::decode(hash).expect("hex info hash").try_into().expect("20 bytes")
}

fn sha1_of(b: &[u8]) -> [u8; 20] {
    use sha1::{Digest as _, Sha1};
    let mut h = Sha1::new();
    h.update(b);
    h.finalize()[..].try_into().expect("20 bytes")
}

/// The whole B6 story on one chain of two sealed eras and a bit (building it
/// commits ~16k blocks): the archive node writes the static set, the set is
/// internally consistent (manifest signature, torrent pieces, magnet info
/// hash), its bytes are served over plain HTTP, and a node that pruned era 0
/// gets it back, verified.
#[test]
fn archive_node_exports_a_verifiable_set_and_pruned_peers_fetch_it() {
    let (dir_a, dir_b, out) = (tmp("node"), tmp("pruned"), tmp("export"));
    let mut a = Node::start(&dir_a);
    a.run_to(2 * ERA_LEN + 20, |h| h % 1000 == 0);
    let store_a = a.chain.store().unwrap();
    wait_until("eras 0 and 1 sealed", || store_a.staged_eras().unwrap() == vec![2] && dir_a.join("eras").join(era::file_name(1)).exists());
    drop(store_a);
    let (blocks, nonce) = (a.blocks.clone(), a.nonce);
    drop(a);
    // The archive node: the same data, reopened.
    let a = Node::reopen(&dir_a, blocks.clone(), nonce);

    // It exports: era files, manifests, torrents, an index.
    let port = Port::reserve().expect("reserve archive export RPC port");
    let http = format!("http://127.0.0.1:{port}");
    let args = ExportArgs {
        dir: out.clone(),
        webseeds: vec![GH_WEBSEED.to_string()],
        https_base: Some(http.clone()),
        sign_key: dir_a.join("archive-export.key"),
    };
    let written = export::once(&a.chain, &args).unwrap();
    assert!(written >= 7, "era files, torrents, manifests and the index ({written})");
    // A second pass changes nothing.
    assert_eq!(export::once(&a.chain, &args).unwrap(), 0, "already exported");

    let index: Value = serde_json::from_slice(&std::fs::read(out.join("index.json")).unwrap()).unwrap();
    assert_eq!(index["chain_id"], serde_json::json!(CHAIN));
    assert_eq!(index["eras"].as_array().unwrap().len(), 2, "eras 0 and 1");
    let signer = index["signer"].as_str().unwrap().to_string();

    // Each era's set is internally consistent and matches the sealed file.
    for e in 0..2u64 {
        let entry = &index["eras"][e as usize];
        let name = era::file_name(e);
        let sealed = std::fs::read(dir_a.join("eras").join(&name)).unwrap();
        assert_eq!(std::fs::read(out.join(&name)).unwrap(), sealed, "the export is byte for byte the sealed file");
        assert_eq!(entry["blake3"], serde_json::json!(rpc::blake3_hex(&sealed)));
        assert_eq!(entry["size"], serde_json::json!(sealed.len()));
        assert_eq!(entry["first"], serde_json::json!(e * ERA_LEN));

        // The manifest: signed by the index's key, correct range, both mirrors.
        let m: aether_types::Manifest =
            serde_json::from_slice(&std::fs::read(out.join(format!("{name}.json"))).unwrap()).unwrap();
        assert_eq!(m.blake3, rpc::blake3_hex(&sealed));
        assert_eq!(m.size, sealed.len() as u64);
        assert_eq!(m.range, Some((e * ERA_LEN, (e + 1) * ERA_LEN - 1)));
        assert_eq!(m.chain_id, CHAIN);
        assert!(export::verify_manifest(&m, &signer), "the manifest is signed by the exporter's key");
        let magnet = entry["magnet"].as_str().unwrap().to_string();
        let (https, torrent_mirror) = match &m.mirrors[..] {
            [aether_types::Mirror::Https { url }, t @ aether_types::Mirror::Torrent { .. }] => (url.clone(), t.clone()),
            _ => panic!("an Https mirror and a Torrent mirror, in that order"),
        };
        assert_eq!(https, format!("{http}/era/{name}"));
        match torrent_mirror {
            aether_types::Mirror::Torrent { magnet: m2, webseeds } => {
                assert_eq!(m2, magnet);
                assert_eq!(webseeds[0], https);
                assert_eq!(webseeds[1], GH_WEBSEED.replace("~name~", &name), "the placeholder takes the file name");
            }
            _ => unreachable!(),
        }

        // The torrent: pieces hash the file, the info hash is the magnet's.
        let torrent_bytes = std::fs::read(out.join(format!("{name}.torrent"))).unwrap();
        let (decoded, _) = export::ben_decode(&torrent_bytes).unwrap();
        assert_pieces_hash(&decoded, &sealed);
        let export::Ben::Dict(top) = &decoded else { panic!("the metainfo is a dictionary") };
        let (_, info) = top.iter().find(|(k, _)| k == b"info").expect("an info dict");
        assert_eq!(sha1_of(&export::ben_encode(info.clone())), btih_of(&magnet), "the magnet names this torrent");
        // The era still verifies against the root the index records.
        let root: [u8; 32] = hex::decode(entry["root"].as_str().unwrap()).unwrap().try_into().unwrap();
        era::read(&sealed, Some(&root)).unwrap();
    }

    // The archive node serves: the era RPCs, and the file itself as a plain
    // GET (a webseed).
    let rt = tokio::runtime::Runtime::new().unwrap();
    let st_a = rpc_state(a.chain.clone(), None);
    rt.spawn(test_rpc::serve(port, st_a));
    std::thread::sleep(std::time::Duration::from_millis(300));
    let name0 = era::file_name(0);
    let sealed0 = std::fs::read(dir_a.join("eras").join(&name0)).unwrap();
    rt.block_on(async {
        let got = reqwest::get(format!("{http}/era/{name0}")).await.unwrap();
        assert_eq!(got.status(), 200);
        assert_eq!(got.bytes().await.unwrap().to_vec(), sealed0, "the webseed serves the sealed bytes");
        // Only era files: no path tricks, no other names.
        let bad = reqwest::get(format!("{http}/era/../state.redb")).await.unwrap();
        assert_eq!(bad.status(), 404);
        let bad = reqwest::get(format!("{http}/era/whatever")).await.unwrap();
        assert_eq!(bad.status(), 404);
    });

    // A pruned peer gets era 0 back from the archive node, verified against
    // its own certified history root (the era_net path a pruned Mac takes).
    copy_dir(&dir_a, &dir_b);
    let b = Node::reopen(&dir_b, blocks.clone(), nonce);
    let r = Retention { blocks: ERA_LEN, keep_era_files: false };
    prune::prune_once(&b.chain, &r).unwrap().expect("era 0 pruned");
    assert!(!dir_b.join("eras").join(&name0).exists(), "the era file is gone here");
    let up = || Upstream::Http(vec![http.clone()]);
    let path = rt.block_on(era_net::fetch_into(&b.chain, &up(), 0)).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), sealed0);
    assert_eq!(b.chain.old_block(5).unwrap(), blocks[5], "the era's blocks are readable again");
    assert_eq!(b.chain.old_block(5).unwrap().height.get(), 5);

    drop((a, b));
    let _ = std::fs::remove_dir_all(&dir_a);
    let _ = std::fs::remove_dir_all(&dir_b);
    let _ = std::fs::remove_dir_all(&out);
}

/// Every piece of the metainfo hashes the file it claims (piece length from
/// the metainfo itself, `length` equal to the file), webseeds are inside
/// (url-list), a tracker is not (DHT discovery).
fn assert_pieces_hash(torrent: &export::Ben, file: &[u8]) {
    let export::Ben::Dict(top) = torrent else { panic!("the metainfo is a dictionary") };
    let (_, info) = top.iter().find(|(k, _)| k == b"info").expect("an info dict");
    let export::Ben::Dict(info) = info else { panic!("info is a dictionary") };
    let get = |k: &str| info.iter().find(|(key, _)| key == k.as_bytes()).expect(k).1.clone();
    let (export::Ben::Int(length), export::Ben::Int(piece_len), export::Ben::Bytes(pieces)) =
        (get("length"), get("piece length"), get("pieces"))
    else {
        panic!("length, piece length, pieces");
    };
    assert_eq!(length as usize, file.len());
    let piece_len = piece_len as usize;
    assert_eq!(pieces.len(), file.len().div_ceil(piece_len) * 20);
    for (i, chunk) in file.chunks(piece_len).enumerate() {
        assert_eq!(&pieces[i * 20..(i + 1) * 20], &sha1_of(chunk), "piece {i}");
    }
    assert!(top.iter().any(|(k, _)| k == b"url-list"));
    assert!(!top.iter().any(|(k, _)| k == b"announce"));
}
