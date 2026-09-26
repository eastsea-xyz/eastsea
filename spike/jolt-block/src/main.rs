//! S3 spike: cycles (and, for small blocks, proofs) of Aether block execution in Jolt.
//!   cargo run --release -- analyze 1 10 100     cycle counts
//!   cargo run --release -- prove 1               prove + verify a 1-tx block

use aether_crypto::{P256Signer, Signer};
use aether_execution::{execute_block_sequential, sign_call, BlockContext, EvmCall, WorldState};
use aether_types::{Address, Bytes, GasVector, U256};
use guest::Witness;
use jolt_sdk as jolt;
use jolt_inlines_p256 as _; // link the inline registrations
use std::time::Instant;

const CHAIN: u64 = 7777;

fn witness(n: usize) -> (Vec<u8>, [u8; 32]) {
    let signers: Vec<P256Signer> = (0..n)
        .map(|i| {
            let mut s = [0u8; 32];
            s[0] = 0x11;
            s[30] = (i >> 8) as u8;
            s[31] = i as u8 + 1;
            P256Signer::from_seed(&s).unwrap()
        })
        .collect();
    let mut pre = WorldState::default();
    for s in &signers {
        pre.set_balance(aether_crypto::address_of(&s.public_key()).unwrap(), U256::from(10u128.pow(21))).unwrap();
    }
    let entries: Vec<([u8; 32], [u8; 32])> = pre.journal().writes.iter().map(|(k, v)| (*k, v.expect("set"))).collect();
    let txs: Vec<_> = signers
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let call = EvmCall { to: Some(Address::repeat_byte(0x40 + (i % 100) as u8)), value: U256::from(1000u64), input: Bytes::new(), gas_limit: 21_000, delegate: None };
            sign_call(s, CHAIN, 0, 1, &call).unwrap()
        })
        .collect();
    let beneficiary = [0xbe; 20];
    let ctx = BlockContext { chain_id: CHAIN, number: 1, timestamp: 1, beneficiary: Address::from(beneficiary), limits: GasVector { exec: 30_000_000, state: u64::MAX, prove: u64::MAX } };
    let native = execute_block_sequential(&WorldState::from_parts(entries.clone(), Default::default()), &ctx, &txs).unwrap().state.root().0;
    let w = Witness { entries, chain_id: CHAIN, number: 1, timestamp: 1, beneficiary, txs };
    (serde_json::to_vec(&w).unwrap(), native)
}

fn main() {
    tracing_subscriber::fmt().with_env_filter("warn").init();
    let args: Vec<String> = std::env::args().collect();
    let target_dir = "/tmp/jolt-guest-targets";
    match args.get(1).map(String::as_str) {
        Some("analyze") => {
            for n in args[2..].iter().map(|s| s.parse::<usize>().unwrap()) {
                let (w, native) = witness(n);
                let t = Instant::now();
                let summary = guest::analyze_prove_block(w.clone());
                let cycles = summary.trace_len();
                let root_ok = summary.io_device.outputs.windows(32).any(|x| x == native);
                drop(summary);
                let tree = guest::analyze_pre_root(w.clone()).trace_len();
                let sig = guest::analyze_verify_signatures(w).trace_len();
                println!("  parse+pre-state tree (2 accounts/tx): {tree} cycles ({} per tx)", tree / n);
                println!(
                    "block {n:>4} tx: {cycles:>11} cycles total ({:>8} per tx), signatures {sig:>11} ({:>8} per tx, {:.0}%), root matches native: {root_ok}  [{:.1}s]",
                    cycles / n,
                    sig / n,
                    100.0 * sig as f64 / cycles as f64,
                    t.elapsed().as_secs_f64()
                );
            }
        }
        Some("debug") => {
            let (w, native) = witness(1);
            let pw: Witness = serde_json::from_slice(&w).unwrap();
            let native_pre = WorldState::from_parts(pw.entries, Default::default()).root().0;
            let out = |s: jolt::host::analyze::ProgramSummary| s.io_device.outputs.clone();
            println!("native pre  {}", hex(&native_pre));
            println!("guest  pre  {}", hex(&out(guest::analyze_pre_root(w.clone()))));
            println!("native post {}", hex(&native));
            let s = guest::analyze_prove_block(w.clone());
            println!("guest  post {} (panic={})", hex(&s.io_device.outputs), s.io_device.panic);
            let e = guest::analyze_block_error(w);
            println!("guest  error: {}", String::from_utf8_lossy(&e.io_device.outputs));
            let x = [7u8; 32];
            use aether_hash::Hasher;
            println!("native H(7) {}", hex(&aether_execution::ChainHasher::new().hash_bytes(&x)));
            println!("guest  H(7) {}", hex(&out(guest::analyze_hash_probe(x))));
        }
        Some("profile") => {
            // Per-instruction-address cycle counts of the (symbol-carrying) block_error guest.
            let n: usize = args[2].parse().unwrap();
            let (w, _) = witness(n);
            let summary = guest::analyze_block_error(w);
            let mut counts: std::collections::HashMap<u64, u64> = Default::default();
            for row in &summary.trace {
                *counts.entry(row.address()).or_default() += 1;
            }
            let mut out = String::new();
            for (a, c) in counts {
                out.push_str(&format!("{a:x} {c}\n"));
            }
            std::fs::write("pc_counts.txt", out).unwrap();
            println!("{} cycles, {} distinct pcs -> pc_counts.txt", summary.trace_len(), summary.trace.len());
        }
        Some("prove") => {
            let n: usize = args[2].parse().unwrap();
            let (w, native) = witness(n);
            let mut program = guest::compile_prove_block(target_dir);
            let shared = guest::preprocess_shared_prove_block(&mut program).unwrap();
            let prover_pp = guest::preprocess_prover_prove_block(shared.clone());
            let verifier_pp = guest::verifier_preprocessing_from_prover_prove_block(&prover_pp);
            let prove = guest::build_prover_prove_block(program, prover_pp);
            let verify = guest::build_verifier_prove_block(verifier_pp);
            let t = Instant::now();
            let (root, proof, io) = prove(w.clone());
            let prove_s = t.elapsed().as_secs_f64();
            let t = Instant::now();
            let ok = verify(w, root, io.panic, proof);
            println!("block {n} tx: proved in {prove_s:.1}s, verified={ok} in {:.0}ms, root matches native: {}", t.elapsed().as_secs_f64() * 1e3, root == native);
        }
        _ => eprintln!("usage: analyze N... | prove N"),
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}
