#!/usr/bin/env python3
"""Exercise the real canary shell/observer without SSH, mounts, or an app launch.

The embedded remote shell is extracted verbatim, then its fixed app path and
every process/install tool are redirected into a fixture under ROOT/tmp. The
tool doubles only operate on inert files there; uptime advances without sleep.
No installed application, privileged process, or remote host is used.
"""

import hashlib
import os
from pathlib import Path
import re
import shlex
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
SOURCE = ROOT / "scripts/release-canary.sh"
TOOLS = (
    "/usr/bin/uname", "/usr/sbin/ioreg", "/usr/bin/id", "/usr/bin/stat",
    "/usr/libexec/PlistBuddy", "/usr/bin/codesign", "/usr/sbin/spctl",
    "/usr/bin/sudo", "/usr/bin/hdiutil", "/usr/bin/open", "/bin/ps",
    "/usr/bin/osascript", "/bin/sleep", "/bin/kill", "/bin/mv",
    "/usr/bin/ditto", "/bin/mkdir",
)

DRIVER = r'''#!/bin/bash
set -euo pipefail
name=${0##*/}
f=$CANARY_FIXTURE
mode=$CANARY_FIXTURE_MODE
state="$f/state"
app="$f/Applications/EastSea.app"
printf '%s %s\n' "$name" "$*" >> "$f/tool-calls.log"
case "$name" in
  uname) echo Darwin ;;
  ioreg) echo '"IOPlatformUUID" = "1111-AAAA"' ;;
  id)
    if [ "$1" = -u ]; then
      if [ "$mode" = root-user ]; then echo 0; else echo 501; fi
    else echo canary; fi ;;
  stat) if [ "$mode" = wrong-console ]; then echo 502; else echo 501; fi ;;
  PlistBuddy)
    case "$2" in *CFBundleIdentifier*) echo com.pipln.eastsea ;; *) echo EastSea ;; esac ;;
  codesign)
    if [ "$mode" = bad-signature ]; then exit 1; fi
    if [ "$1" = -dv ]; then echo TeamIdentifier=45WU468FZE >&2; fi ;;
  spctl) : ;;
  sudo)
    [ "$1" = -n ]; shift
    case "$mode" in
      writable-no-sudo|sudo-unavailable) echo 'fixture sudo is unavailable' >&2; exit 1 ;;
    esac
    if [ "$1" = -v ]; then exit 0; fi
    # Simulate privilege only inside the fixture. The real installation tools
    # below still reject every path outside this temporary tree.
    /bin/chmod u+w "$f/Applications"
    if [ -d "$app" ]; then /bin/chmod u+w "$app"; fi
    exec "$@" ;;
  hdiutil)
    case "$1" in
      attach)
        mount=''
        while [ "$#" -gt 0 ]; do
          if [ "$1" = -mountpoint ]; then mount=$2; break; fi
          shift
        done
        case "$mount" in "$f"/*) ;; *) exit 98 ;; esac
        /usr/bin/ditto "$f/ship/EastSea.app" "$mount/EastSea.app" ;;
      detach)
        echo detached > "$state/detached"
        if [ "$mode" = detach-failure ]; then exit 1; fi ;;
    esac ;;
  open)
    [ "$1" = "$app" ]
    echo 'fake GUI process' > "$state/running"
    reports="$f/home/Library/Logs/DiagnosticReports"
    case "$mode" in
      new-report) echo 'new crash' > "$reports/EastSea-new.ips" ;;
      classic-report) echo 'classic crash' > "$reports/EastSea_classic.crash" ;;
      changed-report) echo 'modified crash' > "$reports/EastSea-existing.ips" ;;
    esac ;;
  ps)
    if [ ! -f "$state/running" ]; then
      if [ "$2" = -axo ]; then exit 0; else exit 1; fi
    fi
    if [ "$2" = -axo ]; then echo "4242 501 $app/Contents/MacOS/EastSea"
    else
      n=0
      if [ -f "$state/ps-count" ]; then n=$(cat "$state/ps-count"); fi
      n=$((n + 1)); echo "$n" > "$state/ps-count"
      if [ "$mode" = pid-exit ] && [ "$n" -ge 3 ]; then exit 1; fi
      stamp='Sun Oct  9 00:00:00 2026'
      if [ "$mode" = pid-restart ] && [ "$n" -ge 3 ]; then stamp='Sun Oct  9 00:00:10 2026'; fi
      echo "  501 $stamp $app/Contents/MacOS/EastSea"
    fi ;;
  osascript)
    n=-30
    if [ -f "$state/clock" ]; then n=$(cat "$state/clock"); fi
    n=$((n + 30)); echo "$n" > "$state/clock"; echo "$n" ;;
  sleep) : ;;
  kill) /bin/rm -f "$state/running" ;;
  mv|ditto|mkdir)
    # The only real mutations these doubles delegate are fixture file copies,
    # moves, and directory creation. Refuse every path outside this fixture.
    for arg in "$@"; do
      case "$arg" in -*) ;; "$f"/*) ;; *) exit 98 ;; esac
    done
    if [ "$name" = mv ] && [[ "$mode" = *install-failure ]] &&
       [ "${2:-}" = "$app" ] && [ ! -f "$state/move-failed" ]; then
      echo once > "$state/move-failed"; exit 1
    fi
    case "$name" in
      mv) exec /bin/mv "$@" ;;
      ditto) exec /usr/bin/ditto "$@" ;;
      mkdir) exec /bin/mkdir "$@" ;;
    esac ;;
  *) echo "unhandled fixture tool: $name" >&2; exit 98 ;;
esac
'''


def check(condition, message):
    if not condition:
        raise AssertionError(message)


def run(args, env):
    return subprocess.run(args, env=env, text=True, stdout=subprocess.PIPE,
                          stderr=subprocess.STDOUT, timeout=60)


def test_cli(base, source):
    fixture = base / "cli"
    (fixture / "bin").mkdir(parents=True)
    marker = fixture / "forbidden-tool-called"
    for name in ("ssh", "scp", "open", "osascript", "sudo", "ditto", "mv", "kill", "hdiutil", "rsync"):
        tool = fixture / "bin" / name
        tool.write_text('#!/bin/bash\nprintf "%s\\n" "$0" >> "$CANARY_TOOL_MARKER"\nexit 99\n')
        tool.chmod(0o755)
    # Redirect absolute mutation tools too, so a broken dry-run branch cannot
    # escape the rejecting PATH doubles. Local hostname/hash reads stay real.
    for tool in TOOLS:
        if Path(tool).name in ("open", "osascript", "sudo", "ditto", "mv", "kill", "hdiutil"):
            source = source.replace(tool, str(fixture / "bin" / Path(tool).name))
    script = fixture / "release-canary.sh"
    script.write_text(source)
    env = os.environ.copy()
    env.pop("AETHER_CANARY_SECONDS", None)
    env.pop("AETHER_CANARY_POLL_SECONDS", None)
    env.update(PATH=str(fixture / "bin") + ":" + env["PATH"],
               CANARY_TOOL_MARKER=str(marker))
    cases = (
        ("unbuilt DMG", ["--dry-run", "missing-dist/EastSea.dmg", "stub-mac"], {}, 0),
        ("local host", ["--dry-run", "missing.dmg", "localhost"], {}, 1),
        ("loopback", ["--dry-run", "missing.dmg", "user@127.0.0.1"], {}, 1),
        ("root SSH", ["--dry-run", "missing.dmg", "root@stub-mac"], {}, 1),
        ("validator host", ["--dry-run", "missing.dmg", "poc-cuda"], {}, 1),
        ("host injection", ["--dry-run", "missing.dmg", "stub;touch bad"], {}, 1),
        ("short observation", ["--dry-run", "missing.dmg", "stub-mac"], {"AETHER_CANARY_SECONDS": "1799"}, 1),
        ("wide poll", ["--dry-run", "missing.dmg", "stub-mac"], {"AETHER_CANARY_POLL_SECONDS": "31"}, 1),
        ("zero poll", ["--dry-run", "missing.dmg", "stub-mac"], {"AETHER_CANARY_POLL_SECONDS": "0"}, 1),
        ("missing real DMG", ["missing.dmg", "stub-mac"], {}, 1),
    )
    for name, args, overrides, expected in cases:
        case_env = env.copy()
        case_env.update(overrides)
        result = run(["/bin/bash", str(script)] + args, case_env)
        check(result.returncode == expected, name + ": " + result.stdout)
        check(not marker.exists(), name + " invoked a remote/app/install tool")
        check(not (fixture / "tmp").exists(), name + " created a dry-run temp directory")
    print("ok: canary CLI/guards: 10 cases, zero remote/app/install commands")


def test_observer(base, remote, mode):
    fixture = base / mode
    (fixture / "bin").mkdir(parents=True)
    (fixture / "state").mkdir()
    reports = fixture / "home/Library/Logs/DiagnosticReports"
    reports.mkdir(parents=True)
    for directory, label in ((fixture / "Applications/EastSea.app", "old"),
                             (fixture / "ship/EastSea.app", "shipped")):
        (directory / "Contents/MacOS").mkdir(parents=True)
        (directory / "Contents/Info.plist").write_text("inert test fixture")
        (directory / "fixture-label").write_text(label)
    if mode == "changed-report":
        (reports / "EastSea-existing.ips").write_text("baseline crash")
    app = fixture / "Applications/EastSea.app"
    protected = mode in ("protected-app", "protected-applications", "sudo-unavailable", "protected-install-failure")
    if mode in ("protected-app", "sudo-unavailable"):
        app.chmod(0o555)
        check(not os.access(app, os.W_OK), "fixture app protection did not take effect")
    if mode in ("protected-applications", "sudo-unavailable", "protected-install-failure"):
        app.parent.chmod(0o555)
        check(not os.access(app.parent, os.W_OK), "fixture Applications protection did not take effect")
    transformed = remote.replace("app=/Applications/EastSea.app", "app=" + shlex.quote(str(app)))
    for binary in TOOLS:
        tool = fixture / "bin" / Path(binary).name
        tool.write_text(DRIVER)
        tool.chmod(0o755)
        # Invoke the signed system Bash explicitly: executing many uniquely
        # named shebang files directly can stall on macOS script assessment.
        transformed = transformed.replace(binary, "/bin/bash " + shlex.quote(str(tool)))
    check(not re.search(r"(?<![A-Za-z0-9_/.-])/Applications(?:/EastSea\.app)?(?=[/\s\"']|$)", transformed),
          "real app path escaped fixture replacement")
    for binary in TOOLS:
        check(not re.search(r"(?<![A-Za-z0-9_/.-])" + re.escape(binary) + r"(?=[\s\"'])", transformed),
              "real process/install tool escaped replacement: " + binary)
    script = fixture / "remote.sh"
    script.write_text(transformed)
    env = os.environ.copy()
    env.update(HOME=str(fixture / "home"), CANARY_FIXTURE=str(fixture), CANARY_FIXTURE_MODE=mode)
    args = ["fixture-run", hashlib.sha256(b"fake DMG").hexdigest(), "1800", "30",
            "1111-AAAA" if mode == "same-machine" else "FFFF-BBBB"]
    prepared = run(["/bin/bash", str(script), "prepare"] + args, env)
    directory = fixture / "home/EastSea-canary-backup/fixture-run"
    if mode in ("same-machine", "wrong-console", "root-user"):
        check(prepared.returncode != 0 and not directory.exists(), "unsafe target guard wrote run files: " + mode)
    else:
        check(prepared.returncode == 0, mode + " prepare: " + prepared.stdout)
        (directory / "tmp/shipped.dmg").write_bytes(b"fake DMG")
        try:
            observed = run(["/bin/bash", str(script), "observe"] + args, env)
        finally:
            # Restore fixture permissions so TemporaryDirectory can remove it,
            # including when sudo was unavailable or the assertion fails.
            app.parent.chmod(0o755)
            if app.exists():
                app.chmod(0o755)
        success = mode in ("success", "writable-no-sudo", "protected-app", "protected-applications")
        check((observed.returncode == 0) == success, mode + ": " + observed.stdout)
        check((fixture / "state/detached").exists(), mode + " did not detach its fake DMG")
        calls = (fixture / "tool-calls.log").read_text().splitlines()
        sudo_calls = [call for call in calls if call.startswith("sudo ")]
        if mode in ("success", "writable-no-sudo", "install-failure"):
            check(not sudo_calls, mode + " invoked sudo for user-writable installation: " + repr(sudo_calls))
        if protected:
            check(sudo_calls and sudo_calls[0] == "sudo -n -v", mode + " did not check noninteractive sudo")
            check(not any("/ditto " in call for call in sudo_calls), mode + " staged the app with unnecessary sudo")
        if mode in ("protected-app", "protected-applications"):
            check(len(sudo_calls) == 3, mode + " did not use sudo for both installation moves: " + repr(sudo_calls))
        if mode == "sudo-unavailable":
            check("noninteractive sudo is unavailable" in observed.stdout, "missing actionable permission failure: " + observed.stdout)
            check(not any(call.startswith(("kill ", "mv ", "open ")) for call in calls), "permission failure modified or launched the installed app")
            check((app / "fixture-label").read_text() == "old", "permission failure changed the existing app")
        if success:
            check(observed.stdout.index("detach read-only DMG") < observed.stdout.index("PASS: canary"), "PASS preceded detach")
            elapsed = int(re.search(r"survived (\d+)s", observed.stdout).group(1))
            check(elapsed >= 1830, "observation/grace was shortened")
            check((directory / "previous/EastSea.app/fixture-label").read_text() == "old", "previous backup missing")
        else:
            check("FAIL: canary" in observed.stdout and "PASS: canary" not in observed.stdout, mode + " incorrectly passed")
        if mode in ("install-failure", "protected-install-failure"):
            check((app / "fixture-label").read_text() == "old", "failed installation did not restore previous app")
            if protected:
                check(len(sudo_calls) == 4, "privileged rollback did not preserve the chosen install permissions")
    print("ok: isolated canary observer: " + mode)


def main():
    (ROOT / "tmp").mkdir(exist_ok=True)
    source = SOURCE.read_text()
    remote = source.split("<<'REMOTE'\n", 1)[1].split("\nREMOTE\n", 1)[0]
    with tempfile.TemporaryDirectory(prefix="release-canary-test.", dir=ROOT / "tmp") as temporary:
        base = Path(temporary)
        parsed = base / "remote-original.sh"
        parsed.write_text(remote)
        check(run(["/bin/bash", "-n", str(SOURCE)], os.environ.copy()).returncode == 0, "canary does not parse")
        check(run(["/bin/bash", "-n", str(parsed)], os.environ.copy()).returncode == 0, "embedded observer does not parse")
        test_cli(base, source)
        for mode in ("same-machine", "wrong-console", "root-user", "success", "writable-no-sudo",
                     "protected-app", "protected-applications", "sudo-unavailable", "protected-install-failure", "new-report",
                     "classic-report", "changed-report", "pid-exit", "pid-restart",
                     "install-failure", "detach-failure", "bad-signature"):
            test_observer(base, remote, mode)


if __name__ == "__main__":
    main()
