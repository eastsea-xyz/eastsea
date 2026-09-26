# S3 spike: an Aether block proved in Jolt

Needs the Jolt checkout used by `benches/spike/jolt.sh` at
`/Volumes/workspace/spike/jolt-akita` (a16z/jolt `feat/akita-metal`) and the
rustup toolchain on PATH (`export PATH=$HOME/.cargo/bin:$PATH`).

    cargo build --release
    ./target/release/aether-jolt-block analyze 1 10 50   # cycles, split by signature cost
    ./target/release/aether-jolt-block prove 1           # prove + verify, root checked vs native
    ./target/release/aether-jolt-block debug             # guest vs native roots, Poseidon2 probe

`patches/foldhash-0.2.0`: skips foldhash's clock-based seed on riscv64 (the
guest has no clock; upstream only exempts `target_os = "zkvm"`).
Results: `docs/research/spike-2026-10.md`.
