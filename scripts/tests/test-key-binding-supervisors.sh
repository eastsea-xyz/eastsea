#!/bin/bash
# Hermetic exit-15 regressions for the shell supervision boundaries. All
# generated agents, binaries, markers and logs stay under this checkout's tmp.
# --baseline REF runs the same assertions against that revision's scripts.
set -eu
root=$(cd "$(dirname "$0")/../.." && pwd)
mkdir -p "$root/tmp"
export TMPDIR="$root/tmp"
python3 - "$root" "$@" <<'PY'
import argparse
import os
import pathlib
import plistlib
import shlex
import subprocess
import sys
import tempfile
import time

root = pathlib.Path(sys.argv[1])
parser = argparse.ArgumentParser()
parser.add_argument("--baseline")
opts = parser.parse_args(sys.argv[2:])
work = pathlib.Path(tempfile.mkdtemp(prefix="key-binding-supervisors-", dir=root / "tmp"))
scripts = work / "scripts"
stubs = work / "stubs"
scripts.mkdir()
stubs.mkdir()
failures = []


def check(condition, message):
    print(("ok   " if condition else "FAIL ") + message, flush=True)
    if not condition:
        failures.append(message)


def source(relative):
    if opts.baseline:
        result = subprocess.run(
            ["git", "-C", str(root), "show", f"{opts.baseline}:{relative}"],
            text=True, capture_output=True,
        )
        if result.returncode:
            return None
        return result.stdout
    return (root / relative).read_text()


def write_script(path, text):
    path.write_text(text)
    path.chmod(0o755)


def replace_required(text, original, replacement):
    if text.count(original) != 1:
        raise RuntimeError(f"fixture isolation anchor changed: {original!r}")
    return text.replace(original, replacement)


def agent_source(relative):
    return replace_required(source(relative), 'LA="$HOME/Library/LaunchAgents"',
                            f"LA={shlex.quote(str(agents))}")


write_script(stubs / "codesign", "#!/bin/bash\nexit 0\n")
write_script(stubs / "launchctl", "#!/bin/bash\nexit 0\n")
write_script(scripts / "testnet.sh", "#!/bin/bash\nexit 0\n")
write_script(stubs / "sudo", '#!/bin/bash\n[ "$1" = -u ] && shift 2\nexec "$@"\n')
write_script(stubs / "caffeinate", '''#!/bin/bash
[ "$1" = -s ] && shift
[ "${1:-}" = -w ] && exit 0
exec "$@"
''')
node = stubs / "aether"
write_script(node, '''#!/bin/bash
printf 'launch\n' >> "$F7_LAUNCH_LOG"
if [ "$F7_CHILD_EXIT" = TERM ]; then kill -TERM $$; fi
exit "$F7_CHILD_EXIT"
''')
env = dict(os.environ, PATH=f"{stubs}:{os.environ['PATH']}", TMPDIR=str(root / "tmp"))

# Substitute only the caffeinate dependency in the copied shared wrapper.
# Production supervisors get no environment-controlled privilege-path seams.
wrapper = source("scripts/aether-launchd-wrapper.sh")
if wrapper is not None:
    write_script(scripts / "aether-launchd-wrapper.sh", wrapper.replace(
        "/usr/bin/caffeinate", str(stubs / "caffeinate")
    ))


def launches(log):
    return len(log.read_text().splitlines()) if log.exists() else 0


def run_launchd(plist_path, child_exit, refused=False):
    config = plistlib.loads(plist_path.read_bytes())
    args = [str(stubs / "caffeinate") if a == "/usr/bin/caffeinate" else a
            for a in config["ProgramArguments"]]
    log = work / f"{plist_path.parent.name}-{plist_path.stem}-{child_exit}-refused{refused}.launches"
    data = work / (log.stem + "-data")
    data.mkdir()
    data_index = args.index("--data") + 1
    args[data_index] = str(data)
    refusal = data / "key-binding-refused"
    if refused:
        refusal.write_text("fixture proven mismatch")
    output = ""
    codes = []
    for _ in range(3):
        child_env = dict(env, F7_LAUNCH_LOG=str(log), F7_CHILD_EXIT=str(child_exit))
        result = subprocess.run(args, env=child_env, text=True, capture_output=True, timeout=5)
        codes.append(result.returncode)
        output += result.stdout + result.stderr
        policy = config.get("KeepAlive", False)
        restart = policy is True or (
            isinstance(policy, dict) and policy.get("SuccessfulExit") is False
            and result.returncode != 0
        )
        if not restart:
            break
    if refused:
        check(refusal.read_text() == "fixture proven mismatch", f"F7 {plist_path.stem} preserves refusal marker")
    return launches(log), codes, output


def check_launchd(plist_path, label, full=False):
    config = plistlib.loads(plist_path.read_bytes())
    valid_args = isinstance(config["ProgramArguments"], list)
    check(valid_args, f"F7 {label} has runnable ProgramArguments array")
    if not valid_args:
        return
    count, codes, output = run_launchd(plist_path, 15)
    check(count == 1, f"F7 {label} exit 15 launches={count}; expected 1")
    check("automatic restart disabled" in output, f"F7 {label} logs terminal refusal")
    if full:
        for status, description in [("TERM", "SIGTERM"), (1, "crash"), (0, "unexpected clean exit")]:
            count, codes, _ = run_launchd(plist_path, status)
            check(count == 3, f"F7 {label} {description} launches={count}; expected 3")
    count, codes, output = run_launchd(plist_path, 1, refused=True)
    check(count == 0, f"F7 {label} fresh bootstrap with refusal launches={count}; expected 0")
    check(codes == [0] and "automatic restart disabled" in output,
          f"F7 {label} fresh bootstrap stops successfully with diagnostic")


# The shared wrapper must select the node's actual data directory in either
# CLI spelling, and refuse malformed flags before it can spawn a child.
if wrapper is not None:
    for index, suffix in enumerate((["--data"], ["--data="], [], ["--data", "--network", "fixture"],
                                    ["--data", ""], ["--data=one", "--data=two"])):
        log = work / f"generic-malformed-{index}.launches"
        result = subprocess.run(["/bin/bash", str(scripts / "aether-launchd-wrapper.sh"), str(node), "run", *suffix],
                                env=dict(env, F7_LAUNCH_LOG=str(log), F7_CHILD_EXIT="1"),
                                text=True, capture_output=True, timeout=5)
        check(result.returncode != 0 and launches(log) == 0,
              f"F7 generic wrapper rejects malformed/missing data {index} before spawn")
    equals_data = work / "equals-data"
    equals_data.mkdir()
    (equals_data / "key-binding-refused").write_text("fixture proven mismatch")
    log = work / "generic-equals-refused.launches"
    result = subprocess.run(["/bin/bash", str(scripts / "aether-launchd-wrapper.sh"), str(node), "run", f"--data={equals_data}"],
                            env=dict(env, F7_LAUNCH_LOG=str(log), F7_CHILD_EXIT="1"),
                            text=True, capture_output=True, timeout=5)
    check(result.returncode == 0 and launches(log) == 0,
          f"F7 generic wrapper --data= fresh bootstrap with refusal launches={launches(log)}; expected 0")


# Exercise the real generation scripts, replacing the LaunchAgents destination
# in isolated source copies. HOME, launchctl, codesign and real data stay intact.
agents = work / "agents"
agents.mkdir()
testnet = work / "testnet"
(testnet / "bin").mkdir(parents=True)
(testnet / "1").mkdir()
write_script(testnet / "bin" / "aether", node.read_text())
text = agent_source("scripts/testnet-launchagent.sh")
text = replace_required(text, '"$HOME"/.config/aether/devicecheck/',
                        shlex.quote(str(work / "devicecheck")) + "/")
write_script(scripts / "testnet-launchagent.sh", text)
result = subprocess.run(["/bin/bash", str(scripts / "testnet-launchagent.sh"), "install"],
                        env=dict(env, AETHER_TESTNET=str(testnet)), text=True, capture_output=True)
check(result.returncode == 0, f"F7 fixture testnet agents generated: {result.stderr.strip()}")
testnet_agent = agents / "com.pipln.aether.testnet.v1.plist"
check_launchd(testnet_agent, "testnet LaunchAgent", full=True)

reserve = work / "reserve"
members = []
for i in range(1, 4):
    data = reserve / str(i)
    data.mkdir(parents=True)
    (data / "validator.key").write_text("fixture only")
    key = f"fixture-key-{i}"
    (data / "validator.pub.json").write_text('{"key": "' + key + '"}')
    members.append({"key": key})
import json
network = work / "network.json"
network.write_text(json.dumps({"identity": "fixture", "reserve": {"validators": members}}))
text = agent_source("scripts/reserve-keys.sh")
write_script(scripts / "reserve-keys.sh", text)
result = subprocess.run(["/bin/bash", str(scripts / "reserve-keys.sh"), "install", str(network)],
                        env=dict(env, AETHER_RESERVE=str(reserve), AETHER_BIN=str(node)),
                        text=True, capture_output=True)
check(result.returncode == 0, f"F7 fixture reserve agents generated: {result.stderr.strip()}")
for i in range(1, 4):
    check_launchd(agents / f"com.pipln.eastsea.reserve.{i}.plist", f"reserve LaunchAgent {i}", full=i == 1)

# Convert both legacy caffeinate-wrapped and already-wrapped agents. The latter
# must not gain a nested wrapper that turns the terminal success back into retry.
legacy = work / "legacy-agents"
legacy.mkdir()
legacy_plist = plistlib.loads(testnet_agent.read_bytes())
legacy_plist["Label"] = "com.pipln.aether.testnet.legacy"
legacy_plist["ProgramArguments"] = ["/usr/bin/caffeinate", "-s", str(node), "run", "--data", str(work / "legacy-data"), "--exit-with-parent"]
legacy_plist["KeepAlive"] = True
(legacy / "com.pipln.aether.testnet.legacy.plist").write_bytes(plistlib.dumps(legacy_plist))
(legacy / testnet_agent.name).write_bytes(testnet_agent.read_bytes())
daemons = work / "daemons"
write_script(scripts / "install-validator-daemons.sh", source("scripts/install-validator-daemons.sh"))
result = subprocess.run(["/bin/bash", str(scripts / "install-validator-daemons.sh"), "--dry-run",
                         "--agents-dir", str(legacy), "--daemons-dir", str(daemons)],
                        env=env, text=True, capture_output=True)
check(result.returncode == 0, f"F7 fixture daemons converted: {result.stderr.strip()}")
for path in sorted(daemons.glob("*.plist")):
    check_launchd(path, f"converted daemon {path.stem}", full="legacy" in path.stem)
    check("--exit-with-parent" not in plistlib.loads(path.read_bytes())["ProgramArguments"],
          f"F7 converted daemon drops login parent lifetime ({path.stem})")

# Run the production daemon loop against one marker and a fake sudo wrapper.
# Path substitution isolates its fixed /Users privilege boundary from the host.
daemon_users = work / "Users"
marker = daemon_users / "fixture" / "Library" / "Application Support" / "EastSea" / "node" / "unattended.plist"
marker.parent.mkdir(parents=True)
bundle = work / "fixture.app"
resource = bundle / "Contents" / "Resources"
resource.mkdir(parents=True)
write_script(resource / "eastsea-node-wrapper.sh", f"#!/bin/bash\nexec {shlex.quote(str(node))}\n")
marker.write_bytes(plistlib.dumps({"user": "fixture", "bundle": str(bundle), "data": str(marker.parent)}))
text = source("apps/wallet/Helpers/eastsea-node-daemon.sh")
text = replace_required(text, "POLL=30", "POLL=0.02")
text = replace_required(text,
    "/Users/*/Library/Application\\ Support/EastSea/node/unattended.plist", shlex.quote(str(marker))
)
text = replace_required(text, '"/Users/$user/"', f'"{daemon_users}/$user/"')
text = replace_required(text, "/usr/bin/sudo", str(stubs / "sudo"))
daemon = scripts / "eastsea-node-daemon.sh"
write_script(daemon, text)
for status in (15, "TERM", "REFUSED"):
    log = work / f"daemon-{status}.launches"
    refusal = marker.parent / "key-binding-refused"
    if status == "REFUSED":
        refusal.write_text("fixture proven mismatch")
    process = subprocess.Popen(["/bin/bash", str(daemon)], env=dict(
        env, F7_LAUNCH_LOG=str(log), F7_CHILD_EXIT="1" if status == "REFUSED" else str(status)),
        text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    deadline = time.monotonic() + 5
    while process.poll() is None and launches(log) < 3 and time.monotonic() < deadline:
        time.sleep(0.01)
    if process.poll() is None:
        process.terminate()
    out, err = process.communicate(timeout=5)
    count = launches(log)
    if status == 15:
        check(count == 1, f"F7 unattended daemon exit 15 launches={count}; expected 1")
        check(process.returncode == 0, "F7 unattended daemon terminal refusal exits successfully for launchd")
        check("automatic restart disabled" in out + err, "F7 unattended daemon logs terminal refusal")
    elif status == "TERM":
        check(count >= 2, f"F7 unattended daemon SIGTERM launches={count}; expected at least 2")
    else:
        check(count == 0 and process.returncode == 0,
              f"F7 unattended daemon fresh start with refusal launches={count}; expected 0")
        check("automatic restart disabled" in out + err, "F7 unattended daemon logs persisted refusal")
        check(refusal.read_text() == "fixture proven mismatch", "F7 unattended daemon preserves refusal marker")

# Exercise the actual user wrapper, including its marker-to-argv agreement.
user_text = source("apps/wallet/Helpers/eastsea-node-wrapper.sh").replace(
    "/usr/bin/caffeinate", str(stubs / "caffeinate")
)
user_wrapper = scripts / "eastsea-node-wrapper.sh"
write_script(user_wrapper, user_text)
for name, status in (("normal", 1), ("signal", "TERM"), ("refused", 1), ("missing", 1), ("malformed", 1), ("disagrees", 1)):
    data = work / ("user-wrapper-" + name)
    data.mkdir()
    marker_path = data / "unattended.plist"
    details = {"data": str(data), "binary": str(node), "prove": "", "argv": ["run", "--data", str(data)]}
    if name == "missing":
        del details["data"]
    elif name == "malformed":
        details["data"] = ""
    elif name == "disagrees":
        details["argv"] = ["run", "--data", str(work / "different-data")]
    elif name == "refused":
        (data / "key-binding-refused").write_text("fixture proven mismatch")
    marker_path.write_bytes(plistlib.dumps(details))
    log = work / ("user-wrapper-" + name + ".launches")
    result = subprocess.run(["/bin/bash", str(user_wrapper), str(marker_path)],
                            env=dict(env, F7_LAUNCH_LOG=str(log), F7_CHILD_EXIT=str(status)),
                            text=True, capture_output=True, timeout=5)
    if name in ("normal", "signal"):
        expected_status = -15 if name == "signal" else 1
        check(launches(log) == 1 and result.returncode == expected_status,
              f"F7 user wrapper {name} without refusal launches={launches(log)}; expected 1")
    elif name == "refused":
        check(launches(log) == 0 and result.returncode == 15,
              f"F7 user wrapper with refusal launches={launches(log)}; expected 0 and status 15")
        check(not (data / "unattended.pid").exists(), "F7 refused user wrapper writes no false pid")
        check((data / "key-binding-refused").read_text() == "fixture proven mismatch", "F7 user wrapper preserves refusal marker")
    else:
        check(launches(log) == 0 and result.returncode != 0,
              f"F7 user wrapper rejects {name} data before spawn")

print(f"{'PASS' if not failures else 'FAIL'}: {len(failures)} failures — {work}")
sys.exit(bool(failures))
PY
