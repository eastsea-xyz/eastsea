# Development loop

This loop is for development and tests. Release profiles, `release-mac.sh`, the
wallet release build, Jolt guest inputs and the guest build remain unchanged.
Run from the repository/worktree root. Every generated scratch file stays under
that root's `tmp/`; Rust artifacts and sccache live on the external workspace SSD.

## Setup and common commands

The repository pins Rust 1.98.1. Prefer rustup over Homebrew's Rust on PATH.
Install the official nextest binary without consuming a Rust compile slot:

```bash
export PATH="$HOME/.cargo/bin:$PATH"
mkdir -p tmp/dev-tools "$HOME/.cargo/bin"
curl -fLsS https://get.nexte.st/0.9/mac -o tmp/dev-tools/nextest.tar.gz
tar -xzf tmp/dev-tools/nextest.tar.gz -C tmp/dev-tools
cp tmp/dev-tools/cargo-nextest "$HOME/.cargo/bin/cargo-nextest"
cargo nextest --version
sccache --version

scripts/test-affected.sh --base lead-merge --list
scripts/test-affected.sh --base lead-merge
scripts/run-rust-tests.sh -- -p aether-hash -p aether-crypto
scripts/test-swift-pure.sh
scripts/test-swift-pure.sh earnings token-send
scripts/test-devnet.sh
scripts/test-devnet.sh 'binary(catchup_devnet)'
```

`test-affected.sh` unions committed changes since the base, staged changes,
unstaged changes and untracked files. It follows reverse workspace dependencies,
so changing a library tests its consumers too. Workspace manifests, lockfiles,
toolchain/build configuration and test orchestration changes invalidate the
selection broadly. Documentation-only changes skip Rust compilation. Use
`--dry-run` to inspect the command. `aether-ffi` is explicitly left to the lead's
staticlib gate, which must run without sccache. Apps/prover and guest workspaces
are excluded. Affected selection supplements the whole-tree integration gate.

`run-rust-tests.sh` builds with nextest once, then runs with the saved binary and
Cargo metadata. Its content cache reuses those build records when sources,
compiler, flags and build options match and the binaries still exist unchanged.
Every invocation executes tests again. A warm run takes no compile slot. Source
edits, replaced/pruned binaries and compiler changes require a gated rebuild.

Compilation uses the existing `wait-compile.sh` counting semaphore, in the same
shell as the build, and four Cargo build jobs on this Mac. The gate owner lives
through the build. A 1,200-second **queue** timeout exits 75; commit/report and let
the lead run remaining gates when that limit is reached. Builds keep inherited
`RUSTC_WRAPPER`, or discover sccache if it was absent. `dev-cargo.sh` requires
explicit packages and rejects release, alternate Cargo profiles/manifests,
whole-workspace and staticlib/guest entry points.

Nextest runs four tests concurrently. Node integration tests share a one-thread
group because process-per-test execution cannot reuse in-process locks. Hung
tests terminate after ten minutes; node scenarios have a thirty-minute ceiling.
No blanket retries hide failures. Add a named retry override only after a flaky
test is identified from evidence; existing bind-only startup retries remain in
the test support crate. Nextest does not run doctests; the lead's complete Cargo
gate remains necessary.

## Shared Rust targets

`dev-cargo.sh` chooses one target under
`/Volumes/workspace/build-cache/targets/aether-<base-family>-<compiler-profile-key>`.
The family is the merge-base with `lead-merge`, or `AETHER_BUILD_BASE`. Compiler,
dev/test profiles and relevant compilation flags distinguish incompatible
families. New worktrees reuse dependency artifacts; Cargo still validates their
own source paths. Cargo serializes concurrent writes to the same target.

```bash
PATH="$HOME/.cargo/bin:$PATH" python3 scripts/build-cache.py path
scripts/clean-build-cache.sh --dry-run
scripts/clean-build-cache.sh --max-gib 64
```

The default aggregate target cap is 64 GiB (`AETHER_BUILD_CACHE_GIB` overrides it).
Cleanup removes least-recently-used **idle, managed** directories only, retaining
active build/test leases, unmanaged directories and symlinks. Wrappers check at
most hourly to avoid repeatedly walking huge target trees; explicit cleanup
checks immediately. Active families can temporarily exceed the cap. This is
separate from the existing sccache cap. `CARGO_TARGET_DIR` overrides are honored.
Use `AETHER_BUILD_CACHE_ROOT` to choose another external-volume cache root.

## Swift pure tests

The existing source table and all 52 tests remain. Cache misses compile in a
four-worker batch under one compile slot; tests execute sequentially. Cached
executables use hashes of all source/test bytes, compiler/version, flags,
orchestration scripts and SDK-related environment. Test results are never cached.
Migration tests keep `-O -assert-config Debug`; native identity fixtures still
create freshly signed C helpers and exercise their owned processes.

Artifacts live in `tmp/swift-test-cache` and `tmp/swift-module-cache`. A targeted
test name avoids compiling unrelated Swift test targets. Editing a shared source
correctly invalidates every test that uses it. Changing the runner itself can
invalidate the whole batch.

## RAM-backed integration storage

`test-devnet.sh` and affected node tests use `test-tmpdir.sh` for runtime only;
compiler scratch and build caches stay on the SSD. The helper creates a macOS RAM
device sized to a quarter of immediately free/speculative memory, bounded to
128–512 MiB, and mounts it under this worktree's `tmp/`. Linux uses verified tmpfs
at `/dev/shm`. Insufficient memory or failed mounting stops the run; there is no
internal-disk fallback.

```bash
scripts/run-rust-tests.sh --ram -- -p aether-node --test devnet
scripts/test-tmpdir.sh python3 -c 'import tempfile; print(tempfile.gettempdir())'
```

To reuse an already mounted RAM filesystem, set `AETHER_TEST_TMPDIR`; the helper
verifies the filesystem rather than trusting its path. Normal exits, failures and
signals terminate the invocation's descendants before deleting temporary data
and detaching its owned device. Disk utilities have thirty-second timeouts and
bounded cleanup. Persistent daemon/key operations in `scripts/devnet.sh` retain
their separate lifecycle; use the test commands above for test scenarios.

## Linux offload

Only `poc-cuda` is authorized. SSH and system `/usr/bin/rsync` are bound to its
Tailscale address `100.121.197.74`, retaining the alias's user/key settings. A
read-only Linux/storage preflight precedes writes. Neither poc-m3, poc-nas nor
arbitrary hosts are accepted.

```bash
scripts/remote-test.sh --dry-run -p aether-crypto
scripts/remote-test.sh --setup
scripts/remote-test.sh -p aether-hash -p aether-crypto
```

Setup checks/installs user-local rustup, nextest and sccache, using official
installers and snapshot-owned compiler temporary files. The source-only snapshot
includes uncommitted Rust changes while excluding credentials, private keys,
symlinks, app data, `.git`, guest projects, targets and temporary files. It lives
under `/mnt/ssd1/aether-dev/lanes/<unique-name>`. Targets share a flat family key
under `/mnt/ssd1/aether-dev/targets`, sccache uses `/mnt/ssd1/aether-dev/sccache`,
and Cargo uses twelve build jobs. Tests run through the same metadata/RAM helper.
Logs remain in local and remote snapshot `tmp/` directories.

The conservative package allowlist is types, hash, crypto, state, consensus, DA
and execution. Node, FFI and other unverified/platform-dependent packages stay
local. Snapshots are retained for diagnosis; the target LRU does not delete source
snapshots. On 2026-10-09 the authorized peer was offline, and the SSH alias also
named an older offline IP. No remote builds or setup mutations were performed.

## Measurements, 2026-10-09

Measurements use this busy Mac, pinned Rust, four build jobs and the existing
shared sccache. “Cold” means a fresh target/binary directory, not an emptied global
sccache or filesystem cache. Compilation queue time is reported separately.
These are bounded crate samples, not a whole-workspace cold build guarantee.

| Workload | Before cold wall | Before warm wall | After cold wall | After warm wall |
| --- | ---: | ---: | ---: | ---: |
| hash + crypto, original profile vs selected profile | 422.02 s | 4.32 s | 358.97 s | 5.98 s |
| hash + test-support, Cargo vs nextest helper | 47.04 s | 1.74 s | 53.19 s | 1.70 s, no gate |
| Swift, full 52-test suite | 525.72 s, including initial queue | not separately measured | clean rerun: exit 75 after 1,215.43 s | not verified |
| affected selection only | 4.47 s initial | 0.28 s | — | — |
| build-record fingerprint only | 2.66 s initial | 0.13 s | — | — |
| stalled RAM mount/cleanup | 131.10 s before utility bounds | — | 30.86 s bounded failure | — |
| remote crate tests | not available | not available | peer offline | peer offline |

Profile comparison without dependency optimization: line tables/64 units took
191.76 s cold / 4.90 s warm; line tables/256 units took 248.60 s / 11.44 s. All 18
hash/crypto tests passed for these candidates. The small hash/support sample's
cold regression is retained in the table; it is not evidence of a cold speedup.
The dependency opt-level-1 variant reduced the crypto test body from 2.26 s to
0.26 s (8.7x) and cold wall time from 422.02 s to 358.97 s. It costs more upfront
than the 191.76-second unoptimized-dependency variant; repeated crypto-heavy
tests benefit after the shared dependency cache is populated. Warm Cargo setup
rose from 1.79 s to 5.43 s during this busy-host sample, so the measured whole
warm command regressed despite faster test execution. The metadata-reuse runner
removes that Cargo setup from unchanged iterations. No claim is made that every
crate or every cold build improved.

The system linker is Apple ld-1267 (ld-prime). Rust ships LLD 22.1.8; its version
probe worked only with the toolchain lib directory in DYLD_LIBRARY_PATH. The
comparative link benchmark was not run before the compile-wait stop condition.
The system default remains; no global or release linker/Rust flags were changed.
A faster alternative has not been established on this host.

The initial Rust queue took 930.10 seconds; warm verified build reuse avoided it.
The first optimized Swift attempt executed all 52 test bodies successfully with
a reported 52 builds/zero cache hits, taking 1,031.39 s including 895.67 s of queue time
(135.72 s outside the queue). Its outer Bash runner exited 1 because it had been
edited while waiting and resumed at an obsolete file offset. This is recorded as
an interrupted measurement, not a green end-to-end result; baseline seeding also raced this batch. Baseline-binary
seeding/copy experiments were archived and excluded. A subsequent frozen, empty
cache rerun hit the 1,200-second queue limit; remaining builds stopped as required
by lane rules. The lead must run frozen cold/warm Swift and a single-cache-miss
sample before accepting the under-one-minute warm target.
RAM attach/format/mount, a 1 MiB write/fsync and detach succeeded once at 165 MiB.
Subsequent mounts stalled, so a complete SSD/RAM I/O comparison and actual devnet
scenarios could not be verified. A copied Swift binary showed a 63.54-second first
launch and a 0.01-second repeat; the mechanism was not profiled. Copied-binary
seeding is excluded from the final benchmark. If executable startup is slow,
inspect the macOS Developer Tools guidance before attributing the delay to Rust
or test code; this lane changed no global security settings.

The pinned 0.7.3 prover reports
`0a040af91cff278b7b964b44da376c332c46077adc1b982778fa9d087761c4b0`.
Protected source/build-file SHA-256 snapshots and the release-profile byte check
passed. No guest or release build was run. Evidence and the lane report are in
`tmp/dev-speed/` and `tmp/dev-speed-report.md`, respectively.

References: [Cargo profiles](https://doc.rust-lang.org/cargo/reference/profiles.html),
[nextest binaries](https://nexte.st/docs/installation/pre-built-binaries/),
[nextest build reuse](https://nexte.st/docs/machine-readable/list/),
[nextest macOS startup](https://nexte.st/docs/installation/macos/).
