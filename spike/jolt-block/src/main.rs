//! S3 spike: cycles (and, for small blocks, proofs) of Aether block execution in Jolt.
//!   cargo run --release -- analyze 1 10 100     cycle counts
//!   cargo run --release -- prove 1               prove + verify a 1-tx block

use aether_crypto::{P256Signer, Signer};
use aether_execution::{execute_block_sequential, sign_call, BlockContext, EvmCall, WorldState};
use aether_types::{Address, Bytes, GasVector, U256};
use jolt_sdk as jolt;
use jolt_inlines_blake3 as _; // link the inline registrations (state hash)
use jolt_inlines_p256 as _; // link the inline registrations
use std::time::Instant;

const CHAIN: u64 = 7777;

/// A block of `n` payments on a state with other accounts too: the prover's
/// input (stateless witness) and the statement commitment native execution gives.
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
    for i in 0..1000u32 {
        let mut a = [0u8; 20];
        a[..4].copy_from_slice(&i.to_be_bytes());
        a[19] = 0xee;
        pre.set_balance(Address::from(a), U256::from(1u64)).unwrap();
    }
    let txs: Vec<_> = signers
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let call = EvmCall { to: Some(Address::repeat_byte(0x40 + (i % 100) as u8)), value: U256::from(1000u64), input: Bytes::new(), gas_limit: 21_000, delegate: None };
            sign_call(s, CHAIN, 0, 1, &call).unwrap()
        })
        .collect();
    let ctx = BlockContext { chain_id: CHAIN, number: 1, timestamp: 1, beneficiary: Address::repeat_byte(0xbe), limits: GasVector { exec: 30_000_000, state: u64::MAX, prove: u64::MAX }, fees: None };
    let input = aether_proving::block::input(&pre, &ctx, &txs, &[]).unwrap();
    let statement = aether_proving::block::execute(&input).unwrap();
    assert_eq!(statement.post_state_root, execute_block_sequential(&pre, &ctx, &txs).unwrap().state.root());
    let bytes = postcard::to_allocvec(&input).unwrap();
    eprintln!("input: {} bytes ({} stems, {} opaque nodes)", bytes.len(), input.witness.tree.stems.len(), input.witness.tree.opaque.len());
    (bytes, statement.commitment())
}

fn main() {
    // RUST_LOG=jolt_prover=info (etc.) prints span timings on close.
    let filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into());
    tracing_subscriber::fmt().with_env_filter(filter).with_span_events(tracing_subscriber::fmt::format::FmtSpan::CLOSE).init();
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
                println!("  witness -> pre-state root: {tree} cycles ({} per tx)", tree / n);
                println!(
                    "block {n:>4} tx: {cycles:>11} cycles total ({:>8} per tx), signatures {sig:>11} ({:>8} per tx, {:.0}%), statement matches native: {root_ok}  [{:.1}s]",
                    cycles / n,
                    sig / n,
                    100.0 * sig as f64 / cycles as f64,
                    t.elapsed().as_secs_f64()
                );
            }
        }
        Some("debug") => {
            let (w, native) = witness(1);
            println!("native statement {}", hex(&native));
            let s = guest::analyze_prove_block(w.clone());
            println!("guest  statement {} (panic={})", hex(&s.io_device.outputs), s.io_device.panic);
            let e = guest::analyze_block_error(w);
            println!("guest  error: {}", String::from_utf8_lossy(&e.io_device.outputs));
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
            let t = Instant::now();
            let shared = guest::preprocess_shared_prove_block(&mut program).unwrap();
            let prover_pp = guest::preprocess_prover_prove_block(shared.clone());
            let verifier_pp = guest::verifier_preprocessing_from_prover_prove_block(&prover_pp);
            let prove = guest::build_prover_prove_block(program, prover_pp);
            let verify = guest::build_verifier_prove_block(verifier_pp);
            println!("preprocessed in {:.1}s", t.elapsed().as_secs_f64());
            // `prove N R`: R proofs in one process; later runs reuse warm setup/GPU state.
            let reps: usize = args.get(3).map_or(1, |r| r.parse().unwrap());
            for rep in 0..reps {
                let t = Instant::now();
                let (root, proof, io) = prove(w.clone());
                let prove_s = t.elapsed().as_secs_f64();
                let proof_bytes = jolt::serialize_verifier_object(&proof).map(|b| b.len()).unwrap_or(0);
                println!("proof size: {:.1} kB, padded trace 2^{}", proof_bytes as f64 / 1024.0, proof.trace_length.ilog2());
                let t = Instant::now();
                let ok = verify(w.clone(), root, io.panic, proof);
                println!("block {n} tx (run {rep}): proved in {prove_s:.1}s, verified={ok} in {:.0}ms, statement matches native: {}", t.elapsed().as_secs_f64() * 1e3, root == native);
            }
        }
        _ => eprintln!("usage: analyze N... | prove N [reps]"),
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}
