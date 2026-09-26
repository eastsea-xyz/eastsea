//! Red team against a running network (docs/design/12-launch-plan.md step 10).
//!
//! Ignored by default; point it at validators' RPC:
//!   AETHER_REDTEAM_RPC=http://127.0.0.1:8601,http://127.0.0.1:8602,... \
//!   cargo test -p aether-node --test redteam -- --ignored --nocapture
//! An attacker key gets test tokens from the faucet, then tries replays,
//! cross-chain and tampered signatures, underpriced and unpayable txs, value
//! overflow, mempool spam and faucet abuse. Every attack must be refused, the
//! attacker must never gain, and all validators must keep finalizing the same
//! chain.

use aether_crypto::{address_of, P256Signer, Signer as _};
use aether_execution::{sign_call_with, EvmCall};
use aether_types::{Address, Bytes, FeeVector, TxEnvelope, U256};
use serde_json::{json, Value};
use std::time::{Duration, Instant};

const GWEI: u128 = 1_000_000_000;

struct Net {
    rpcs: Vec<String>,
}

impl Net {
    fn call(&self, i: usize, method: &str, params: Value) -> Result<Value, String> {
        let body = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params });
        let v: Value = reqwest::blocking::Client::new()
            .post(&self.rpcs[i])
            .json(&body)
            .timeout(Duration::from_secs(10))
            .send()
            .map_err(|e| e.to_string())?
            .json()
            .map_err(|e| e.to_string())?;
        match v.get("error") {
            Some(e) => Err(e["message"].as_str().unwrap_or("error").to_string()),
            None => Ok(v.get("result").cloned().unwrap_or(Value::Null)),
        }
    }

    fn status(&self, i: usize) -> Value {
        self.call(i, "aether_status", json!([])).expect("status")
    }

    fn balance(&self, a: Address) -> U256 {
        let v = self.call(0, "eth_getBalance", json!([a])).expect("balance");
        U256::from_str_radix(v.as_str().unwrap_or("0x0").trim_start_matches("0x"), 16).unwrap_or_default()
    }

    fn nonce(&self, a: Address) -> u64 {
        let v = self.call(0, "eth_getTransactionCount", json!([a])).expect("nonce");
        u64::from_str_radix(v.as_str().unwrap_or("0x0").trim_start_matches("0x"), 16).unwrap_or_default()
    }

    fn send(&self, tx: &TxEnvelope) -> Result<String, String> {
        self.call(0, "aether_sendTransaction", json!([tx])).map(|v| v["hash"].as_str().unwrap_or_default().to_string())
    }

    fn wait_receipt(&self, hash: &str) -> Option<Value> {
        let end = Instant::now() + Duration::from_secs(30);
        while Instant::now() < end {
            if let Ok(r) = self.call(0, "aether_getReceipt", json!([hash])) {
                if r.get("receipt").is_some() {
                    return Some(r);
                }
            }
            std::thread::sleep(Duration::from_millis(300));
        }
        None
    }
}

fn transfer(to: Address, value: U256) -> EvmCall {
    EvmCall { to: Some(to), value, input: Bytes::new(), gas_limit: 21_000, delegate: None }
}

fn fees(net: &Net) -> FeeVector {
    let s = net.status(0);
    let base = |k: &str| s["base_fee"][k].as_str().and_then(|v| v.parse::<u128>().ok()).unwrap_or(GWEI);
    FeeVector { exec: base("exec") * 2 + GWEI, state: 0, prove: base("prove") * 2 }
}

#[test]
#[ignore]
fn attacks_on_a_live_network_are_refused() {
    let Ok(rpcs) = std::env::var("AETHER_REDTEAM_RPC") else {
        eprintln!("set AETHER_REDTEAM_RPC to validators' RPC URLs");
        return;
    };
    let net = Net { rpcs: rpcs.split(',').map(str::to_string).collect() };
    let chain_id = net.status(0)["chain_id"].as_u64().expect("chain id");
    let seed: [u8; 32] = rand::random();
    let attacker = P256Signer::from_seed(&seed).unwrap();
    let me = address_of(&attacker.public_key()).unwrap();
    let victim = Address::repeat_byte(0x71);
    let mut report = Vec::new();

    // Funds from the faucet (and the faucet refuses a second grant).
    let grant = net.call(0, "aether_faucet", json!([me])).expect("faucet grant");
    net.wait_receipt(grant["hash"].as_str().unwrap()).expect("grant finalized");
    let funded = net.balance(me);
    assert!(funded > U256::ZERO);
    assert!(net.call(0, "aether_faucet", json!([me])).is_err(), "faucet paid the same address twice");
    report.push("faucet: second grant refused");

    let caps = fees(&net);
    let sign = |nonce: u64, call: &EvmCall, caps: FeeVector, chain: u64| sign_call_with(&attacker, chain, nonce, caps, GWEI, call).unwrap();

    // One honest payment, then replays of it.
    let pay = sign(0, &transfer(victim, U256::from(1_000u64)), caps, chain_id);
    let h = net.send(&pay).expect("honest payment");
    assert_eq!(net.wait_receipt(&h).expect("finalized")["receipt"]["success"], json!(true));
    let _ = net.send(&pay);
    std::thread::sleep(Duration::from_secs(3));
    assert_eq!(net.balance(victim), U256::from(1_000u64), "a replay paid twice");
    report.push("replay: no double payment");

    let n = net.nonce(me);
    let attempts: Vec<(&str, TxEnvelope)> = vec![
        ("another chain id", sign(n, &transfer(victim, U256::from(1u64)), caps, chain_id + 1)),
        ("tampered signature", {
            let mut t = sign(n, &transfer(victim, U256::from(1u64)), caps, chain_id);
            let mut s = t.signature.to_vec();
            s[10] ^= 1;
            t.signature = Bytes::from(s);
            t
        }),
        ("tampered value after signing", {
            let mut t = sign(n, &transfer(victim, U256::from(1u64)), caps, chain_id);
            t.header.tip += 1;
            t
        }),
        ("zero prove budget", {
            let mut t = sign(n, &transfer(victim, U256::from(1u64)), caps, chain_id);
            t.header.gas.prove = 0;
            // Re-sign so only the budget is wrong.
            let payload = match &t.payload {
                aether_types::TxPayload::Plain(b) => b.clone(),
                _ => unreachable!(),
            };
            let mut h = t.header.clone();
            h.gas.prove = 0;
            let mut env = TxEnvelope { header: h, payload: aether_types::TxPayload::Plain(payload), signature: Bytes::new() };
            let mut s = attacker.sign(&env.signing_bytes()).unwrap();
            s.extend_from_slice(&attacker.public_key().bytes);
            env.signature = Bytes::from(s);
            env
        }),
        ("value overflow", sign(n, &transfer(victim, U256::MAX), caps, chain_id)),
        ("more than the balance", sign(n, &transfer(victim, funded * U256::from(2u64)), caps, chain_id)),
    ];
    // Under the base fee (only an attack while the network is congested; uncongested
    // transactions are free by design, R1′).
    let base = fees(&net);
    let mut attempts = attempts;
    if base.exec > GWEI {
        let under = FeeVector { exec: (base.exec - GWEI) / 4, state: 0, prove: base.prove / 4 };
        attempts.push(("below the base fee", sign(n, &transfer(victim, U256::from(1u64)), under, chain_id)));
    }
    for (what, tx) in &attempts {
        let r = net.send(tx);
        assert!(r.is_err(), "{what}: accepted ({r:?})");
        report.push(what);
    }

    // Mempool spam from one key: far-future nonces are capped per sender.
    let mut refused = 0;
    for k in 0..80u64 {
        let t = sign(n + 1_000 + k, &transfer(victim, U256::from(1u64)), caps, chain_id);
        if net.send(&t).is_err() {
            refused += 1;
        }
    }
    assert!(refused >= 16, "one sender filled the mempool ({refused} of 80 refused)");
    report.push("mempool spam: capped per sender");

    // The attacker gained nothing and the network kept finalizing one chain.
    assert!(net.balance(me) <= funded);
    std::thread::sleep(Duration::from_secs(3));
    let heights: Vec<u64> = (0..net.rpcs.len()).map(|i| net.status(i)["height"].as_u64().unwrap()).collect();
    let common = *heights.iter().min().unwrap();
    let hashes: Vec<Value> = (0..net.rpcs.len()).map(|i| net.call(i, "aether_getBlock", json!([common])).unwrap()["hash"].clone()).collect();
    assert!(hashes.windows(2).all(|w| w[0] == w[1]), "validators disagree at {common}");
    let later = net.status(0)["height"].as_u64().unwrap();
    std::thread::sleep(Duration::from_secs(3));
    assert!(net.status(0)["height"].as_u64().unwrap() > later, "the chain stopped finalizing");
    eprintln!("red team: {} attacks refused; validators agree at height {common}", report.len());
    for r in report {
        eprintln!("  ✓ {r}");
    }
}
