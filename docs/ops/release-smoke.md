# Launch the shipped app before publishing latest

EastSea 0.7.3 (build 16) crashed on fresh installs in
`NodeController.nodeLogTail`. Rendering screens did not exercise that path:
screen builds deliberately disable the node. These gates launch the app from
the notarized DMG, with its shipped node, and require evidence before the release
can become GitHub's latest release. They do not change or rebuild the app.

Never launch the app on the release/build Mac: first-launch migration can move
the real node folder. Lanes run local script tests only. The lead runs the VM
gate and the canary; the canary host `poc-m3` also runs validator v4. These scripts
must not administer that validator or touch `~/aether-testnet`.

## Publication

```bash
# Builds/packages/notarizes, creates a draft, runs both gates, then publishes latest.
scripts/release-mac.sh --canary-host poc-m3

# Runs both gates but deliberately leaves the release as a draft.
scripts/release-mac.sh --draft --canary-host poc-m3

# Retry a failed/held draft: download its uploaded DMG, rerun both gates, publish.
scripts/release-mac.sh --publish-draft --canary-host poc-m3

# Inspect the publication and host changes without builds, SSH or app launches.
scripts/release-mac.sh --dry-run --canary-host stub-mac
```

`--prepare` still prepares artifacts without publishing. `--publish-prepared`
still finalizes the approved on-chain manifest; its GitHub publication goes
through the same draft and smoke gates. Options can be combined with `--draft`.

Each candidate is created with `--draft --latest=false`. A failed notarization
ticket check, VM gate, canary gate, or DMG fingerprint check returns nonzero and
leaves that draft unpromoted. Only after successful gates does the script run
`gh release edit --draft=false --latest`. The release log is
`dist/release-gates.log`; it includes the candidate SHA-256, timestamp, host,
gate output, evidence paths, and the final publication decision. A retry tests
the uploaded asset itself and does not rebuild or push a tag.

Initial and prepared publications also download the staged draft's DMG and run
both gates on those uploaded bytes. Its SHA-256 must match the fingerprint taken
before upload, so a local file changing while GitHub uploads it cannot turn the
test into approval of a different artifact.

If Tart or the clean base has not been set up, the lead may explicitly use:

```bash
scripts/release-mac.sh --skip-vm-smoke --canary-host poc-m3
```

This prints and records `ALARM: VM SMOKE SKIPPED`, and still requires the full
canary gate. It cannot turn a failing, configured VM gate into a pass. There is
no canary bypass. Once Tart and the base disk/configuration exist, this flag is
refused. A dry run is only a plan, and never counts as a gate pass.

## One-time Tart setup (founder)

The commands below are for the founder to review and run. The scripts do not
install software, pull a base image, or configure the founder's Mac. Allow space
for the Xcode image and disposable VM disks on the external workspace volume.
Keep the host's GUI login/keychain unlocked when running Tart.

Tart's official [quick start](https://tart.run/quick-start/) documents standalone
installation, Sonoma images, and directory sharing. Its
[FAQ](https://tart.run/faq/#automatic-pruning) documents `TART_HOME` and pruning.
Setting `TART_HOME` places images, OCI cache, VM disks and Tart scratch space on
the external disk. The standalone install below also keeps the Tart app there.
Do not use the default `~/.tart` or move/symlink VM storage onto the internal disk.

Run on the build Mac, from the repository root:

```bash
cd /Volumes/workspace/aether-node
/usr/sbin/diskutil info /Volumes/workspace
df -h /Volumes/workspace
# Verify the mounted volume is external before continuing.
mkdir -p "$PWD/tmp/tart-setup"
export TMPDIR="$PWD/tmp/tart-setup"
export TART_HOME=/Volumes/workspace/build-cache/vm
export TART_NO_AUTO_PRUNE=
mkdir -p "$TART_HOME/tools"
curl --fail --location \
  https://github.com/cirruslabs/tart/releases/latest/download/tart.tar.gz \
  --output "$TMPDIR/tart.tar.gz"
tar -xzf "$TMPDIR/tart.tar.gz" -C "$TMPDIR"
/usr/bin/ditto "$TMPDIR/tart.app" "$TART_HOME/tools/tart.app"
export PATH="$TART_HOME/tools/tart.app/Contents/MacOS:$PATH"
tart --version
tart clone ghcr.io/cirruslabs/macos-sonoma-xcode:latest eastsea-smoke-base
tart set eastsea-smoke-base --cpu 4 --memory 8192
tart run eastsea-smoke-base
```

The image's initial `admin` login/password is `admin`. In its Terminal, create
a dedicated VM-only account (the password below is solely for this disposable
VM; never reuse a real password):

```bash
sudo sysadminctl -addUser smoke -fullName 'EastSea release smoke' \
  -password 'EastSea-VM-only' -admin
printf 'smoke ALL=(ALL) NOPASSWD: ALL\n' | \
  sudo tee /etc/sudoers.d/eastsea-release-smoke
sudo chmod 440 /etc/sudoers.d/eastsea-release-smoke
sudo visudo -c
```

In the **guest** System Settings, select `smoke` for automatic login, turn off
screen locking/sleep for that VM, and log in as `smoke`. FileVault must be off
for automatic login. Complete the new user's macOS setup screens. Do not install
or launch EastSea in the base, and do not copy any real wallet/node data into it.
Verify `/usr/bin/python3 --version` works (the Xcode image supplies Python).

In a Terminal logged in as **guest user `smoke`**, install the shared-directory
launcher. This launcher only executes the gate's helper when a disposable clone
boots with the gate's shared directory:

```bash
mkdir -p "$HOME/Library/LaunchAgents" "$HOME/tmp"
touch "$HOME/.eastsea-release-smoke-vm"
cat > "$HOME/Library/LaunchAgents/com.pipln.eastsea.release-smoke.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>com.pipln.eastsea.release-smoke</string>
  <key>ProgramArguments</key>
  <array>
    <string>/bin/bash</string><string>-c</string>
    <string>while [ ! -f '/Volumes/My Shared Files/release-smoke/guest-runner.sh' ]; do /bin/sleep 2; done; exec /bin/bash '/Volumes/My Shared Files/release-smoke/guest-runner.sh'</string>
  </array>
  <key>RunAtLoad</key><true/>
</dict>
</plist>
PLIST
plutil -lint "$HOME/Library/LaunchAgents/com.pipln.eastsea.release-smoke.plist"
sudo -n true
sudo shutdown -h now
```

Keep `eastsea-smoke-base` powered off and clean. The gate clones it twice,
serially; it never runs the base itself. No SSH, sshpass, guest-agent package or
network access to validator hosts is needed. Only the disposable guest accesses
testnet RPC and the normal network through Tart's NAT. If automatic login or the
LaunchAgent is missing, the gate times out and fails; it does not fabricate a
pass. Repeat setup from a clean image if the base becomes contaminated.

## VM gate

```bash
scripts/release-vm-smoke.sh --dry-run dist/EastSea-0.7.3.dmg
scripts/release-vm-smoke.sh dist/EastSea-0.7.3.dmg
```

The two runs use fresh `smoke` profiles from separate clean clones: first an
empty profile, then a profile with a 64 MiB synthetic `node.log`. The gate records
the seeded size before launch; the app normally truncates the log when starting
its node. This preserves evidence that the seeded input was present without
editing the app or its helpers.

The DMG is shared read only and installed into guest `/Applications/EastSea.app`.
The guest verifies the shipped network is 7780 and enables the normal node switch
(`nodeEnabled`) and battery permission (`nodeOnlyOnPower=false`). It disables
automatic updates for the test, so the shipped candidate remains the one under
observation. It never registers the guest as a voting node or enables proving.

For each profile, observe the actual app for at least 600 seconds, check
app/node diagnostic reports, and query the app's node at **guest** `127.0.0.1:18545`. Both
that RPC and the independent reference RPC must report chain 7780. Require the
local height to advance and follow the advancing chain head within 12 blocks,
then quit/relaunch the app and observe it for another 60 seconds. The final
30 seconds must show both local and reference head advancement. Missing final
reference samples fail the gate rather than treating a stalled local node as
healthy. Evidence is retained under repository
`tmp/vm-smoke.<run>/`, including per-profile logs/RPC samples and atomic results.
The helper/configuration share is read only; a separate `release-smoke-output`
share carries writable evidence. The host waits for the helper's atomic exit
status and complete result, so an early or partial PASS file is insufficient.
VMs are stopped on every outcome; only this run's disposable clones are deleted.

The gate refuses unavailable Tart/base storage and internal/symlinked VM disk
locations. Use the founder setup above to fix preflight; never run the candidate
directly on the host as a substitute.

## Canary gate (lead only)

```bash
scripts/release-canary.sh --dry-run dist/EastSea-0.7.3.dmg poc-m3
scripts/release-canary.sh dist/EastSea-0.7.3.dmg poc-m3
```

The script prints each change before doing it. It copies the DMG to a unique
`~/EastSea-canary-backup/<run>/tmp/` on the canary, mounts it read only, stages and
verifies the shipped app, preserves the previous app at
`~/EastSea-canary-backup/<run>/previous/EastSea.app`, replaces
`/Applications/EastSea.app`, and launches it in the logged-in GUI user's session.
The SSH user must be that GUI user, and the host must be a different Mac from the
release host. Noninteractive SSH and `sudo -n` for installation must already work;
the script does not change permissions or configure remote access to fix them.

It observes at least 1,800 seconds and fails if the app disappears/restarts or a
new/changed `~/Library/Logs/DiagnosticReports/EastSea-*` report appears. Existing
reports form a baseline and remain untouched. The backup, DMG and observation
evidence remain for the lead. A failure does not automatically restore or launch
the old app after observation begins; the lead can inspect the evidence and
choose recovery. If the replacement move itself fails after the old app was
preserved, installation cleanup puts that old bundle back without launching it.

It does not edit user preferences, node data, LaunchAgents/Daemons, or
`~/aether-testnet`, and does not stop/restart the validator or its helpers.
Launching/quitting EastSea itself can perform the app's ordinary migration and
node lifecycle actions; therefore this gate is run only on the intended canary
with the lead's authority. Never redirect it to the build Mac.

## Local regression checks

```bash
scripts/test-release-scripts.sh
```

Tests exercise the real publication script with local tool doubles, including
dry runs with stub hosts, successful ordering, failed gates, draft retention,
explicit VM skip logging, and retries using the uploaded artifact. They build
nothing, contact no host, and launch no app. Syntax/static checks complement
these tests; they cannot supply the live 10-minute VM or 30-minute canary evidence
required for a real release.
