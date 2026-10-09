# Chain-announced wallet releases

Design 34 U1–U4 uses finalized `ReleaseLog.Published` receipts for discovery.
The node checks the pinned contract code, finalized entry storage, payload
hashes, and two distinct builder signatures (three for an emergency). It
announces the release only after the next finalized header certifies the
publication state. Later permissionless spam cannot replace an approved
release. Exact payloads are kept in bounded store metadata and checked again
after a restart or checkpoint change.

`aether_status.release` contains the entry index, chain and contract, version,
build, manifest/archive/signature hashes, exact manifest and signature JSON,
approval count and threshold, publication height/time, install height/time,
artifact metadata, and restart-slot metadata. It is discovery data. The
wallet independently checks `verified_release`, including its certified
storage and code proofs, before treating an entry as approved.

Automatic discovery checks default to off. Only an app whose bundled
`network.json` identifies legacy 7777/7780 without a release pin enables the
existing hourly Sparkle feed. A present pin is enforced even on 7780;
malformed pins and other chains without pins never enable that fallback.
The wallet reacts to the existing chain-status stream, so it does not add a
release-discovery timer or contact GitHub before an announced entry exists.

The approved manifest may name an `EastSea.dmg` artifact with `size`, `url`,
`webseeds`, and `peers` (HTTP(S) artifact-serving peers). Without a URL the
wallet uses the versioned GitHub release URL. Each source is tried by hash;
the archive is streamed into a private content-addressed cache and its
SHA-256 and signed size are checked before use. Sparkle receives a generated
appcast and the checked archive through a private loopback HTTP server,
retaining its bundled EdDSA verification. It does not download the unchecked
origin again. A bad source may fall back to another source with the same hash.

For every chain entry, including emergencies, installation waits for both
72 hours of certified chain time and publication height plus 259,200 blocks.
Only a signed additional height may extend that wait; RPC discovery heights
cannot postpone it. The wallet checks the proof and cached archive again
immediately before stopping writers.
Seated nodes additionally need a fresh restart-slot decision from the same
signed local RPC listener as their voting-membership observation. Missing,
stale, or unavailable slot data keeps them waiting. Existing storage-move,
migration, signing, membership, writer-quiesce, run-lock, and rollback gates
continue to apply. An eligible fixture takes the silent Sparkle path; an
actual installed wallet is never changed by the tests. Transport failures
retry the already announced release with the existing bounded backoff, and
failed supersession clears the old feed. The synchronous Sparkle callback
compares file identity; full hash verification runs asynchronously before
download and installation. Already installed announcements preserve the
post-install health window.

## What is missing on 7780

The bundled 7780 network file has no `release` object: no pinned ReleaseLog
address/code hash, no three builder keys, and no 2/3 and 3/3 release thresholds.
Its legacy genesis has no ReleaseLog predeploy. Consequently this wallet has
no verifiable on-chain 0.7.4 approval to discover there. Deploying a contract
alone cannot create the absent trust pin. This lane changes neither genesis
nor the bundled network bytes and does not query or change the live validators.

`crates/node/tests/release_watch.rs` uses an isolated devnet with the existing
new-genesis predeploy and deterministic fixture builder keys. Swift tests use
task-owned files and independently supplied proof fixtures. They cover forged
entries, insufficient or duplicate signatures, incorrect archive hashes,
early installation, and a valid approved release. Each refusal has one plain
English/Korean sentence; a valid release needs no approval dialog.

## Lane verification and integration handoff

The lane merged `lead-merge` at `fc75af2`. The 14 ReleaseLog/restart-slot
checks failed against the old node and passed with the watcher. FFI discovery
regressions, an inflated discovery height, and the installed-build policy
were also checked against their old behavior before the passing tests.

`cargo test -p aether-ffi`, `scripts/test-swift-pure.sh` (including the native
Sparkle handoff and four refusal cases), `scripts/test-release-scripts.sh`,
release-manifest tests, localization lint, and the generated bindings check
passed. The bindings match the fresh FFI metadata; the two added Swift lines
only normalize generator trailing whitespace. Rust formatting and production
Swift syntax checks passed. The existing 0.7.2 shutdown/storage/rollback gates
remain in the install path.

The full `cargo test -j4 -p aether-node --tests` run passed 431 checks before
`catchup_devnet` timed out preparing its validators: it requires 1,000 blocks
within 300 seconds, while this debug run reached 237. The follower had not
started. All four isolated validators continued finalizing contiguous blocks.
This test already supports `AETHER_CATCHUP_TEST_BIN` for a same-source optimized
fixture; the lead must run that full integration gate with its optimized
binary. This lane does not change the scenario's assertions or build release
artifacts. The 14 lane tests were rerun after removing unnecessary release work
on chains without a pin; RPC status compatibility was checked separately.

The lead also owns `WALLET_ADHOC=1 scripts/build-wallet.sh`. An actual app
replacement/relaunch is not performed by the native fixture: it checks the
real Sparkle item and HTTP enclosure, the EdDSA-signed fixture bytes, and a
prompt-free install closure. No installed app or real wallet data is changed.

Guest inputs remain identical to the shipped baseline, and the copied guest
ELF is `0a040af91cff278b7b964b44da376c332c46077adc1b982778fa9d087761c4b0`.
No guest build, genesis change, remote host operation, or push was performed.
