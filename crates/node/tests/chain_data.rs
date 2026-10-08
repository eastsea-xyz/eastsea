//! `aether run --chain-data` (the wallet's 블록 데이터 위치 on a secondary
//! disk): an unplugged disk leaves its /Volumes path missing, and the node
//! must never create it — that would quietly re-sync the whole chain onto
//! the internal disk under a /Volumes name. The real binary exits 13
//! (`EXIT_CHAIN_DATA_MISSING`) and writes nothing at all.

use aether_test_support::{Port, TestChild};
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

struct TmpDir(PathBuf);
impl Drop for TmpDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn key_fixture(tag: &str) -> TmpDir {
    let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().parent().unwrap();
    let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let root = TmpDir(workspace.join("tmp").join(format!("aether-key-volume-{tag}-{}-{nonce}", std::process::id())));
    std::fs::create_dir_all(root.0.join("node")).unwrap();
    std::fs::create_dir(root.0.join("other")).unwrap();
    std::fs::write(root.0.join("node/validator.key"), b"fixture key sentinel").unwrap();
    // Any run that passes the volume check still ends before opening a
    // network or starting a node; these bytes are deliberately invalid.
    std::fs::write(root.0.join("invalid-network.json"), b"invalid fixture network").unwrap();
    root
}

fn key_fixture_command(root: &TmpDir) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_aether"));
    command.arg("run").arg("--data").arg(root.0.join("node"))
        .arg("--network").arg(root.0.join("invalid-network.json"))
        .arg("--min-free-disk=0")
        .env_remove("AETHER_TEST_INTERNAL_KEY_DIR");
    command
}

#[cfg(all(feature = "test-seam", debug_assertions))]
#[test]
fn only_an_exact_fixture_volume_approval_passes_external_key_rejection() {
    let root = key_fixture("approval");
    let data = root.0.join("node");
    if !aether_node::supervisor::volume_is_external(&data) { return; }
    let denied = key_fixture_command(&root).output().unwrap();
    assert_eq!(denied.status.code(), Some(aether_node::supervisor::EXIT_KEYS_ON_CHAIN_DATA));
    let mismatch = key_fixture_command(&root)
        .env("AETHER_TEST_INTERNAL_KEY_DIR", root.0.join("other")).output().unwrap();
    assert_eq!(mismatch.status.code(), Some(aether_node::supervisor::EXIT_KEYS_ON_CHAIN_DATA));
    let allowed = key_fixture_command(&root)
        .env("AETHER_TEST_INTERNAL_KEY_DIR", &data).output().unwrap();
    assert_eq!(allowed.status.code(), Some(1), "the invalid fixture never starts a node");
    assert!(String::from_utf8_lossy(&allowed.stderr).contains("is not a network.json"),
        "exact fixture approval reaches network validation: {}", String::from_utf8_lossy(&allowed.stderr));
}

#[test]
fn fixture_volume_approval_never_allows_keys_in_chain_data() {
    let root = key_fixture("chain");
    let chain = root.0.join("chain");
    std::fs::create_dir(&chain).unwrap();
    std::fs::write(chain.join("validator.key"), b"chain key sentinel").unwrap();
    let refused = key_fixture_command(&root).arg("--chain-data").arg(&chain)
        .env("AETHER_TEST_INTERNAL_KEY_DIR", root.0.join("node")).output().unwrap();
    assert_eq!(refused.status.code(), Some(aether_node::supervisor::EXIT_KEYS_ON_CHAIN_DATA));
    assert!(String::from_utf8_lossy(&refused.stderr).contains("key files found in the chain data directory"));
    assert_eq!(std::fs::read(chain.join("validator.key")).unwrap(), b"chain key sentinel");
}

#[test]
fn a_missing_chain_data_directory_exits_13_and_creates_nothing() {
    let root = TmpDir(std::env::temp_dir().join(format!("aether-chain-data-{}", std::process::id())));
    let _ = std::fs::remove_dir_all(&root.0);
    std::fs::create_dir_all(&root.0).unwrap();
    let data = root.0.join("node");
    let volume = root.0.join("Volumes").join("Unplugged");
    let chain = volume.join("EastSea");
    let rpc_port = Port::reserve().expect("reserve RPC port");
    let p2p_port = Port::reserve().expect("reserve P2P port");
    let log_path = root.0.join("run.log");
    let mut command = Command::new(env!("CARGO_BIN_EXE_aether"));
    command.arg("run")
        .arg("--data").arg(&data)
        .arg("--chain-data").arg(&chain)
        .arg("--rpc-port").arg(rpc_port.to_string())
        .arg("--port").arg(p2p_port.to_string());
    let child = TestChild::spawn(command, &log_path)
        .expect("spawn the aether binary");
    let deadline = Instant::now() + Duration::from_secs(120);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break Some(status);
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    let out = std::fs::read_to_string(&log_path).unwrap_or_default();
    let status = status.unwrap_or_else(|| panic!("a run without its disk must stop, not wait: {out}"));
    assert_eq!(status.code(), Some(aether_node::supervisor::EXIT_CHAIN_DATA_MISSING), "{out}");
    assert!(!volume.exists(), "the missing disk's path must not be created: {out}");
    assert!(!data.exists(), "nothing is written before the disk is back: {out}");
    assert!(out.contains("not connected") || out.contains("does not exist"), "the refusal says why: {out}");
}
