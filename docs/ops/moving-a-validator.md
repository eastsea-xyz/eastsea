# Moving or restoring a validator on another Mac

The node binds its validator public key to this Mac's hardware ID. A successful
read of a different ID stops signing with exit 15. An unavailable read keeps
the process running with **“waiting to confirm this Mac”** and retries with
backoff; wait for confirmation rather than rebinding for that condition.

`aether keys rebind --data <dir>` is owner recovery for an intentional move or
logic-board replacement. It preserves the validator key, node account,
threshold share, and committee identity. It does not retire the old Mac, prove
that every old copy is offline, repair a stale share or vote journal, perform
DeviceCheck registration, or replace the network's committee.

**Only do this if you moved this Mac's node on purpose; running the same keys
on two Macs gets the validator slashed.** Rebinding is never an automatic
response to a failed hardware read.

## Planned move

1. Record the validator's public address (`validator.pub.json` → `key`),
   chain/committee identity, latest threshold round, and finalized height/hash.
   Confirm the remaining committee can finalize while this validator is down.
2. Stop the old node and every wallet, supervisor, LaunchAgent, and daemon that
   can start it. Disable unattended startup and verify `run.lock` is no longer
   held. Keep the old Mac offline throughout the move. Record the last signed
   view and finalized checkpoint. `validator move-out`/`move-in` are design
   proposals, not shipped commands; there is no automatic retirement record.
3. Make an offline snapshot of the complete identity and safety state:
   `validator.key`, `validator.pub.json`, `node-account.key`, `key-binding.json`,
   the latest seated `threshold.json`, `network.json`, any ceremony record,
   pending reshare/handoff state, and all vote journals (`vote-*`), including
   any pre-recovery records. Carry the sibling `<data>.identity` marker too.
   Do not delete or reset vote journals or restore a share from an older round.
   Check [consensus recovery](consensus-recovery.md) and preserve its safeguards.
4. After the old signer is stopped, observe the surviving validators finalize
   beyond its recorded last signed view. Complete any in-flight handoff or
   reshare first. If that cannot be established, stop and resolve committee
   recovery rather than starting a second signer from a stale snapshot.
5. Transfer the snapshot securely to an owner-controlled directory on the new
   Mac's internal disk. Verify file hashes, private-file mode 0600, and the
   same public validator address. Keep every destination startup job disabled.
   Do not move keys onto the external chain-data disk.
6. On the new Mac, in the owner's interactive terminal, run:

   ```sh
   aether keys rebind --data /path/to/node
   ```

   Read the warning and type the displayed 64-hex-digit validator address
   (its Ed25519 public key, not the wallet account address). Pipes and a
   confirmation such as `yes` are refused. The command verifies ownership,
   takes `run.lock` and the creation lock, reads the current hardware ID,
   atomically replaces the binding, and fsyncs the file and directory.
   `key-rebind.log` and terminal output record old → new hardware hashes.
   A storage/durability error is a failure; retain the diagnostics.
7. Start only the destination node. Check the public address, threshold round,
   chain identity, checkpoint, and resumed finality. Retire the old startup
   configuration and keep its key snapshot offline. A confirmed exit 15 never
   automatically restarts through the wallet or launchd wrappers; owner
   recovery must deliberately start the node again.

   A proved mismatch leaves `key-binding-refused` in the key directory, so an
   attached wallet or a later app launch cannot replay refused keys. Successful
   owner rebind clears it durably. Do not erase that marker to skip recovery.

In EastSea, the same recovery appears only on the **keys came from another
Mac** stop screen. It requires the typed validator address and the existing
Secure Enclave owner authentication (Touch ID). The **waiting to confirm this
Mac** state has no rebind action and does not trigger watchdog restarts.

## Lost hardware or board replacement

Establish that the old hardware cannot sign or be restarted, then follow the
snapshot, journal, share, and confirmation steps above. A complete current
backup restores the existing seat; registering a new key does not restore it.
If the latest share or vote-safety state is missing, follow committee recovery
instead. Rebinding alone cannot make an unsafe or stale backup safe.

## v4 move on 2026-10-06

The [7780 upgrade runbook](testnet-7780-upgrade.md) records that v4 moved from
Mac Studio to `poc-m3` on 2026-10-06. The old Mac Studio `4/` directory is a
stale copy; it must never be included in a local-validator restart loop. v4's
active startup job, RPC port 8604, and `node4.log` are on `poc-m3`.

That move predates hardware binding. If its destination keys are still
unbound, the first 0.7.3 start creates their binding on `poc-m3`; it does not
retroactively reject the completed legacy move. Before that first start,
confirm the Mac Studio v4 job and every stale-copy startup path remain retired.
An already-bound v4 backup moved later requires the explicit stopped-owner
`keys rebind` procedure above, with its original vote journals and current
threshold share. Do not run the old `testnet-upgrade.sh` loop against the stale
Mac Studio `4/` directory; follow the versioned, one-validator-at-a-time gates
in the upgrade runbook.

These are instructions for an authorized maintenance window. This change's
tests used isolated fixtures; they did not move or operate live validators.
