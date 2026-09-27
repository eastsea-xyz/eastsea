//! aether-prover: the block-proving sidecar the node spawns.
//!
//!   aether-prover prove <input.postcard> <proof.out>
//!       stdout: {"commitment":"<hex>","proof_bytes":N,"seconds":S}
//!   aether-prover verify <proof> <commitment-hex>
//!       exit 0 only if the proof verifies with that commitment as output
//!   aether-prover serve
//!       one JSON request per stdin line, one JSON reply per stdout line; keeps
//!       the per-shape Akita setups warm across requests (see README)
//!   aether-prover self-test [n]
//!       prove + verify a sample n-tx block, and check that bad proofs are rejected
//!   aether-prover sample-input <n> <out.postcard>
//!       write the self-test's n-tx BlockInput (for testing prove/verify)
//!   aether-prover info
//!       the embedded guest ELF's SHA-256 and size
//!
//! Exit codes: 0 ok, 1 failure / rejected, 2 usage. Diagnostics go to stderr;
//! stdout carries exactly one JSON line per command (per request in `serve`).

#[cfg(not(feature = "akita"))]
compile_error!("aether-prover needs the `akita` (CPU) or `metal` (Apple GPU) feature");

mod engine;
mod program;
mod sample;

use std::io::BufRead;
use std::process::ExitCode;

use engine::{hex, Engine};
use jolt_inlines_blake3 as _; // link the inline registrations (state hash)
use jolt_inlines_p256 as _; // link the inline registrations (signatures)
use program::BoxError;
use serde_json::{json, Value};

const USAGE: &str = "usage: aether-prover prove <input.postcard> <proof.out>
       aether-prover verify <proof> <commitment-hex>
       aether-prover serve
       aether-prover self-test [n]
       aether-prover sample-input <n> <out.postcard>
       aether-prover info";

fn main() -> ExitCode {
    // RUST_LOG=jolt_prover=info (etc.) prints span timings on close.
    let filter =
        tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into());
    tracing_subscriber::fmt().with_env_filter(filter).with_writer(std::io::stderr).init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let positive = |n: &str| n.parse::<usize>().ok().filter(|&n| n > 0);
    let result = match args.as_slice() {
        ["prove", input, out] => Engine::new().and_then(|e| e.prove(input, out)),
        ["verify", proof, commitment] => Engine::new().and_then(|e| e.verify(proof, commitment)),
        ["serve"] => serve(),
        ["self-test"] => Engine::new().and_then(|e| e.self_test(10)),
        ["self-test", n] => match positive(n) {
            Some(n) => Engine::new().and_then(|e| e.self_test(n)),
            None => return usage(),
        },
        ["sample-input", n, out] => match positive(n) {
            Some(n) => sample_input(n, out),
            None => return usage(),
        },
        ["info"] => Ok(engine::info()),
        _ => return usage(),
    };
    match result {
        Ok(Value::Null) => ExitCode::SUCCESS,
        Ok(reply) => {
            println!("{reply}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("aether-prover: {e}");
            ExitCode::FAILURE
        }
    }
}

fn usage() -> ExitCode {
    eprintln!("{USAGE}");
    ExitCode::from(2)
}

fn sample_input(n: usize, out: &str) -> Result<Value, BoxError> {
    let (input, commitment) = sample::block(n)?;
    engine::write_atomic(out, &input)?;
    Ok(json!({ "commitment": hex(&commitment), "input_bytes": input.len() }))
}

/// Line-delimited JSON over stdin/stdout. Requests:
///   {"cmd":"prove","input":"<path>","out":"<path>"}
///   {"cmd":"verify","proof":"<path>","commitment":"<hex>"}
///   {"cmd":"info"}
/// Reply: the CLI's JSON plus "ok":true, or {"ok":false,"error":"..."}.
/// Requests run one at a time; EOF on stdin ends the process (exit 0).
fn serve() -> Result<Value, BoxError> {
    let engine = Engine::new()?;
    println!("{}", with_ok(engine::info()));
    for line in std::io::stdin().lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        // One bad request never takes the long-lived sidecar down.
        let reply = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| handle(&engine, &line))) {
            Ok(Ok(v)) => with_ok(v),
            Ok(Err(e)) => json!({ "ok": false, "error": e.to_string() }),
            Err(_) => json!({ "ok": false, "error": "request panicked" }),
        };
        println!("{reply}");
    }
    Ok(Value::Null)
}

fn handle(engine: &Engine, line: &str) -> Result<Value, BoxError> {
    let req: Value = serde_json::from_str(line)?;
    let field = |k: &str| -> Result<&str, BoxError> {
        req.get(k).and_then(Value::as_str).ok_or_else(|| format!("missing string field {k:?}").into())
    };
    match field("cmd")? {
        "prove" => engine.prove(field("input")?, field("out")?),
        "verify" => engine.verify(field("proof")?, field("commitment")?),
        "info" => Ok(engine::info()),
        other => Err(format!("unknown cmd {other:?}").into()),
    }
}

fn with_ok(mut v: Value) -> Value {
    if let Value::Object(map) = &mut v {
        map.insert("ok".into(), Value::Bool(true));
    }
    v
}
