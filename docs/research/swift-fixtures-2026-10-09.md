# R07/R11 wallet fixture reliability — 2026-10-09

Base: `613d079`, branch `codex/swift-fixtures`, development Mac, macOS 26.2 (25C56). Changes are confined to the two native fixture scripts and three Swift test files. No production source, prohibited crate, prover, wallet build, application launch, remote host, or push was involved.

## Reproduction and evidence

Each initial test ran separately with the compile semaphore in the same build shell. All three unchanged tests passed. The block-data invocation used the unchanged `run block-data` source table/function extracted from `scripts/test-swift-pure.sh`; other suites and native fixtures were excluded. Five additional executions of its baseline binary also passed.

Exact initial stdout/stderr and exit statuses are retained under `tmp/swift-fixtures-evidence/baseline-*`. The original lane logs were copied there as `prior-*` without changing their worktrees.

The successful baseline daemon run exited 0 while stderr contained:

```text
/Volumes/workspace/aether-node/.claude/worktrees/swift-fixtures/tmp/R11-node-identity.EHpjp0/R11-old: replacing existing signature
/Volumes/workspace/aether-node/.claude/worktrees/swift-fixtures/tmp/R11-node-identity.EHpjp0/R11-new: replacing existing signature
```

`--force` was already present. The script tests exit status, not whether stderr is empty. The earlier update-noupdate directory had signed `R11-old`/`R11-new` but no `R11-check`, and its stdout was empty. These artifacts contain no evidence of checker execution and are consistent with a stop before it; they do not establish why the checker was missing. A compiler failure, interruption, or resource exhaustion remains unproven.

The retained storage-defaults failure was exactly:

```text
FAIL R11 listener fixture: Error Domain=R11Fixture Code=1 "listener did not become ready" UserInfo={NSLocalizedDescription=listener did not become ready}
```

Both retained helper signatures verified successfully and the original helper pair launched successfully on this run. Delaying the listener before bind/listen by six seconds deterministically reproduces the old five-second timeout. The fixed fixture passes the same delay. This proves the old startup allowance was insufficient for that controlled schedule; it does not prove host load caused the historical timeout.

The retained storage-reward-v2 failure was exactly:

```text
FAIL R07 round trip preserves the preexisting internal endpoint key
```

The R07 fixture waited for `follow/state.db` to disappear before reversing the move. Production cleanup unlinks that file, syncs directories, and only then saves `cleanupDone`. Delaying the directory-sync boundary by 250 ms in a temporary copy of the unchanged production source reproduced:

```text
TRACE R07: source state unlinked; cleanupDone still false; delaying directory sync 250 ms
FAIL R07 round trip preserves the preexisting internal endpoint key
```

The retained failing fixture had `committed=true`, `cleanupDone=false`, no source state file, and the exact original `b'internal-endpoint-key'` bytes. The assertion failed because reverse copy encountered the unfinished outward record and retained its external selection; this was not evidence of key loss. Waiting for the completed record passes under the same delay. The temporary instrumented source is never committed; production behavior is unchanged.

The first host observation was load averages `20.43 30.44 30.51` and swap usage `16371.75M` of `17408.00M`. Load was recorded, not identified as an observed OOM/compiler termination. TMPDIR remained on the workspace volume throughout. External-volume effects, port reuse, date and timezone were not established as historical causes.

## Changes

- `apps/wallet/Tests/block-data/main.swift`: wait for the committed record's `cleanupDone` and expected target before reverse moves; replace fixed cleanup sleeps with completion checks; run cleanup-policy replay assertions synchronously; use a monotonic 30-second deadline with an explicit target/file message. Restate the locked in-memory preferences stub's unchecked Sendable conformance to eliminate its warning.
- `apps/wallet/Tests/update-daemon/main.swift`: await a helper-written PID/label readiness file before checking loaded identity, under a monotonic 30-second bound.
- `apps/wallet/Tests/update-daemon-tree/main.swift`: await complete PID/port readiness files asynchronously; wait for a new PID on the same port after replacement; require both a fresh B response and actual replacement so nil results cannot make the proof pass vacuously. Report supervisor status, readiness path and last contents on failure.
- `scripts/test-update-daemon.sh`: publish helper readiness atomically; identify the failing build/check stage and preserve its exit status. Keep successful codesign warnings visible and use the workspace Swift module cache.
- `scripts/test-update-listener.sh`: publish readiness atomically, report child/exec/socket failures, reap an unexpectedly exited child, protect fork/PID publication from termination signals, and provide stage/exit/work diagnostics. Continue using ephemeral loopback ports and SO_REUSEADDR for the deliberate same-port replacement.

Polling sleeps only pace predicate checks; they do not stand in for readiness or cleanup completion. No dependencies were added.

## Verification

| Fixed test | Consecutive runs | Exit statuses |
| --- | --- | --- |
| `scripts/test-update-daemon.sh` | 5/5 passed | 0, 0, 0, 0, 0 |
| `scripts/test-update-listener.sh` | 5/5 passed | 0, 0, 0, 0, 0 |
| Isolated block-data pure Swift test | 5/5 passed | 0, 0, 0, 0, 0 |

The five-run checks rebuilt and ran each script sequentially, one test at a time. Every build shell called `~/.claude/playbooks/aether-team/wait-compile.sh` and remained alive until its test ended. The controller had a 20-minute compile/test cap. Raw logs, per-run status files and `five-pass-results.json` are under `tmp/swift-fixtures-evidence` and excluded from the commit.

For reverted checks, the five original files were materialized with `git show 613d079:<path>` under `tmp/swift-fixtures-evidence/base/`; the source worktree stayed at the fix. Baseline and fixed versions used the same bounded fault controls:

| Control | Reverted result | Fixed result |
| --- | --- | --- |
| R07: 250 ms delay after source unlink, before sync/record publication | Exit 1, exact prior R07 round-trip failure | Exit 0, `OK block-data` |
| R11 listener: six-second startup delay before bind/listen | Exit 1, exact prior readiness failure | Exit 0, all assertions passed |
| R11 daemon: silent Swift compiler exits 23 | Exit 23, only signature-replacement output; diagnostic assertion fails | Expected exit 23 with phase, status and work path; diagnostic assertion passes |

The daemon control deliberately substitutes a silent Swift compiler that exits 23. Both versions correctly fail; only the fixed wrapper identifies `phase=compile Swift checker exit=23`. The diagnostic contract therefore fails when reverted. This is a fault-injection regression for failure reporting, not reproduction or proof of a fix for the historical daemon stoppage. Its historical cause remains unresolved.

`bash -n`, ShellCheck and `git diff --check` passed. Compiling/running the three fixtures provides their Swift typecheck and integration checks. The native compiler warnings seen in the baseline runs are absent from the fixed runs. Final cleanup confirmed no task-created native fixture processes remained.

## Limits

The ordinary baseline failures were intermittent and did not occur in the initial runs. The R07 race and listener timeout were reproduced under controlled schedules; the daemon's historical stoppage could not be reproduced naturally. Do not interpret the controlled daemon negative check as proof that its original cause was fixed. No real product bug was demonstrated, and no product fix was made.
