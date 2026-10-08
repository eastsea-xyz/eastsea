#!/usr/bin/env bash
# Exercise the actual unattended wrapper with a task-owned argv recorder,
# never the EastSea app, a real node binary or the person's node folder.
set -euo pipefail
repo_root=$(cd "$(dirname "$0")/../../../.." && pwd)
fixture_root="$repo_root/tmp/country-wrapper-$$"
mkdir -p "$fixture_root"
export TMPDIR="$fixture_root"
python3 - "$repo_root" "$fixture_root" <<'PY'
import os
import plistlib
import subprocess
import sys
from pathlib import Path

repo = Path(sys.argv[1])
fixtures = Path(sys.argv[2])
wrapper = repo / "apps/wallet/Helpers/eastsea-node-wrapper.sh"
binary = Path("/bin/bash")

def run(name, country_flags, choice=None, country=None):
    data = fixtures / name
    data.mkdir()
    # Use an existing system executable. A newly created executable script
    # can stall during macOS execution assessment before its body runs.
    (data / "run").write_text('printf \'%s\\n\' run "$@" > "$ARGV_CAPTURE"\n')
    capture = data / "actual-argv.txt"
    marker = data / "unattended.plist"
    base = ["run", "--data", str(data), "--rpc-port", "18545", "--port", "19101"]
    payload = {"data": str(data), "binary": str(binary), "prove": "", "argv": base + country_flags}
    if choice is not None:
        payload["presence_country_choice"] = choice
    if country is not None:
        payload["presence_country"] = country
    marker.write_bytes(plistlib.dumps(payload))
    environment = dict(os.environ, ARGV_CAPTURE=str(capture))
    stdout_path = data / "wrapper.stdout.log"
    stderr_path = data / "wrapper.stderr.log"
    # caffeinate inherits these handles and waits for the wrapper PID to be
    # reaped. Files let run.wait() reap promptly; PIPE would wait for EOF first.
    with stdout_path.open("wb") as output, stderr_path.open("wb") as errors:
        result = subprocess.run(["/bin/bash", str(wrapper), str(marker)], env=environment,
                                cwd=fixtures, stdout=output, stderr=errors, timeout=10)
    assert result.returncode == 0, (name, result.returncode, stderr_path.read_text())
    actual = capture.read_text().splitlines()
    return base, actual

for mode in ["default-on", "ask-before-sending"]:
    for form, flags in [("inline", ["--presence-country=KR"]), ("separate", ["--presence-country", "KR"])]:
        base, actual = run(f"{mode}-legacy-{form}", flags)
        assert actual == base, ("old unconsented marker leaked country", mode, form, actual)
    base, actual = run(f"{mode}-pending", ["--presence-country=KR"], "", "")
    assert actual == base, ("pending screen leaked country", mode, actual)
    base, actual = run(f"{mode}-decline", ["--presence-country=KR"], "decline", "")
    assert actual == base, ("revoked marker restored country", mode, actual)
    base, actual = run(f"{mode}-share", ["--presence-country=KR"], "share", "KR")
    assert actual == base + ["--presence-country=KR"], ("answered marker lost authorized preference", mode, actual)
    base, actual = run(f"{mode}-mismatched", ["--presence-country=US"], "share", "KR")
    assert actual == base, ("marker leaked an unauthorized country", mode, actual)
    base, actual = run(f"{mode}-unknown-answer", ["--presence-country=KR"], "yes", "KR")
    assert actual == base, ("unknown choice became implicit consent", mode, actual)

print("ok")
PY
