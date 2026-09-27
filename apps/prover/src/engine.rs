//! The sidecar's operations over one program preprocessing. The preprocessing
//! carries jolt-sdk's per-shape Akita setup cache (shared by clones), so a
//! long-lived `Engine` (`serve`) pays each shape's setup once; a one-shot CLI
//! call pays it every time.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::time::Instant;

use jolt_sdk as jolt;
use serde_json::{json, Value};

use crate::program::{self, BoxError};
use crate::sample;

pub struct Engine {
    pp: jolt::JoltProverPreprocessing,
}

impl Engine {
    pub fn new() -> Result<Self, BoxError> {
        let t = Instant::now();
        let pp = guarded(program::preprocessing)?;
        eprintln!("aether-prover: program preprocessed in {:.2}s", t.elapsed().as_secs_f64());
        Ok(Self { pp })
    }

    /// Prove the postcard `BlockInput` at `input_path`; write the proof to `out_path`.
    pub fn prove(&self, input_path: &str, out_path: &str) -> Result<Value, BoxError> {
        let t = Instant::now();
        let input = std::fs::read(input_path).map_err(|e| format!("read {input_path}: {e}"))?;
        // Native pre-check: reject a bad input in milliseconds instead of after
        // a full proof, and cross-check the guest's answer afterwards.
        let native = native_commitment(&input)?;
        let proved = guarded(|| program::prove(&self.pp, input))?;
        if proved.commitment != native {
            return Err(format!(
                "guest commitment {} differs from native execution {}",
                hex(&proved.commitment),
                hex(&native)
            )
            .into());
        }
        write_atomic(out_path, &proved.proof_bytes)?;
        eprintln!(
            "aether-prover: proved in {:.1}s (trace 2^{}, ram_K 2^{})",
            proved.prove_seconds,
            proved.proof.trace_length.ilog2(),
            proved.proof.ram_K.ilog2()
        );
        Ok(json!({
            "commitment": hex(&proved.commitment),
            "proof_bytes": proved.proof_bytes.len(),
            "seconds": secs(t),
        }))
    }

    /// Verify the proof at `proof_path` against `commitment_hex`.
    pub fn verify(&self, proof_path: &str, commitment_hex: &str) -> Result<Value, BoxError> {
        let commitment = parse_hex32(commitment_hex)?;
        let proof = std::fs::read(proof_path).map_err(|e| format!("read {proof_path}: {e}"))?;
        let t = Instant::now();
        guarded(|| program::verify(self.pp.clone(), &proof, commitment))?;
        Ok(json!({ "verified": true, "seconds": secs(t) }))
    }

    /// Prove a sample block, verify it with a fresh preprocessing (as a
    /// separate `verify` process would), then again warm, and check that a
    /// wrong commitment, a corrupted proof and a truncated proof are rejected.
    pub fn self_test(&self, n: usize) -> Result<Value, BoxError> {
        let (input, native) = sample::block(n)?;
        eprintln!("self-test: {n}-tx block, input {} bytes, commitment {}", input.len(), hex(&native));
        let proved = guarded(|| program::prove(&self.pp, input))?;
        if proved.commitment != native {
            return Err("guest commitment differs from native execution".into());
        }
        let bytes = &proved.proof_bytes;

        let t = Instant::now();
        let cold = Engine::new()?;
        guarded(|| program::verify(cold.pp.clone(), bytes, native))?;
        let verify_cold = secs(t);
        let t = Instant::now();
        guarded(|| program::verify(cold.pp.clone(), bytes, native))?;
        let verify_warm = secs(t);

        let mut wrong = native;
        wrong[0] ^= 1;
        let mut corrupted = bytes.clone();
        corrupted[bytes.len() / 2] ^= 0x55;
        let rejects = [
            ("wrong commitment", guarded(|| program::verify(cold.pp.clone(), bytes, wrong))),
            ("corrupted proof", guarded(|| program::verify(cold.pp.clone(), &corrupted, native))),
            ("truncated proof", guarded(|| program::verify(cold.pp.clone(), &bytes[..bytes.len() - 1], native))),
        ];
        for (what, result) in rejects {
            match result {
                Ok(()) => return Err(format!("a {what} was accepted").into()),
                Err(e) => eprintln!("self-test: {what} rejected ({e})"),
            }
        }
        Ok(json!({
            "self_test": "passed",
            "txs": n,
            "commitment": hex(&native),
            "proof_bytes": bytes.len(),
            "trace_log2": proved.proof.trace_length.ilog2(),
            "prove_seconds": proved.prove_seconds,
            "verify_cold_seconds": verify_cold,
            "verify_warm_seconds": verify_warm,
        }))
    }
}

pub fn info() -> Value {
    json!({
        "guest_elf_sha256": hex(&program::elf_sha256()),
        "guest_elf_bytes": program::GUEST_ELF.len(),
        "backend": if cfg!(feature = "metal") { "akita-metal" } else { "akita-cpu" },
    })
}

/// Run `f`, turning a panic inside Jolt into an error (a malformed proof or
/// input must not take down `serve`).
fn guarded<T>(f: impl FnOnce() -> Result<T, BoxError>) -> Result<T, BoxError> {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or_else(|panic| {
        let msg = panic
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| panic.downcast_ref::<&str>().copied())
            .unwrap_or("unknown panic");
        Err(format!("panicked: {msg}").into())
    })
}

fn native_commitment(input: &[u8]) -> Result<[u8; 32], BoxError> {
    let input: aether_proving::block::BlockInput = postcard::from_bytes(input)
        .map_err(|e| format!("input is not a postcard BlockInput: {e}"))?;
    let statement = aether_proving::block::execute(&input)
        .map_err(|e| format!("block does not execute: {e:?}"))?;
    Ok(statement.commitment())
}

pub fn write_atomic(path: &str, bytes: &[u8]) -> Result<(), BoxError> {
    let tmp = format!("{path}.tmp");
    std::fs::write(&tmp, bytes).map_err(|e| format!("write {tmp}: {e}"))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("rename {tmp} -> {path}: {e}"))?;
    Ok(())
}

fn secs(t: Instant) -> f64 {
    (t.elapsed().as_secs_f64() * 1000.0).round() / 1000.0
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn parse_hex32(s: &str) -> Result<[u8; 32], BoxError> {
    let s = s.strip_prefix("0x").unwrap_or(s);
    if s.len() != 64 || !s.is_ascii() {
        return Err("commitment must be 32 bytes of hex".into());
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&s[2 * i..2 * i + 2], 16)
            .map_err(|_| "commitment must be 32 bytes of hex")?;
    }
    Ok(out)
}
