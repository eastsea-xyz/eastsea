//! The embedded prove_block guest and the prove / verify calls on it.
//!
//! The guest ELF is compiled ahead of time (build-guest.sh, run by build.rs) and
//! embedded with `include_bytes!`. `EmbeddedGuest` hands it to jolt-sdk through
//! the `JoltProgramSource` trait, so nothing is compiled at runtime and the
//! verifier's preprocessing is a pure function of these bytes (plus the
//! `#[jolt::provable]` memory configuration compiled into the guest crate).

use std::error::Error;
use std::time::Instant;

use jolt_sdk as jolt;
use jolt_sdk::host::JoltProgramSource;
use sha2::{Digest, Sha256};

pub type BoxError = Box<dyn Error + Send + Sync>;

/// The riscv64 ELF of `guest::prove_block`.
pub static GUEST_ELF: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/prove_block.elf"));

pub type Proof = jolt::RV64IMACProof;

/// SHA-256 of the embedded ELF: identifies the program the proofs are about.
pub fn elf_sha256() -> [u8; 32] {
    Sha256::digest(GUEST_ELF).into()
}

/// A `JoltProgramSource` over the embedded ELF (no compute-advice build: the
/// guest does not use advice-tape hints).
pub struct EmbeddedGuest;

impl JoltProgramSource for EmbeddedGuest {
    fn get_elf_contents(&self) -> Option<Vec<u8>> {
        Some(GUEST_ELF.to_vec())
    }

    fn get_elf_compute_advice_contents(&self) -> Option<Vec<u8>> {
        None
    }
}

/// Program preprocessing (decoded bytecode, memory image and layout) derived
/// from the embedded ELF. Deterministic; shared by prover and verifier.
pub fn preprocessing() -> Result<jolt::JoltProverPreprocessing, BoxError> {
    let shared = guest::preprocess_shared_prove_block(&mut EmbeddedGuest)?;
    Ok(jolt::prover_preprocessing_from_shared(shared))
}

pub struct Proved {
    pub commitment: [u8; 32],
    pub proof: Proof,
    pub proof_bytes: Vec<u8>,
    pub prove_seconds: f64,
}

/// Prove `prove_block` on a postcard `BlockInput`. The input goes in as
/// untrusted advice, so the proof is checkable from the commitment alone.
pub fn prove(
    preprocessing: &jolt::JoltProverPreprocessing,
    input: Vec<u8>,
) -> Result<Proved, BoxError> {
    let advice = jolt::postcard::to_stdvec(&jolt::UntrustedAdvice::new(input))?;
    let layout = jolt::prover_memory_layout(preprocessing);
    if advice.len() as u64 > layout.max_untrusted_advice_size {
        return Err(format!(
            "input is {} bytes, the guest accepts at most {}",
            advice.len(),
            layout.max_untrusted_advice_size
        )
        .into());
    }
    let t = Instant::now();
    let advice_tape = jolt::compute_advice_tape(&EmbeddedGuest, &[], &advice, &[], layout)?;
    let (proof, io) =
        jolt::prove_program(&EmbeddedGuest, preprocessing, &[], &advice, &[], None, None, advice_tape)?;
    let prove_seconds = t.elapsed().as_secs_f64();
    if io.panic {
        return Err("the guest panicked (invalid block input)".into());
    }
    let commitment: [u8; 32] = io
        .outputs
        .get(..32)
        .and_then(|b| b.try_into().ok())
        .ok_or("the guest produced no commitment")?;
    let proof_bytes = jolt::serialize_verifier_object(&proof)?;
    Ok(Proved { commitment, proof, proof_bytes, prove_seconds })
}

/// Verify a serialized proof against the expected commitment. `Ok(())` only if
/// the proof decodes, verifies for the embedded program, says the guest did
/// not panic, and its public output is exactly `commitment`.
pub fn verify(
    preprocessing: jolt::JoltVerifierPreprocessing,
    proof_bytes: &[u8],
    commitment: [u8; 32],
) -> Result<(), BoxError> {
    let proof: Proof = jolt::deserialize_verifier_object(proof_bytes)?;
    check_shape(&preprocessing, &proof)?;
    // The verifier rebuilds the public I/O: no public inputs, output =
    // postcard([u8; 32]) = the 32 bytes, panic = false.
    let check = guest::build_verifier_prove_block(preprocessing);
    if check(commitment, false, proof) {
        Ok(())
    } else {
        Err("proof rejected".into())
    }
}

/// Bound the proof-declared shape before the verifier derives an Akita setup
/// for it: the setup cost grows with the shape, so an unchecked `ram_K` would
/// let a malformed proof make the verifier build an arbitrarily large setup.
/// (`trace_length` is bounded by jolt-sdk's `verify_program`.)
fn check_shape(pp: &jolt::JoltVerifierPreprocessing, proof: &Proof) -> Result<(), BoxError> {
    let layout = jolt::verifier_memory_layout(pp);
    let words = (layout.heap_end - layout.get_lowest_address()) / 8 + 1;
    let max_ram_k = words.next_power_of_two() as usize;
    if !proof.ram_K.is_power_of_two() || proof.ram_K > max_ram_k {
        return Err(format!("proof declares ram_K {} (at most {max_ram_k})", proof.ram_K).into());
    }
    Ok(())
}
