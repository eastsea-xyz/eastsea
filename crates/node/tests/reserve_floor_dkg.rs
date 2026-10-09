//! An evolved network file may seat a reserve key in its current committee,
//! but starting a fresh DKG with that committee must still reject the overlap.
//! The frozen genesis roster and a completed previous round cannot bypass the
//! initial-ceremony boundary, including on a nonzero-round retry.

use aether_node::roster::{LocalKeys, NetworkFile, ReserveFile, KEY_FILE, PUBLIC_FILE};
use aether_types::Address;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

struct TmpDir(PathBuf);

impl TmpDir {
    fn new(tag: &str) -> Self {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        let tmp = workspace.join("tmp");
        std::fs::create_dir_all(&tmp).unwrap();
        let path = tmp.join(format!(
            "aether-reserve-floor-dkg-{tag}-{}-{:016x}",
            std::process::id(),
            rand::random::<u64>()
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for TmpDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn dkg_once(network: &Path, data: &Path, log_path: &Path) -> (Option<ExitStatus>, String) {
    let log = std::fs::File::create(log_path).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_aether"))
        .arg("dkg")
        .arg("--network")
        .arg(network)
        .arg("--data")
        .arg(data)
        .args([
            "--port",
            "0",
            "--offline",
            "--round",
            "9",
            "--peers",
            "2@127.0.0.1:0,3@127.0.0.1:0,4@127.0.0.1:0",
        ])
        .env_remove(aether_node::supervisor::WRITER_LEASE_ENV)
        .env_remove("AETHER_TEST_INTERNAL_KEY_DIR")
        .stdout(Stdio::from(log.try_clone().unwrap()))
        .stderr(Stdio::from(log))
        .spawn()
        .expect("spawn the fixture's DKG command");
    let deadline = Instant::now() + Duration::from_secs(20);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            result => {
                let _ = child.kill();
                let _ = child.wait();
                if let Err(error) = result {
                    panic!("wait for the fixture's DKG command: {error}");
                }
                break None;
            }
        }
    };
    (status, std::fs::read_to_string(log_path).unwrap())
}

fn current_reserve_overlap_is_rejected(node_only: bool) {
    let root = TmpDir::new(if node_only { "node" } else { "key" });
    let data = root.0.join("node");
    let independent: Vec<_> = (0..4).map(|_| LocalKeys::generate()).collect();
    independent[0].save(&data).unwrap();
    let initial: Vec<_> = independent.iter().map(LocalKeys::public).collect();
    let reserve = LocalKeys::generate().public();
    let mut current = initial.clone();
    if node_only {
        current[3].node = reserve.node.clone();
    } else {
        current[3] = reserve.clone();
    }
    let file = NetworkFile {
        chain_id: 7_809,
        validators: current,
        // p2p_args does not decode a previous round's identity or output when
        // starting DKG. Populating both must not weaken the overlap ban.
        identity: Some("ab".repeat(48)),
        round: 7,
        output: Some("cd".repeat(64)),
        epochs: vec![],
        faucet: None,
        registrar: None,
        epoch_blocks: None,
        min_streak: None,
        draw_epochs: None,
        history: Some(2),
        protocol: None,
        node_rewards: Some(true),
        reserve: Some(ReserveFile {
            operator: Address::repeat_byte(0xf0),
            validators: vec![reserve],
        }),
        group: None,
        max_committee: None,
        genesis_validators: Some(initial),
        release: None,
    };
    assert!(
        file.genesis().is_ok(),
        "the frozen genesis stays disjoint from reserve when the current roster evolves"
    );
    let network = root.0.join("input-network.json");
    let network_bytes = serde_json::to_vec_pretty(&file).unwrap();
    std::fs::write(&network, &network_bytes).unwrap();
    let key_bytes = std::fs::read(data.join(KEY_FILE)).unwrap();
    let public_bytes = std::fs::read(data.join(PUBLIC_FILE)).unwrap();

    let (status, log) = dkg_once(&network, &data, &root.0.join("dkg.log"));
    let status = status.expect("overlap rejects before a runtime or ceremony starts");
    assert_eq!(status.code(), Some(1), "the overlap is a CLI error: {log}");
    let expected = if node_only {
        "validator 4: its node id is also a reserve key's"
    } else {
        "validator 4: its key is also a reserve key"
    };
    assert_eq!(log.trim(), format!("error: {expected}"));
    let mut entries: Vec<_> = std::fs::read_dir(&data)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    entries.sort();
    assert_eq!(
        entries,
        vec![KEY_FILE.to_string(), PUBLIC_FILE.to_string()],
        "rejection creates no threshold share, runtime, journal, or network output"
    );
    assert_eq!(std::fs::read(data.join(KEY_FILE)).unwrap(), key_bytes);
    assert_eq!(std::fs::read(data.join(PUBLIC_FILE)).unwrap(), public_bytes);
    assert_eq!(std::fs::read(network).unwrap(), network_bytes);
}

#[test]
fn a_nonzero_dkg_retry_rejects_a_current_reserve_validator_key() {
    current_reserve_overlap_is_rejected(false);
}

#[test]
fn a_nonzero_dkg_retry_rejects_a_current_reserve_node_id() {
    current_reserve_overlap_is_rejected(true);
}
