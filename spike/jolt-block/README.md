# S3 spike: an Aether block proved in Jolt

Needs the Jolt checkout at `/Volumes/workspace/aether-jolt/jolt` (a16z/jolt
`feat/akita-metal`, branch `aether`, whose jolt-sdk has the `akita`/`metal`
features) with our akita fork next to it at `/Volumes/workspace/aether-jolt/akita`,
and the rustup toolchain on PATH (`export PATH=$HOME/.cargo/bin:$PATH`).

    cargo build --release
    ./target/release/aether-jolt-block analyze 1 10 50   # cycles, split by signature cost
    ./target/release/aether-jolt-block prove 1           # prove + verify, root checked vs native
    ./target/release/aether-jolt-block debug             # guest vs native roots, Poseidon2 probe

PCS / backend (Dory is the default):

    cargo build --release --features akita --target-dir target-akita   # Akita, CPU
    cargo build --release --features metal --target-dir target-metal   # Akita, Metal
    ./target-metal/release/aether-jolt-block prove 10 [reps]

The Metal PIOP now carries the pc in its own column (our jolt branch), so
`metal` runs the full Metal backend for this guest (2^21-entry bytecode);
`JOLT_AKITA_METAL=full|hybrid|commit|cpu` overrides the choice.

`patches/foldhash-0.2.0`: skips foldhash's clock-based seed on riscv64 (the
guest has no clock; upstream only exempts `target_os = "zkvm"`).
Results: `docs/research/spike-2026-10.md`.
