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
