//! `aether validator-program`, the release gate's reading of the network's
//! proof program (scripts/prover-gate.sh): the validators' answer when they
//! give one, the compiled-in program of their chain when they predate the RPC,
//! and a failure (never a guess) otherwise.

use aether_test_support::Port;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::Command;

const OLD_7780: &str = "3e9c897628c91fc1bbbf977a1a05d6b5d53ae4cd69038fad5abe7106b989b1ec";

/// A JSON-RPC endpoint that answers every request with `body`.
fn serve(body: &'static str) -> String {
    let port = Port::reserve().expect("reserve validator-program RPC port");
    let listener = port.bind_tcp().unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        let _port = port;
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            let reply = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(reply.as_bytes());
        }
    });
    url
}

/// The app's bundled network.json, with its chain id replaced by `chain_id`.
fn network(chain_id: u64) -> PathBuf {
    let bundled = concat!(env!("CARGO_MANIFEST_DIR"), "/../../apps/wallet/Resources/network.json");
    let mut file: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(bundled).unwrap()).unwrap();
    file["chain_id"] = chain_id.into();
    let path = std::env::temp_dir().join(format!("validator-program-{}-{chain_id}.json", std::process::id()));
    std::fs::write(&path, file.to_string()).unwrap();
    path
}

fn ask(chain_id: u64, rpc: &str) -> (bool, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_aether"))
        .args(["validator-program", "--network"])
        .arg(network(chain_id))
        .args(["--rpc", rpc, "--timeout", "10"])
        .output()
        .unwrap();
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).trim().to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

const METHOD_NOT_FOUND: &str =
    r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32601,"message":"method not found: aether_proverProgram"}}"#;

#[test]
fn an_answering_validator_is_the_source_of_truth() {
    let rpc = serve(r#"{"jsonrpc":"2.0","id":1,"result":"aaaabbbb"}"#);
    let (ok, program, _) = ask(7780, &rpc);
    assert!(ok);
    assert_eq!(program, "aaaabbbb", "the RPC answer wins over the 7780 pin");
}

#[test]
fn old_7780_validators_map_to_their_known_program() {
    let rpc = serve(METHOD_NOT_FOUND);
    let (ok, program, stderr) = ask(7780, &rpc);
    assert!(ok, "{stderr}");
    assert_eq!(program, OLD_7780);
    assert!(stderr.contains("predate aether_proverProgram"), "{stderr}");
}

#[test]
fn an_old_validator_of_an_unpinned_chain_or_no_answer_is_a_failure() {
    let rpc = serve(METHOD_NOT_FOUND);
    let (ok, program, stderr) = ask(1, &rpc);
    assert!(!ok, "no pin for chain 1: unknown, not a guess ({program})");
    assert!(program.is_empty());
    assert!(stderr.contains("method not found"), "{stderr}");
    // Nobody listening: a failure too.
    let closed = Port::reserve().expect("reserve unreachable RPC port");
    let url = format!("http://{}", closed.addr());
    let (ok, program, _) = ask(7780, &url);
    assert!(!ok, "an unreachable network must not read as the 7780 pin ({program})");
}
