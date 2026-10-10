# Registered app content (0.7.4)

The Mac wallet resolves `sea://demo.sea` through the configured Names and
AppRegistry contracts, fetches the active bundle from its local node, and
verifies the canonical index and every file before opening a page. It shows
the name as the origin; the WebKit document uses an isolated
`eastsea-app://<base32-appId>/` origin internally. Registration and matching
content hashes do not establish a publisher's safety or a finality proof for
the local node's registry reads.

## Build and pin content

Choose a static output folder containing `index.html` and separate script
files. The transport is deterministic ustar: relative ASCII paths in byte
order, mode 0644, zero uid/gid/mtime, no links, directories or duplicate names.
The archive includes canonical `bundle.json`. Its SHA-256 is `bundleHash`;
the archive's SHA-256 is not the registry identity. Every indexed file has
its own size and SHA-256. The complete archive, including padding and headers,
is limited to **20,000,000 bytes**; there may be at most 2,000 files, each
at most 10 MiB. The index may be at most 1 MiB.

Use an owned development directory:

```sh
target/debug/aether app-bundle build --folder ./tmp/my-app --out ./tmp/my-app.tar
target/debug/aether app-bundle pin --data ./tmp/my-node --archive ./tmp/my-app.tar --hash <bundleHash>
target/debug/aether app-bundle configure --data ./tmp/my-node --seed true
```

Publish the returned **index hash** in the app's active registry record and
bind that app ID in the name's `app` text record. Registry deployment pins
are per chain in `apps/wallet/Resources/name-sources.json`: each of `names`
and `apps` supplies an `address` and a lowercase `code_sha256` of its runtime
bytes. The resource currently has no deployment addresses; the wallet refuses
unconfigured networks instead of guessing them. The `sea-names` source and
ContentSource protocol were imported unchanged from that lane's draft;
`NodeAppContentSource` implements the protocol using the verified snapshot.

## Node cache and transport

`aether_appBundle` takes `[bundleHash, path]`, where `bundle.json` requests the
index. Its result is `{bundleHash, path, sha256, size, data}`, with base64
bytes. It serves indexed paths only. This method is local HTTP RPC;
public iroh RPC and the read-only gateway refuse it. The separate
`aether/apps/1` ALPN transfers bounded archives from verified cache entries.
Nodes use their existing roster and learned wallet-server peer IDs.

The cache lives in `<node-data>/apps`, has a 500 MiB LRU ceiling and a bounded
entry count, and retains explicit pins within that ceiling. Pins never bypass
the node's free-disk floor. Downloads and cache writes check the actual volume
before allocation and again before publishing a cache entry. Verified cached
reads can continue at the floor. A two-bundle immutable memory cache avoids
rehashing the entire archive for each requested asset. It is bounded to
40,000,000 archive bytes. On a disk read, all hashes are checked again.

Seeding starts **off**. It has a bounded peer/request budget and a transfer
deadline. The following settings take effect after restarting the owned node:

```sh
target/debug/aether app-bundle configure --data ./tmp/my-node --seed false
target/debug/aether app-bundle configure --data ./tmp/my-node --enabled false
target/debug/aether app-bundle unpin --data ./tmp/my-node --hash <bundleHash>
```

## Wallet execution and developer mode

Only verified, immutable bundle assets reach the scheme handler. Top-level
documents must be HTML. A CSP header and an HTML meta policy deny remote
scripts, inline scripts, eval, frames, forms and network connections, including
loopback RPC. Scripts must be separate bundle files. SVG stays an image asset.
Permissions and website storage are scoped to chain, registry and app ID.
The privileged provider handler lives in an isolated WebKit content world;
native dispatch also checks the active view, main frame and exact app host.
Navigation, account, network, lock and developer-mode changes cancel pending
requests. Each account connection and transaction uses the existing native
confirmation flow and displays the resolved `sea://` origin.

In Settings, enable **Developer mode**, then use **Open a local app folder**
in the browser address bar. A snapshot of a static folder opens under its own
fresh namespace. The fixed native banner says **개발 중 · 검증 안 됨** in Korean.
The picker is hidden while developer mode is off. Turning the mode off closes
local content. Local files can sign only on the wallet's owned development
chain (7777), with the normal confirmation flow.

Unavailable, missing, oversized or mismatched content displays an error and
never reaches the renderer. Registry changes during a download also refuse
the load. The running view pins the verified release until the next opening.

## Owned-devnet verification

```sh
scripts/test-app-content-devnet.sh
```

The script observes the team's compiler queue before each compiler command.
It builds a standalone WebKit test executable and tests an owned on-disk
Chain with signed, finalized deployment/registration transactions. The test
uses synthetic one-record Names/Registry runtimes, real loopback iroh transfer,
the wallet's actual name reader and ContentSource, all-file hashing, CSP,
private-scheme page loading and an EIP-1193 chain-ID round trip. It also checks
JavaScript modules/imports and a working counter interaction, alongside
corruption, path traversal, offline cache reads and rejection of unverified
local content. It starts no EastSea application or consensus validators and
sends no transactions to a live network. It does not establish production
registry contract coverage, independent-validator finality, manifest
permissions, bidirectional `name_binding` provenance, or public relay reachability.

`scripts/test-app-content-devnet.sh --build-helper` builds just the native
helper. Set `AETHER_APP_PAGE_HARNESS=$PWD/tmp/app-content/page-check` when
running the full node test gate to include the wallet page portion there.
Pure Swift tests cover index/file tampering, traversal, canonical encoding,
size bounds, developer gating and filesystem symlink escapes. Node unit tests
exercise pin/LRU persistence, the disk floor, opt-out and peer fetching.

## Native gate rerun (2026-10-08–09)

Verification starts at `9165a4a5f9f6e62c6af1b430043f23dac25c3380`
(`codex/app-content`). All builds and tests run on this Mac with
`CARGO_BUILD_JOBS=4`, `TMPDIR=$PWD/tmp`, and the lane's own build outputs and
pinned Jolt/Akita worktrees under `tmp/app-content/jolt-forks`. Every native
command retains the compiler semaphore in its caller's shell.

The initial rerun found that the old per-compiler calls in the native test
scripts could acquire both available lane slots from the same long-lived
shell and then block their next acquisition. Commit `2b4d7d8` adds a local
helper that reuses only a live caller/ancestor's slot for the same physical
worktree. These runners invoke compilers sequentially; the helper is for
that sequential scope. It does not alter the shared semaphore or its slot
count, and an unavailable gate remains a hard failure.

The isolated regression fixture uses compiler no-ops to test allocation
lifetime independently of native compilation. Before edits, each of the
four runners blocked after one compiler with both slots owned by live
shells. After the repair, they completed 51/2/3/3 compiler calls respectively
with a single slot. A separate missing-gate check caught an intermediate
`--build-helper` exit-zero bug before the failure-propagation fix; it now
exits 1 without reaching a compiler.

Reproduce these recorded red/green checks (no compiler semaphore needed):

```sh
TMPDIR=$PWD/tmp CARGO_BUILD_JOBS=4 python3 tmp/app-content/semaphore-check/check-lifecycle.py original
TMPDIR=$PWD/tmp CARGO_BUILD_JOBS=4 python3 tmp/app-content/semaphore-check/check-lifecycle.py repaired
TMPDIR=$PWD/tmp CARGO_BUILD_JOBS=4 python3 tmp/app-content/semaphore-check/check-unavailable.py before
TMPDIR=$PWD/tmp CARGO_BUILD_JOBS=4 python3 tmp/app-content/semaphore-check/check-unavailable.py after
```

Bash syntax and ShellCheck pass for the helper and the three sequential
native runners. The pure Swift runner retains exactly its pre-existing
ShellCheck diagnostics. The standalone WebKit helper also compiled from
the base revision with exit 0 (`tmp/app-content/gates/helper-head.log`).

| Gate | Command (after the semaphore) | Result |
| --- | --- | --- |
| Provider JavaScript | `node --test apps/extension/test/wallet-provider.test.mjs` (no compiler gate) | PASS: 14 tests |
| Korean copy | `scripts/check-wallet-l10n.sh --lint` and `--missing` (no compiler gate) | PASS |
| Rust format | `rustfmt --check --edition 2021 crates/node/src/app_bundle.rs crates/node/tests/app_content_e2e.rs` (no compiler gate) | PASS |
| Native page helper | `scripts/test-app-content-devnet.sh --build-helper` | PASS: exit 0 |
| Touched Rust crates | `AETHER_APP_PAGE_HARNESS=$PWD/tmp/app-content/page-check cargo test -j4 -p aether-node -p aether-net --tests --no-fail-fast -- --test-threads=1` | 30 targets PASS (524 tests); nine remaining targets queued |
| Owned devnet | `scripts/test-app-content-devnet.sh` | Cargo app-content scenario PASS; standalone script queued |
| Pure Swift | `scripts/test-swift-pure.sh` | Queued at semaphore |
| Wallet | `WALLET_ADHOC=1 scripts/build-wallet.sh` | Queued at semaphore |
| Rust static analysis | `cargo clippy -j4 -p aether-node -p aether-net --all-targets -- -D clippy::correctness` | Queued at semaphore |
| Wallet screen renders | `scripts/wallet-screens.sh settings` and `scripts/wallet-screens.sh window-explore` | Queued after wallet build; previous devnet page render verified |

Logs and task-owned evidence are under `tmp/app-content/gates/`.

### Failures recorded before fixes

- Semaphore regressions use the same success assertions against the saved
  pre-edit and repaired runners: `check-lifecycle.py original` exits **1**,
  while `repaired` exits **0**. The missing-gate check likewise exits **1**
  for `before` and **0** for `after`. The subject-runner failures were first
  observed before source edits. Logs and `.exit` files are in
  `tmp/app-content/gates/semaphore-*`.
- The first full Rust gate compiled successfully and passed 356 node library
  tests (4 existing ignored), 13 node CLI tests, the net unit tests and the
  activation integration tests. It then exited **101** at
  `published_app_resolves_from_finalized_owned_devnet_and_loads_verified_content`.
  Cargo ran the helper from `crates/node`, so its workspace-relative
  `apps/wallet/Resources/provider.js` lookup failed with Cocoa error 260 / POSIX
  error 2. The helper launch now explicitly selects the workspace root using
  `CARGO_MANIFEST_DIR/../..`, preserving the real provider and screenshot path.
  Before-fix evidence is in
  `tmp/app-content/gates/cargo-tests-before-cwd-fix.log`. The already-built executable was then run from the workspace root with
  the exact same test and assertions: **1 passed**, exit **0** in 8.28 s.
  It verified module imports, strict CSP, the isolated provider handler,
  chain-ID reply and a counter interaction, and produced
  `tmp/app-content/devnet-page.png` (1280×960; marker and count `1` visually
  checked). The focused Cargo rerun passed with the fixture fix compiled in
  (exit **0**, `tmp/app-content/gates/app-content-target.log`). The full
  retry later passed this same target. Standalone command:

  ```sh
  TMPDIR=$PWD/tmp AETHER_APP_PAGE_HARNESS=$PWD/tmp/app-content/page-check tmp/cargo-target/debug/deps/app_content_e2e-0e7de9c45f0e0cd5 --exact published_app_resolves_from_finalized_owned_devnet_and_loads_verified_content --nocapture --test-threads=1
  ```

  Its log is `tmp/app-content/gates/standalone-e2e-from-workspace.log`.
- Follower startup converted a typed disk error from `Chain::open` into a
  generic string, losing the storage exit code. The native full-disk scenario
  exited **1** instead of **4**. Commit `5f91e7d` keeps typed disk errors until
  classification in both restoration attempts, preserving the database and
  the distinct storage exit code. The new deterministic binary regression
  injects ENOSPC after store opening. With only the test setup added and the
  production code unchanged, it failed at runtime with exit **101**; the
  unchanged assertions passed with exit **0** after the production fix.
  The red source patch, red/green logs and exact command are recorded in
  `tmp/app-content/storage-restore-red.patch` and
  `tmp/app-content/gates/storage-restore-evidence.md`:

  ```sh
  cargo test -j4 -p aether-node --bin aether tests::restore_disk_failure_exits_with_storage_code -- --exact --nocapture --test-threads=1
  ```

  It ran under the retained compiler slot with the same workspace `TMPDIR`
  and four-job environment. The subsequent full retry also passed all 14
  binary unit tests. The new regression specifically covers the first
  restoration attempt; the retry branch uses the same error handler.

### Resume from the interrupted retry

Resume HEAD is `5f91e7d060e7f9020599f60bcc89a046c142895d`. The saved full Rust
retry built the fixed source and completed 30 targets through `run_lock`:
**524 passed, 0 failed, 9 existing ignored**. This includes the owned app-content
WebKit page, all 12 devnet scenarios, and the new restoration regression.
The process stopped during `selfheal` when the earlier session ended; it
left no live test process. Its log is
`tmp/app-content/gates/cargo-tests-retry-2.log`, with per-target outcomes in
`tmp/app-content/gates/suite-coverage-before-resume.json`.

The resume runner retains the outer shell's slot and invokes the same
assertions for the nine interrupted/remaining targets, followed by the
standalone devnet, pure Swift, Clippy and ad-hoc wallet gates:

```sh
export CARGO_BUILD_JOBS=4 TMPDIR="$PWD/tmp"
~/.claude/playbooks/aether-team/wait-compile.sh && /bin/bash tmp/app-content/resume-native-gates.sh
```

Its commands and individual exit records are under
`tmp/app-content/gates/*-resumed.{log,exit}`. Independent review of the three
fix commits found no blocking issue and confirmed the tests' assertions were
preserved. The resume's JavaScript provider tests (14), both Korean copy
checks, Rust formatting, Bash syntax and the helper/runner ShellCheck gate
all exit **0**.

The Rust continuation command in that runner is:

```sh
cargo test -j4 -p aether-node --test selfheal --test shadow_replay --test shards --test sim --test state_budget --test store --test validator_program --test wake_signal --test zero_fee --no-fail-fast -- --test-threads=1
```

As of 2026-10-09 05:49 KST, the outer semaphore has not returned, so the
resume runner has not started. Both shared slots are held by old
`test-update-daemon.sh` callers in other worktrees:

- Slot 2: `issuance-gate`, owner PID 1050, waiting child PID 67604.
- Slot 1: `update-noupdate`, owner PID 69873, waiting child PID 58271.

Each caller remains alive while waiting to acquire another slot; neither has
a compiler child. Slot 0 remains reserved for release work. No semaphore
configuration, other lane's processes or shared release state was changed.
The snapshot is `tmp/app-content/gates/compile-blocker.json`. Stopping these
two other-lane runners requires separate authorization; that request is
pending. **Native verification is incomplete**, and the queued gates and
screen renders must finish before claiming the lane is verified.
