# 0.7.0 (EastSea) testnet release: first-launch migration check

Date: 2026-10-06/07. Source: `lead-merge` @ 6e8b587, built in the detached worktree
`.claude/worktrees/release-070`. Nothing was published, the real data was never moved, and the GUI app was never launched.

## Verdict

**The migration logic is ready. The 0.7.0 canary release is not: NO-GO until blockers B1 to B4 are fixed.**

On a clone of this Mac's real 0.6.6 data, `DataMigration.migrate` worked in every test:
- It moved the node and copied the keys with hashes intact.
- A second launch did nothing.
- Kills at random points and a resume after a kill all recovered.
- The cross-volume path quarantined the old signer correctly.
- The migrated data followed chain 7780 at the tip, block for block with the real node.

The blockers are in how the release reaches users (version number, the Sparkle update path, the build itself) and in one gap in the migration's done flag that is already live on this Mac.

## 1. What first launch touches

**How paths resolve.** `DataMigration.supportURL` uses `FileManager.urls(.applicationSupportDirectory, .userDomainMask)`. `NodeController.dataDir` uses `homeDirectoryForCurrentUser + "Library/Application Support/EastSea/node"`. The app is not sandboxed (no entitlements; the 2023 `~/Library/Containers/com.pipln.eastsea` belongs to an unrelated old project). Both resolve through getpwuid / CFCopyHomeDirectoryURL, so **`$HOME` does not redirect them**; only `CFFIXED_USER_HOME` affects Foundation, and nothing redirects cfprefsd, SMAppService or the keychain.

**When it runs.** `DataMigration.ensure()` (a full `migrate()` on every call until the done flag is set) is called from:
- `UpdateTracker.defaultRecordURL`, which runs during `AppDelegate` property init, on the main thread, before `applicationDidFinishLaunching`
- `NodeController.dataDir`
- `EnclaveAccount.storeURL`

The gates `mayStartNode()` (NodeController.start) and `mayCreateFreshWalletKey()` (EnclaveKey.loadOrCreate) hold off node start and new wallet keys while old data waits.

**Files.** All of these live under `~/Library/Application Support/`.

| Old path | New path | How it moves |
|---|---|---|
| `Aether/node/` (8.5 GB here: keys, network.json, state.redb, follow/state.redb, follow/wallet-node.key, run.lock …) | `EastSea/node/` | Same volume: one `rename` of the whole tree, checked by a name→size manifest (no hash). Otherwise: verified copy with SHA-256 of every file, then the old tree's `validator.key`, `validator.pub.json`, `node-account.key`, `threshold.json`, `aether-consensus*`, `dkg-agreement-*` and `vote-epoch-*` move into `Aether/node/eastsea-quarantine-<ms>/`, and the marker `MIGRATED-TO-EASTSEA` is written |
| `AetherWallet/enclave-key.dat`, `simulator-software-key.dat` | `EastSeaWallet/…` | Verified copy; the old file stays (same Secure Enclave key, so the same address in both apps) |
| `Aether/update-state.json` | `EastSea/update-state.json` | Verified copy; the old file stays |
| `Aether/node.identity` (the sibling identity guard) | (not migrated) | Stays. The new node rewrites `EastSea/node.identity` from the moved keys on first start. The old one keeps an old binary from minting a new identity in a recreated `Aether/node` |
| `Aether/agent/` | (not moved, by design) | Still read in place |

`run.lock`: `open(O_CREAT|O_RDWR)` plus a non-blocking `flock` on `Aether/node/run.lock`. The 0.6.6 node supervisor (`aether run`, pid 90796 here) holds this lock, so while the old node runs the result is `.deferred` and nothing is touched.

**UserDefaults:**
- It reads the `com.pipln.aether` domain (CFPreferences, read only) and copies any keys missing from `UserDefaults.standard`, which is `com.pipln.eastsea`.
- It writes `renameMigrationDone` and `renameNodeMigrationDone` into `com.pipln.eastsea`.

**Keychain / Secure Enclave:** the migration itself makes none of these calls (only file copies of the key handle). The first `loadOrCreate` afterwards reads the handle; if no handle exists and the gate allows it, it **creates a new SE key**.

**Processes:** none are stopped or killed. The only check on the old app is the flock above.

**Network:** none in the migration.

**Other first-launch effects, outside the migration** (these are why the GUI must not be launched on this Mac):
- `SMAppService.mainApp.register()` (a login item for com.pipln.eastsea), unless `loginItemDefaultApplied` is already set.
- `UnattendedDaemon`, when enabled or by default for a registered validator: `SMAppService.daemon("com.pipln.eastsea.node.plist")`, and a marker `EastSea/node/unattended.plist` that the root stub reads from `/Users/*/…`.
- A node on 18545/19101, which would clash with the running 0.6.6.
- DeviceCheck tokens, Sparkle checks and installs, RPC to public nodes.
- `~/.local/bin` links, only from a menu action.

## 2. Could it be tested safely?

**Launching the GUI against a copy: not safe, not done.** HOME cannot redirect Application Support. SMAppService, the keychain, cfprefsd (the real `com.pipln.eastsea` domain) and the ports are system-wide. And see B4: on this Mac a GUI launch would mint a new wallet key and a new validator identity.

**Running the migration on a copy: safe, and done that way.** `migrate(support:defaults:)` takes injected roots, so I ran it through a small harness (`tmp/harness/harness`, built from `DataMigration.swift` plus a main that refuses the real Application Support path) on APFS clones (`cp -c`, read only on the source) of the real `Aether/`, `AetherWallet/` and `node.identity`. Each run used its own defaults suite `eastsea.release070.harness.*`.

I used the scratchpad, not `$PWD/tmp/home`. An APFS clone is only possible on the home volume, and it is an instant, near-consistent snapshot of a live 8.5 GB redb, where a slow cross-volume copy would not be.

The harness's only real-system contacts:
- a read-only CFPreferences read of `com.pipln.aether`
- an flock on the *copy's* run.lock

The GUI, a full end-to-end first launch, and the Sparkle update path from 0.6.6 still need a throwaway macOS: a Tart VM (`tart clone ghcr.io/cirruslabs/macos-sequoia-base`), or a second user account with 0.6.6 installed and synced. Run them there, never on this account.

## 3. Results

| # | Scenario | Result |
|---|---|---|
| U | Unit suite `Tests/rename-migration` | 116 ok, 0 fail |
| A | First launch on a clone, same volume | `.done` in 0.03 s; the whole tree was renamed to `EastSea/node`; all 13 small files match the base hashes (validator.key, node-account.key, validator.pub.json, network.json, wallet-node.key, enclave-key.dat, update-state.json …); the old `AetherWallet/enclave-key.dat`, `update-state.json` and `node.identity` stay in place; the old prefs (nodeEnabled, acceptedTerms, proveAddress, loginItemDefaultApplied) were copied |
| A2 | Second launch, and a launch with a lost defaults domain | `.done`; no file in the tree changed (mtime and size identical) |
| C | Old node running (copy's run.lock held) | `.deferred("Quit the old Aether app first …")`, nothing touched, node and new-key gates BLOCKED; after release the next launch finishes `.done` |
| D | 60 random SIGKILLs (0–30 ms) during same-volume runs, then a relaunch | 0 violations. 24 kills landed before the move, 15 after it, 21 runs finished first. Every relaunch reached `.done` with every hash correct. In no killed state was the new node allowed to start while the old tree still held `validator.key` |
| E | Cross volume (EastSea/ on a mounted APFS image): SIGKILL about 25 s into the copy, then a relaunch | Resume finished `.done` (both flags true). New tree byte-identical, including `follow/state.redb` (8.5 GB) and `state.redb` (SHA-256 checked). The old tree's validator.key, validator.pub.json and node-account.key moved to `eastsea-quarantine-…/`; `MIGRATED-TO-EASTSEA` was written; network.json and run.lock stayed. A third launch was a no-op. Hashing cost about 160 s of CPU for 8.5 GB with the pure-Swift SHA-256; the 27 min wall time came from the test disk image |
| F | Migrated node follows 7780 | The new `target/release/aether follow` on loopback **28545** with the migrated data: chainId 7780, height in lock-step with the real node (448938 → 449015 over 100 s, equal at every sample), and `aether_getBlock(449000)` byte-identical on both. Run **without** `--candidate/--keys` and with the copy's `follow/wallet-node.key` deleted, so it could not beacon or reuse this Mac's identity or endpoint. A full `aether run` with the real validator keys was deliberately not run, because it would be a second live copy of this Mac's validator |
| B | Stale `renameMigrationDone=true` plus unmigrated old data | `.done` immediately, **nothing moved**, both gates "allowed". `aether candidate-info` on the empty `EastSea/node` minted a new validator key (see B4) |
| Build | Release app | Rust and the prover build. The app builds only with `CODE_SIGNING_ALLOWED=NO` plus ad-hoc signing by hand; `codesign --verify --deep --strict` passes. The bundle reports **0.6.6 (11)**. See B1 and B3 |

## 4. Blockers (must fix before the canary)

**B1. The version was never bumped.** `project.yml` still says `MARKETING_VERSION: 0.6.6`, `CURRENT_PROJECT_VERSION: 11`, the same as the shipped 0.6.6 (build 11). Sparkle would never offer it. Set 0.7.0 and build ≥ 12.

**B2. Sparkle cannot update 0.6.6 to EastSea.** In the shipped Sparkle, `SUInstaller` looks in the archive for a bundle named `Aether.app`, for `<CFBundleName "Aether">.app`, or for one with bundle id `com.pipln.aether` (SUInstaller.m:35, 102-105). `EastSea.app` / `com.pipln.eastsea` matches none of them, so every 0.6.6 user would hit an install failure as soon as 0.7.0's appcast is at `releases/latest`.

Decide one of:
- (a) a bridge 0.6.7 under `com.pipln.aether` that sends users to the DMG
- (b) ship the archive's app as `Aether.app` and verify in a VM what happens to the login item and the path

Either way, keep 0.7.0 off the feed that 0.6.6 reads until then. A related consequence of the bundle-id change: `Aether.app` and its `SMAppService.mainApp` login item stay installed next to EastSea. The old app then starts at login, takes run.lock and defers the migration on every launch. After a migration, it would also start an identity-less follower that re-syncs about 8.5 GB in `Aether/node` and fights over ports 18545/19101. The new app should detect a running or installed Aether.app and tell the user to quit and delete it.

**B3. The Release build is broken at HEAD.**
- `scripts/build-wallet.sh:67`: `"${swift_flags[@]}"` with `set -u` and an empty array fails under macOS `/bin/bash` 3.2 ("unbound variable") whenever `OTHER_SWIFT_FLAGS` is unset, and `package-mac.sh` calls this script.
- The Xcode CodeSign step fails: "code object is not signed at all, in subcomponent `Contents/Helpers/eastsea-node-daemon.sh`". The postBuild script copies the two daemon shell stubs into `Contents/Helpers` without signing them, after commit 7eedb60. I saw this with ad-hoc signing. The same nested-code rule should fail Developer ID signing too, but check that. The fix: sign the stubs in the postBuild loop, or move them to `Contents/Resources`, keeping in mind that the daemon plist's `BundleProgram` points at them.

**B4. A stale done flag skips the migration, and this Mac has one.** The real `~/Library/Preferences/com.pipln.eastsea.plist` already contains `renameMigrationDone = true` (written 2026-10-04 13:50, the earlier incident), while the live data is back in `Aether/`. On this Mac, 0.7.0 would therefore:
- skip the migration
- create `EastSea/node` empty, where `aether run` mints a **new validator identity**, because `EastSea/node.identity` is absent
- create a **new Secure Enclave wallet key** (a second address), because `mayCreateFreshWalletKey` returns nil once the flag is set

Any tester who ran an earlier EastSea build has the same trap. The fix belongs in code: `migrate()` must not trust the flag while `Aether/node` or an old key handle exists and its new counterpart does not. Add a unit case for it. Before installing on this Mac, run `defaults delete ~/Library/Preferences/com.pipln.eastsea.plist renameMigrationDone` with the full path: plain `defaults … com.pipln.eastsea` reads the 2023 container instead.

## 5. Non-blocking findings

- **M1, medium.** The migration runs on the main thread at `AppDelegate` init. A cross-volume or resume run hashes the whole tree (about 160 s of CPU for 8.5 GB, plus I/O), so the app hangs before any window appears and users are tempted to force-quit. Force-quitting is safe (scenario E), but the hang is bad UX. Move the copy path off the main thread and show progress.
- **M2, medium.** The comments don't match what the code does across volumes. `FileManager.moveItem` does **not** fail across volumes: it copies and then deletes the source (confirmed here). An uninterrupted cross-volume run therefore takes the `.moved` branch, verified by sizes only, deletes the old tree, and copies `run.lock`. This contradicts "verified … every file by content" and "nothing is ever deleted except by a successful same-volume move". Real users are unlikely to hit it, since Application Support is one volume and symlinks are refused. Either compare `volumeIdentifier`s and use the copy path, or correct the comment. Unit test 7's "forced cross-volume" does not exercise the real behaviour.
- **L1, low.** In the copy path, `Aether/node/follow/` stays behind, which is 8.5 GB of disk and also `follow/wallet-node.key`. If the old app ever runs again, two endpoints publish under the same wallet node id. Add `follow/wallet-node.key` to the quarantine list, and consider deleting the old follow DB after verification.
- **L2, low.** `ensure()` runs a full `migrate()` on every `dataDir`/`storeURL` access until it is done. Two concurrent calls contend for the flock and the same files, which gives a transient false "quit the old app" or `.failed` that retries later. Serialize it with a lock or a once-token.
- **L3, low.** Builds into an existing `build/` keep a stale `Helpers/aether.prev`: the codesign rewrite trips the `cmp` in the postBuild script. Build releases from a clean derived-data directory.
- **Note.** About 580 `~/Library/Preferences/rename-migration-test-*.plist` files from earlier unit-test runs hold copies of real 0.6.6 prefs (proveAddress, balance and history blobs). The test calls `removePersistentDomain` but never deletes the files. I removed the 15 my own run created and left the others.

## 6. Hygiene

- The real data was never moved or modified: `Aether/node` is intact and run.lock is still held by pid 90796. The 0.6.6 wallet (pid 90610) and its node were not touched. Nothing ran on poc-m3 or poc-nas, and `~/aether-testnet` was not touched.
- All clones of user data (keys included) are deleted. The disk image is detached and removed. The harness defaults suites are dropped and their plists deleted.
- The built `EastSea.app` was never launched and was unregistered from LaunchServices (`lsregister -u`), so `aether://` and `eastsea://` links cannot open it.
- One deviation: my xcodebuild ran while another agent's `cargo test` was compiling (rustc was running).
- Artifacts in `.claude/worktrees/release-070/tmp/`: `harness/`, `build.log`, `xcodebuild*.log`. The worktree can be removed.
