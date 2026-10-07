//! A copied identity must stop the real validator binary before any vote.

#![cfg(all(feature = "test-seam", debug_assertions))]

use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

struct Dir(PathBuf);
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn key_binding_copied_data_directory_on_another_mac_refuses_to_vote() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let root = Dir(workspace
        .join("tmp")
        .join(format!("aether-key-binding-copy-{}", std::process::id())));
    let source = root.0.join("original");
    let copied = root.0.join("copied");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::create_dir_all(&copied).unwrap();
    aether_node::roster::LocalKeys::devnet(1)
        .save(&source)
        .unwrap();
    aether_node::faucet::Faucet::generate(&source.join(aether_node::candidate::ACCOUNT_FILE))
        .unwrap();
    let keys = aether_node::candidate::CandidateKeys::load_or_create(&source).unwrap();
    std::fs::write(source.join("network.json"), serde_json::to_vec(&serde_json::json!({
        "chain_id": 7_780,
        "validators": (1..=4).map(|i| aether_node::roster::LocalKeys::devnet(i).public()).collect::<Vec<_>>(),
    })).unwrap()).unwrap();
    let hash = Sha256::digest(b"eastsea.bindMAC-A");
    // Fixture represents the originating Mac's binding, independent of the
    // test runner's own hardware. The child seam alone pretends to be Mac B.
    std::fs::write(
        source.join("key-binding.json"),
        serde_json::to_vec(&serde_json::json!({
            "validator_pub": hex::encode(keys.validator_key()),
            "platform_uuid_hash": hex::encode(hash),
            "created_at": 1,
        }))
        .unwrap(),
    )
    .unwrap();
    let original = Command::new(env!("CARGO_BIN_EXE_aether"))
        .arg("candidate-info")
        .arg("--data")
        .arg(&source)
        .env("AETHER_TEST_PLATFORM_UUID", "MAC-A")
        .output()
        .unwrap();
    assert!(
        original.status.success(),
        "the originating Mac must still accept its keys: {}",
        String::from_utf8_lossy(&original.stderr)
    );
    for entry in std::fs::read_dir(&source).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_file() {
            std::fs::copy(entry.path(), copied.join(entry.file_name())).unwrap();
        }
    }
    std::fs::copy(
        source.with_extension("identity"),
        copied.with_extension("identity"),
    )
    .unwrap();
    let (code, output) = validator(&copied);
    assert_eq!(
        code,
        Some(aether_node::key_binding::EXIT_KEY_ELSEWHERE),
        "a copied validator must stop with the hardware-binding reason: {output}"
    );
    assert!(
        output.contains("another Mac"),
        "the refusal must be a plain error line: {output}"
    );
    assert!(
        !copied.join("vote-epoch-0.seen").exists(),
        "no voting engine was entered"
    );
    assert_eq!(
        std::fs::read(copied.join("validator.key")).unwrap(),
        std::fs::read(source.join("validator.key")).unwrap()
    );
}

fn validator(data: &Path) -> (Option<i32>, String) {
    let log_path = data.join("validator.log");
    let log = std::fs::File::create(&log_path).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_aether"))
        .args(["node", "--offline", "--port", "0", "--rpc-port", "0"])
        .args(["--peers", "2@127.0.0.1:9,3@127.0.0.1:9,4@127.0.0.1:9"])
        .arg("--data")
        .arg(data)
        .arg("--network")
        .arg(data.join("network.json"))
        .env("AETHER_TEST_PLATFORM_UUID", "MAC-B")
        .stdout(Stdio::from(log.try_clone().unwrap()))
        .stderr(Stdio::from(log))
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            let status = child.wait().unwrap();
            break status;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    (status.code(), std::fs::read_to_string(log_path).unwrap())
}

#[test]
fn key_binding_legacy_key_is_bound_once_and_logged() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let root = Dir(workspace
        .join("tmp")
        .join(format!("aether-key-binding-legacy-{}", std::process::id())));
    let data = root.0.join("node");
    aether_node::candidate::CandidateKeys::load_or_create(&data).unwrap();
    let path = data.join(aether_node::key_binding::BINDING_FILE);
    let _ = std::fs::remove_file(&path);
    let run = || {
        Command::new(env!("CARGO_BIN_EXE_aether"))
            .arg("candidate-info")
            .arg("--data")
            .arg(&data)
            .env("RUST_LOG", "warn")
            .output()
            .unwrap()
    };
    let first = run();
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert!(
        path.exists(),
        "an existing identity must gain its hardware binding"
    );
    let bytes = std::fs::read(&path).unwrap();
    let output = format!(
        "{}{}",
        String::from_utf8_lossy(&first.stdout),
        String::from_utf8_lossy(&first.stderr)
    );
    assert_eq!(
        output.matches("created missing hardware binding").count(),
        1,
        "{output}"
    );
    let second = run();
    assert!(second.status.success());
    assert_eq!(std::fs::read(path).unwrap(), bytes);
    let output = format!(
        "{}{}",
        String::from_utf8_lossy(&second.stdout),
        String::from_utf8_lossy(&second.stderr)
    );
    assert!(
        !output.contains("created missing hardware binding"),
        "{output}"
    );
}
