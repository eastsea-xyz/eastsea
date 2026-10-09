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
scripts/dev-test.sh
scripts/dev-test.sh --remote
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

`dev-test.sh` defaults to changes since `HEAD`, including staged, unstaged and
untracked paths. `--base REF` also includes committed changes since that ref.
It uses `affected-crates.py` for Rust dependency selection and the existing Swift
source table for pure-test selection. An edit to a registered Rust integration
test selects that binary; library, shared fixture and Cargo configuration edits
keep the broader package selection. Swift shared sources fan out to every
registered consumer; UI sources without pure coverage print the missing coverage.
`--dry-run` prints the selection and commands. `--changed-file PATH` scopes a
focused sample explicitly, and `--rust-test NAME` selects an integration gate for
a source-edit sample. Those scoped samples do not establish whole-crate coverage.

On a cache miss the fast path tries the normal local counting semaphore with a
60-second queue limit. It offloads only when the gate's timing record confirms a
queue timeout; an ordinary test failure, even exit 75, is returned. This is a
measured bounded wait, not a predicted queue duration. `--remote` skips that local
attempt entirely. Unchanged cached binaries still run their tests without a slot.
Timing records under `tmp/dev-test-*/timing.json` include command wall time,
selection, queue, compilation, test execution and remaining orchestration time.

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

The wrapper releases only the semaphore slot whose recorded PID and worktree
match its completed child. The semaphore's two-minute orphan grace remains for
unknown owners; a verified completed build does not impose that delay on the
next edit. The wrapper records queue and gated-command time separately.
Signal handlers unwind Python's subprocess wait before cleanup waits again;
cancelled builds and runtime tests reap owned process groups and preserve their
signal exit status in timing records.

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

## poc-m3 offload

The authorized builder is the `poc-m3` SSH alias, account `kjaylee`, macOS M3 with
24 GiB RAM and preinstalled Rust 1.98.1. All remote work stays in
`~/eastsea-lab/dev-speed`. A read-only preflight checks the account, platform,
owned paths, free/speculative RAM and disk before any remote write.

```bash
scripts/remote-test.sh --dry-run -p aether-crypto
scripts/remote-test.sh --setup
scripts/remote-test.sh -p aether-hash -p aether-crypto
scripts/remote-test.sh -p aether-node --test rpc_alias
scripts/remote-test.sh --swift tx-status-text
```

Setup validates the installed toolchain and downloads official prebuilt nextest
and sccache 0.18.0 only when absent. Tools and Cargo/sccache state stay in the
lane's `tmp/`; setup never compiles a tool or installs globally. The source-only
snapshot includes uncommitted Rust and pure Swift inputs while excluding
credentials, private keys, symlinks, app data, `.git`, guest projects and targets.
System `/usr/bin/rsync` synchronizes the stable `source/` directory and removes
deleted sources there. It preserves `source/tmp/` and the sibling warm targets.
A snapshot lease prevents simultaneous transfers/builds from mixing sources.
The target family includes the base commit, compiler and profile/config inputs.

Every remote build and test runs at `nice -n 15`, with four Cargo workers and two
Swift workers. The guard samples owned processes every 0.5 seconds, including its
private foreground sccache server. It stops if aggregate owned RSS exceeds
12 GiB, free plus speculative RAM drops below 4 GiB, or free disk drops below
30 GiB. It never counts inactive/reclaimable pages as free. Signals, lost command
ownership, resource stops and normal completion clean up only owned processes.
Failed process discovery also stops the run; cleanup signals recorded private
groups and reaps direct children without requiring another successful `ps` query.
Warm build artifacts remain for the next run. Remote builds bypass the local
gate only in the exact guarded `kjaylee` snapshot; they never take this Mac's slot.

Explicit normal workspace packages and registered pure Swift tests are accepted;
FFI/staticlib, alternate manifests/profiles, release and Jolt guest entry points
are rejected. The runner does not access the v4 validator/port 8604,
`~/aether-testnet`, its LaunchAgent or `/Applications/EastSea.app`.

## Measurements, 2026-10-09

### Round 2: edits while lanes were active

These are wall-clock measurements on this Mac with other Rust/Swift work,
validators and background CPU work running. The test probes append one comment
line and restore the source after execution. The initial dependency-population
probe added a comment plus a separating blank line; it is not a warm one-line
sample. Every local compile used the existing semaphore and four Cargo jobs.
No local queue reached the 20-minute stop limit in this round.

| Actual workload | Command wall | Semaphore queue | Compile | Test launch + run | Other time | Result |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| Populate node dependencies; supervisor source probe → `wake_signal` | 255.81 s | 0.28 s | 248.20 s | 3.96 s | 3.37 s | exit 100; startup/signal test failed |
| Warm dependencies; one-line supervisor edit → `wake_signal` | 90.34 s | 0.21 s | 83.64 s | 3.73 s | 2.77 s | exit 100; same test failed |
| One-line `crates/node/tests/rpc_alias.rs` edit → affected integration binary | 98.24 s | 0.40 s | 55.54 s | 33.41 s | 8.89 s | three tests passed |
| One-line `apps/wallet/Sources/TxStatusText.swift` edit → affected pure test | 3.49 s | 0.02 s | 1.86 s | 0.43 s | 1.18 s | passed |
| Full pure Swift suite, 52 fresh builds | 67.17 s | 0.23 s | 22.78 s | 40.82 s | 3.34 s | all 52 passed |
| Full pure Swift suite, 52 cache hits | 23.44 s | 0.00 s | 0.00 s | 22.37 s | 1.07 s | all 52 passed; no compile slot |

Rust compilation above is the complete gated Cargo command, including Cargo
setup/linking and any Cargo artifact-lock wait; the saved logs do not establish
an independent artifact-lock split. Swift compilation is the measured compiler
batch. Test time includes process startup and nextest setup. The `rpc_alias`
test bodies took only 0.056 s according to nextest, despite 33.41 s for that whole
runtime stage. The difference is measured startup/orchestration time; its cause
was not profiled. Whole command wall also includes selection and cache/metadata
work. Components are independently rounded; the wall column is authoritative.

The source edit in the table uses an explicit `--rust-test wake_signal` scope;
it does not represent all node tests. The integration edit selects `rpc_alias`
automatically from Cargo's registered target source path. Cargo/shared fixture
edits retain the wider gate. The Swift edit uses the exact existing source table,
and an `EarningsModel.swift` edit selects all eleven registered consumers.

Round 1 measured 930.10 s of Rust queue and 895.67 s of Swift queue on a different
busy-host sample. This round observed sub-second queues for the listed edits.
Those are different load/cache samples, not matched before/after speedup ratios.
The pure Swift edit is near real-time in the measured subset. The measured Rust
source and integration edits remain above one minute; no whole-node real-time
claim is supported. Initial dependency population is not comparable to a warm
edit. Remote speedup and warm remote target reuse have not been measured.

Real remote preflight reached `poc-m3` and refused the run before transferring
sources or creating lane artifacts: free plus speculative RAM was 0.14 GiB,
below the 4 GiB floor; free disk was 106.31 GiB. Earlier read-only RAM samples
were about 2.5–3.3 GiB. Existing machine workloads were left untouched. No remote
build/test or setup was started, and no other remote host was used. The remote
path, warm target reuse, 60-second fallback, resource stops and owned-process
cleanup are tested with fixtures; fixture delays are not performance measurements.

The two `wake_signal` failures are retained. Its supervisor test sleeps exactly
two seconds, checks only that the process is alive, then sends SIGUSR1; it has
no startup-readiness check. The observed default SIGUSR1 exit is consistent with
the handler not being installed yet. A later safe `aether --help` probe took
0.0264 s, which cannot prove the failed launch's startup latency. No retry was
used to convert these failures to a passing measurement, and no node/guest
source change is included in this round.

The original Swift clean rerun's 1,215.43-second wall and exit 75 were an intentional
1,200-second semaphore queue timeout. Its log contains no compiler diagnostic or
test result, and the saved report records zero compiled artifacts. A 0.25-second
blocked-gate fixture reproduced exit 75 through the real Swift wrapper with zero
compiler calls/test bodies, taking 1.2757 s. The extra 15.43 s in round 1 cannot be
split retrospectively from saved evidence. The separate earlier Bash exit 1 came
after editing a live runner while it waited; it is a different failure. Final
round-2 Swift measurements used frozen runner inputs. The timeout stays intact;
`dev-test.sh` limits its local attempt to 60 seconds before guarded offload.

Raw measurements, timing JSON, source snapshots, the Swift diagnosis and the
remote resource refusal are under `tmp/dev-speed-round2/`; per-command fast-path
timing records are under `tmp/dev-test-*/timing.json`.

### Release and guest isolation proof

Compared round-1 parent `fc75af2c969af9884d997e60bcc2abf2f40e5489`, round 1
`234cb0f49124917a2a1d84a65d572421dd359e7d`, and this round's working manifest.
Parsing each root manifest with `tomllib` and removing only `profile.dev` and
`profile.test` gives identical documents. The release family is unchanged:

```json
{"bench":{"debug":true,"inherits":"release"},"release":{"codegen-units":1,"lto":"fat"}}
```

Normalized sorted-key compact JSON plus a trailing newline hashes to
`aafb884c8bc4a342ba650243e36f2e8c234229806612872fb209b31537e38e2a` in all three
versions. Original release-table bytes hash to
`2ef5db72cb6395749e05265dc7fa8671d061e9c994849eb538d6ade7cd2f88be` in all three.
The complete manifest after removing dev/test profiles hashes identically to
`0adaeae6480bd77a777c2af35ac2de05672a41940ec96251c5b78b24e2b340e9`.
There are no additional custom profiles inheriting dev for release, no ancestor
or user Cargo config files, and no `CARGO_PROFILE_*` environment overrides in the
checked session. The inherited wrapper environment was preserved.

Both read-only metadata commands succeeded; neither compiles or runs build scripts:

```bash
PATH="$HOME/.cargo/bin:$PATH" cargo +1.98.1 metadata --manifest-path Cargo.toml --no-deps --locked --offline --format-version 1
PATH="$HOME/.cargo/bin:$PATH" cargo +1.98.1 metadata --manifest-path apps/prover/Cargo.toml --no-deps --locked --offline --format-version 1
```

Root metadata reports 17 members, excluding prover and guest. The separate
`apps/prover` workspace reports exactly prover and guest, and owns its unchanged
release profile (`debug = false`, `codegen-units = 1`). Its `build-guest.sh` clears
host Cargo/profile environment and passes `--release` to Jolt. Shared Aether path
dependencies inherit unchanged workspace package/dependency/lint tables;
root dev/test package overrides do not supply that workspace's guest profile.
Cargo selects release independently of dev; test inherits dev and bench inherits
release. See [Cargo profiles](https://doc.rust-lang.org/cargo/reference/profiles.html)
and [workspace profile ownership](https://doc.rust-lang.org/cargo/reference/workspaces.html).

All 110 protected tracked files match the round-1 parent byte for byte, covering
guest-compiled crates, prover/guest orchestration, locks/toolchains and release
scripts. Their sorted SHA-256 listing hashes to
`4ce11ad4da4b6862be6d46c0cb4914630e74d29c6466ced341f01e3ac763edbb`.
The existing guest-input function reports 644 entries and identical input digest
`f88608b2c6d0e1ea042c5af21c635d4bbd8089c3c8363b32b62c396d0e244c8e` for all three
root manifests with the installed guest toolchain, rustc 1.95.0. That function
hashes workspace inheritance tables, excluding dev/test profile tables. This is
an input digest, not a newly built guest ELF/program id. Metadata proves workspace
membership, not release binary equivalence. Configuration/input evidence proves
the profile additions cannot affect the checked release or guest build paths;
no release or guest build was performed.

Exact commands, normalized documents, metadata, protected hashes and the
non-compiling reproduction checker are in `tmp/dev-speed-round2/profile-*`.

Verification for round 2: all 52 real Swift tests pass from fresh builds and again
from 52 cache hits; the three real Rust `rpc_alias` tests pass. Workflow regression
fixtures (62 unittest cases plus the Swift cache/registration harness) cover
selection, content-cache reuse, gate ownership/timing, cancellation,
remote path/resource/transfer safety and default Swift registrations. ShellCheck,
Python parsing/static checks and diff whitespace checks pass. The `wake_signal`
failure, whole-node edit latency and missing remote performance measurements are
remaining limitations; this report does not mark the full Rust suite green.

### Round 1 historical measurements

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
