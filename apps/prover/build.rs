//! Embed the prove_block guest ELF: build it with build-guest.sh (jolt CLI,
//! riscv64 toolchain) into OUT_DIR, or copy a prebuilt one from
//! AETHER_PROVER_GUEST_ELF. Only building aether-prover needs the toolchains;
//! the binary itself never compiles the guest.

use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR")).join("prove_block.elf");
    println!("cargo:rerun-if-env-changed=AETHER_PROVER_GUEST_ELF");

    if let Some(prebuilt) = std::env::var_os("AETHER_PROVER_GUEST_ELF") {
        let prebuilt = PathBuf::from(prebuilt);
        println!("cargo:rerun-if-changed={}", prebuilt.display());
        std::fs::copy(&prebuilt, &out)
            .unwrap_or_else(|e| panic!("copy AETHER_PROVER_GUEST_ELF {}: {e}", prebuilt.display()));
        return;
    }

    // Rebuild when the guest or any Aether crate it compiles changes.
    for p in ["build-guest.sh", "guest", "Cargo.lock", "rust-toolchain.toml", "patches", "../../crates"] {
        println!("cargo:rerun-if-changed={}", manifest.join(p).display());
    }
    let status = Command::new(manifest.join("build-guest.sh"))
        .arg(&out)
        .current_dir(&manifest)
        .status()
        .expect("run build-guest.sh (needs bash, the jolt CLI and rustup on PATH)");
    assert!(status.success(), "build-guest.sh failed; see the output above");
    assert!(Path::new(&out).exists(), "build-guest.sh did not write {}", out.display());
}
