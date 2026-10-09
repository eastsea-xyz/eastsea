"""Release publication regressions using local tool doubles, never a real app/host."""

import hashlib
import json
import os
from pathlib import Path
import shlex
import shutil
import signal
import subprocess
import sys
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]
VERSION = "0.7.99"
TAG = "app-v" + VERSION
DMG_NAME = "EastSea-" + VERSION + ".dmg"
LOCAL_DMG = b"local prepared candidate\n"
STAGED_DMG = b"exact candidate uploaded to the draft\n"

# All external surfaces are replaced, including commands that would otherwise
# publish, build, mount, connect to a host, or launch the app. The events retain
# artifact hashes so a downloaded-draft retry must consume the uploaded bytes.
TOOL_DOUBLE = r'''
import hashlib, json, os
from pathlib import Path
import sys

tool = Path(sys.argv[1]).name
args = sys.argv[2:]
event = {"tool": tool, "args": args}
if tool in ("release-vm-smoke.sh", "release-canary.sh") and args != ["--check-setup"]:
    candidate = Path(args[0])
elif tool == "xcrun" and args[:2] == ["stapler", "validate"]:
    candidate = Path(args[2])
else:
    candidate = None
if candidate is not None and candidate.is_file():
    event["sha256"] = hashlib.sha256(candidate.read_bytes()).hexdigest()
with open(os.environ["SMOKE_TEST_EVENTS"], "a") as stream:
    stream.write(json.dumps(event) + "\n")

def option(name):
    return args[args.index(name) + 1]

def refuse():
    print("fixture refuses external or unexpected operation: " + tool + " " + repr(args), file=sys.stderr)
    sys.exit(97)

if tool == "gh":
    if args[:2] == ["release", "view"]:
        print(os.environ.get("SMOKE_TEST_DRAFT_STATE", "true"))
    elif args[:2] == ["release", "download"]:
        directory = Path(option("--dir"))
        directory.mkdir(parents=True, exist_ok=True)
        pattern = option("--pattern")
        if pattern == "appcast.xml":
            (directory / pattern).write_text('<rss xmlns:sparkle="http://www.andymatuschak.org/xml-namespaces/sparkle"><channel><item><sparkle:shortVersionString>0.6.7</sparkle:shortVersionString><enclosure url="https://example.invalid/Aether-0.6.7.dmg" /></item></channel></rss>')
        elif pattern == os.environ["SMOKE_TEST_DMG_NAME"]:
            if os.environ.get("SMOKE_TEST_MISSING_DOWNLOAD") != "1":
                uploaded = Path(os.environ["SMOKE_TEST_STAGED_ASSET"])
                (directory / pattern).write_bytes(uploaded.read_bytes() if uploaded.exists()
                                                else b"exact candidate uploaded to the draft\n")
        else:
            refuse()
    elif args[:2] == ["release", "create"]:
        candidate = next(Path(arg) for arg in args if arg.endswith(".dmg") and Path(arg).is_file())
        if os.environ.get("SMOKE_TEST_PRE_UPLOAD_MUTATE") == "1":
            candidate.write_bytes(b"local candidate changed before upload read\n")
        uploaded = Path(os.environ["SMOKE_TEST_STAGED_ASSET"])
        uploaded.write_bytes(candidate.read_bytes())
        if os.environ.get("SMOKE_TEST_STAGED_MUTATE") == "1":
            uploaded.write_bytes(b"uploaded asset changed\n")
        if os.environ.get("SMOKE_TEST_LOCAL_AFTER_UPLOAD_MUTATE") == "1":
            candidate.write_bytes(b"local candidate changed after upload\n")
    elif args[:2] == ["release", "edit"]:
        pass
    else:
        refuse()
elif tool == "git":
    command = args[2:] if args[:1] == ["-C"] else args
    if command[:1] == ["ls-remote"]:
        sys.exit(1)
    if command[:1] in (["rev-parse"], ["hash-object"], ["write-tree"], ["commit-tree"]):
        print("0123456789abcdef0123456789abcdef01234567")
    elif command[:1] in (["fetch"], ["read-tree"], ["update-index"], ["tag"], ["push"], ["status"]):
        pass
    else:
        refuse()
elif tool == "xcrun":
    if args[:2] != ["stapler", "validate"]:
        refuse()
    print("fixture notarization validation")
    sys.exit(int(os.environ.get("SMOKE_TEST_NOTARIZATION_EXIT", "0")))
elif tool == "package-mac.sh":
    target = Path("dist") / os.environ["SMOKE_TEST_DMG_NAME"]
    target.parent.mkdir(exist_ok=True)
    target.write_bytes(b"local prepared candidate\n")
elif tool == "build-extension.sh":
    Path("dist/eastsea-extension-1.zip").write_bytes(b"fixture extension\n")
elif tool == "sign_update":
    print('sparkle:edSignature="fixture-signature" length="25"')
elif tool == "release-identity-gate.sh":
    pass
elif tool == "release-approve.py":
    if args[:1] != ["finalize"]:
        refuse()
    Path(option("--out")).write_text("{}\n")
elif tool == "release-vm-smoke.sh" and args == ["--check-setup"]:
    sys.exit(int(os.environ.get("SMOKE_TEST_VM_SETUP_EXIT", "78")))
elif tool in ("release-vm-smoke.sh", "release-canary.sh"):
    gate = "VM" if tool == "release-vm-smoke.sh" else "CANARY"
    code = int(os.environ.get("SMOKE_TEST_" + gate + "_EXIT", "0"))
    print(("FAIL" if code else "PASS") + ": fixture " + gate + " smoke")
    if os.environ.get("SMOKE_TEST_" + gate + "_MUTATE") == "1":
        candidate.write_bytes(b"candidate changed during the gate\n")
    sys.exit(code)
elif tool == "getconf" and args == ["DARWIN_USER_TEMP_DIR"]:
    print(os.environ["TMPDIR"])
else:
    refuse()
'''


class ReleaseSmokeTests(unittest.TestCase):
    def setUp(self):
        (ROOT / "tmp").mkdir(exist_ok=True)
        temporary = tempfile.TemporaryDirectory(prefix="release-smoke-test.", dir=ROOT / "tmp")
        self.addCleanup(temporary.cleanup)
        self.fixture = Path(temporary.name)
        for directory in (
            "scripts", "bin", "tmp", "dist", "previous.app", "apps/extension",
            "apps/wallet", "apps/wallet/build/SourcePackages/artifacts/sparkle/Sparkle/bin",
            "home/.cargo/bin", "home/.config/app-store-release",
        ):
            (self.fixture / directory).mkdir(parents=True, exist_ok=True)
        shutil.copy2(ROOT / "scripts/release-mac.sh", self.fixture / "scripts/release-mac.sh")
        (self.fixture / "apps/wallet/project.yml").write_text(
            "MARKETING_VERSION: " + VERSION + "\nCURRENT_PROJECT_VERSION: 99\n")
        (self.fixture / "apps/extension/manifest.json").write_text('{"version":"1"}\n')
        (self.fixture / "home/.config/app-store-release/env.sh").write_text(": # local fixture, no credentials\n")
        for name, data in (
            (DMG_NAME, LOCAL_DMG), ("eastsea-extension-1.zip", b"fixture extension\n"),
            ("eastsea-appcast.xml", b"<rss />\n"),
            ("EastSea-" + VERSION + "-manifest.json", b"{}\n"),
            ("EastSea-" + VERSION + "-builder-sigs.json", b"[]\n"),
            ("EastSea-" + VERSION + "-manifest.inventory.json", b"{}\n"),
        ):
            (self.fixture / "dist" / name).write_bytes(data)
        self.driver = self.fixture / "tmp/tool-double.py"
        self.driver.write_text(TOOL_DOUBLE)
        self.double_commands = set()
        self.real_gates = False
        for name in ("package-mac.sh", "build-extension.sh", "release-identity-gate.sh",
                     "release-approve.py", "release-vm-smoke.sh", "release-canary.sh"):
            self.write_double(self.fixture / "scripts" / name)
        self.write_double(self.fixture / "apps/wallet/build/SourcePackages/artifacts/sparkle/Sparkle/bin/sign_update")
        for name in ("gh", "git", "xcrun", "getconf", "ssh", "scp", "rsync", "tart", "open",
                     "hdiutil", "launchctl", "osascript", "codesign", "security", "curl", "nc", "cargo",
                     "xcodebuild", "swift", "swiftc", "sudo"):
            self.write_double(self.fixture / "bin" / name)
        self.event_file = self.fixture / "events.jsonl"
        self.environment = os.environ.copy()
        for key in list(self.environment):
            if key.startswith(("AETHER_", "SMOKE_TEST_")) or key in (
                    "PREV_RELEASE_TAG", "RELEASE_NOTES", "TERMS_BUMP_REASON"):
                del self.environment[key]
        self.environment.update({
            "HOME": str(self.fixture / "home"),
            "PATH": str(self.fixture / "bin") + ":/usr/bin:/bin:/usr/sbin:/sbin",
            "TMPDIR": str(self.fixture / "tmp"),
            "AETHER_PREVIOUS_APP": str(self.fixture / "previous.app"),
            "AETHER_RELEASE_INDEX": "7",
            "SMOKE_TEST_EVENTS": str(self.event_file),
            "SMOKE_TEST_DMG_NAME": DMG_NAME,
            "SMOKE_TEST_STAGED_ASSET": str(self.fixture / "tmp/uploaded-asset.dmg"),
            "PYTHONDONTWRITEBYTECODE": "1",
            "PYTHONNOUSERSITE": "1",
        })

    def write_double(self, path):
        # Explicit interpreters avoid macOS assessment stalls on fresh scripts.
        command = shlex.quote(sys.executable) + " -S " + shlex.quote(str(self.driver))
        path.write_text('#!/bin/bash\nexec ' + command + ' "$0" "$@"\n')
        path.chmod(0o755)
        relative = path.relative_to(self.fixture)
        self.double_commands.add(path.name if relative.parts[0] == "bin" else str(relative))

    def run_release(self, *arguments, **environment):
        self.event_file.unlink(missing_ok=True)
        shell_doubles = self.fixture / "tmp/shell-doubles.sh"
        definitions = []
        for command in sorted(self.double_commands):
            if self.real_gates and command in ("scripts/release-vm-smoke.sh", "scripts/release-canary.sh"):
                definitions.append("function " + command + '() { /bin/bash ' + shlex.quote(command) + ' "$@"; }')
                continue
            definitions.append("function " + command + "() { " + shlex.quote(sys.executable)
                               + " -S " + shlex.quote(str(self.driver)) + " " + shlex.quote(command) + ' "$@"; }')
        shell_doubles.write_text("\n".join(definitions) + "\n")
        environment = dict(self.environment, BASH_ENV=str(shell_doubles), **environment)
        process = subprocess.Popen(
            ["/bin/bash", "scripts/release-mac.sh", *arguments], cwd=self.fixture,
            env=environment, text=True,
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, start_new_session=True)
        try:
            stdout, stderr = process.communicate(timeout=60)
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGKILL)
            stdout, stderr = process.communicate()
            self.fail("local release fixture exceeded 60 seconds:\n" + stdout + stderr)
        self.output = stdout + stderr
        self.events = [json.loads(line) for line in self.event_file.read_text().splitlines()] if self.event_file.exists() else []
        return subprocess.CompletedProcess(process.args, process.returncode, stdout, stderr)

    def calls(self, tool, *prefix):
        matches = [event for event in self.events if event["tool"] == tool and event["args"][:len(prefix)] == list(prefix)]
        if tool == "release-vm-smoke.sh" and not prefix:
            # A setup query is read only; it must not be confused with a VM run.
            matches = [event for event in matches if event["args"] != ["--check-setup"]]
        return matches

    def position(self, tool, *prefix):
        return self.events.index(self.calls(tool, *prefix)[0])

    def assert_not_latest(self):
        self.assertFalse(self.calls("gh", "release", "edit"), self.output)

    def assert_ordered_gates(self):
        stapler = self.position("xcrun", "stapler", "validate")
        vm = self.position("release-vm-smoke.sh")
        canary = self.position("release-canary.sh")
        self.assertLess(stapler, vm, self.events)
        self.assertLess(vm, canary, self.events)
        return canary

    def assert_gates_read_uploaded_bytes(self, expected):
        digest = hashlib.sha256(expected).hexdigest()
        for tool in ("xcrun", "release-vm-smoke.sh", "release-canary.sh"):
            event = self.calls(tool)[0]
            self.assertEqual(event["sha256"], digest, self.events)
            argument = event["args"][2] if tool == "xcrun" else event["args"][0]
            candidate = Path(argument)
            self.assertEqual(candidate.name, DMG_NAME)
            self.assertTrue(candidate.is_relative_to(self.fixture / "tmp"), candidate)

    def test_publication_stays_draft_until_both_gates_pass(self):
        result = self.run_release("--canary-host", "stub-mac")
        self.assertEqual(result.returncode, 0, self.output)
        creation = self.calls("gh", "release", "create")
        self.assertEqual(len(creation), 1, self.events)
        self.assertIn("--draft", creation[0]["args"])
        self.assertIn("--latest=false", creation[0]["args"])
        self.assertNotIn("--latest", creation[0]["args"])
        self.assertLess(self.position("gh", "release", "create"), self.position("gh", "release", "view"))
        self.assertLess(self.position("gh", "release", "view"), self.position("gh", "release", "download", TAG))
        self.assertLess(self.position("gh", "release", "download", TAG), self.position("xcrun", "stapler", "validate"))
        self.assertLess(self.position("gh", "release", "create"), self.position("xcrun", "stapler", "validate"))
        self.assertLess(self.assert_ordered_gates(), self.position("gh", "release", "edit"))
        self.assert_gates_read_uploaded_bytes(LOCAL_DMG)
        self.assertEqual(self.calls("release-canary.sh")[0]["args"][1], "stub-mac")
        self.assertEqual(self.calls("gh", "release", "edit")[0]["args"],
                         ["release", "edit", TAG, "--repo", "eastsea-xyz/eastsea", "--draft=false", "--latest"])
        log = (self.fixture / "dist/release-gates.log").read_text()
        self.assertIn(hashlib.sha256(LOCAL_DMG).hexdigest(), log)
        self.assertIn("PASS: fixture VM smoke", log)
        self.assertIn("PASS: fixture CANARY smoke", log)

    def test_local_changes_after_upload_do_not_replace_the_shipped_smoke_candidate(self):
        result = self.run_release(SMOKE_TEST_LOCAL_AFTER_UPLOAD_MUTATE="1")
        self.assertEqual(result.returncode, 0, self.output)
        self.assertNotEqual((self.fixture / "dist" / DMG_NAME).read_bytes(), LOCAL_DMG)
        self.assert_gates_read_uploaded_bytes(LOCAL_DMG)
        self.assertLess(self.assert_ordered_gates(), self.position("gh", "release", "edit"))

    def test_changed_upload_cannot_launch_a_candidate_with_a_different_fingerprint(self):
        for change in ("SMOKE_TEST_PRE_UPLOAD_MUTATE", "SMOKE_TEST_STAGED_MUTATE"):
            with self.subTest(change=change):
                result = self.run_release(**{change: "1"})
                self.assertNotEqual(result.returncode, 0, self.output)
                self.assertTrue(self.calls("gh", "release", "create"))
                self.assertTrue(self.calls("gh", "release", "download", TAG))
                self.assertFalse(self.calls("xcrun"))
                self.assertFalse(self.calls("release-vm-smoke.sh"))
                self.assertFalse(self.calls("release-canary.sh"))
                self.assert_not_latest()

    def test_notarization_failure_blocks_every_launch_and_promotion(self):
        result = self.run_release(SMOKE_TEST_NOTARIZATION_EXIT="9")
        self.assertNotEqual(result.returncode, 0, self.output)
        self.assertTrue(self.calls("gh", "release", "create"))
        self.assertFalse(self.calls("release-vm-smoke.sh"))
        self.assertFalse(self.calls("release-canary.sh"))
        self.assert_not_latest()

    def test_vm_failure_keeps_draft_and_prevents_canary(self):
        result = self.run_release(SMOKE_TEST_VM_EXIT="8")
        self.assertNotEqual(result.returncode, 0, self.output)
        self.assertFalse(self.calls("release-canary.sh"))
        self.assert_not_latest()

    def test_canary_failure_keeps_draft(self):
        result = self.run_release(SMOKE_TEST_CANARY_EXIT="8")
        self.assertNotEqual(result.returncode, 0, self.output)
        self.assert_ordered_gates()
        self.assertEqual(self.calls("release-canary.sh")[0]["args"][1], "poc-m3")
        self.assert_not_latest()

    def test_unconfigured_vm_cannot_be_silently_skipped(self):
        result = self.run_release(SMOKE_TEST_VM_EXIT="2")
        self.assertNotEqual(result.returncode, 0, self.output)
        self.assertTrue(self.calls("release-vm-smoke.sh"))
        self.assertNotIn("VM SMOKE SKIPPED", self.output)
        self.assert_not_latest()

    def test_explicit_vm_exception_is_loud_and_canary_is_still_mandatory(self):
        result = self.run_release("--skip-vm-smoke", SMOKE_TEST_CANARY_EXIT="8")
        self.assertNotEqual(result.returncode, 0, self.output)
        self.assertTrue(self.calls("release-vm-smoke.sh", "--check-setup"))
        self.assertFalse(self.calls("release-vm-smoke.sh"))
        self.assertTrue(self.calls("release-canary.sh"))
        self.assertIn("ALARM: VM SMOKE SKIPPED (--skip-vm-smoke)", self.output)
        self.assertIn("Canary remains mandatory", self.output)
        log = (self.fixture / "dist/release-gates.log").read_text()
        self.assertIn("ALARM: VM SMOKE SKIPPED (--skip-vm-smoke)", log)
        self.assert_not_latest()

    def test_explicit_unconfigured_vm_exception_can_publish_after_canary(self):
        result = self.run_release("--skip-vm-smoke")
        self.assertEqual(result.returncode, 0, self.output)
        self.assertFalse(self.calls("release-vm-smoke.sh"))
        self.assertLess(self.position("release-canary.sh"), self.position("gh", "release", "edit"))

    def test_configured_vm_cannot_use_the_setup_exception(self):
        result = self.run_release("--skip-vm-smoke", SMOKE_TEST_VM_SETUP_EXIT="0")
        self.assertNotEqual(result.returncode, 0, self.output)
        self.assertTrue(self.calls("release-vm-smoke.sh", "--check-setup"))
        self.assertFalse(self.calls("release-vm-smoke.sh"))
        self.assertFalse(self.calls("release-canary.sh"))
        self.assertNotIn("VM SMOKE SKIPPED", self.output)
        self.assert_not_latest()

    def test_unexpected_vm_setup_failure_cannot_use_the_setup_exception(self):
        result = self.run_release("--skip-vm-smoke", SMOKE_TEST_VM_SETUP_EXIT="2")
        self.assertNotEqual(result.returncode, 0, self.output)
        self.assertTrue(self.calls("release-vm-smoke.sh", "--check-setup"))
        self.assertFalse(self.calls("release-canary.sh"))
        self.assertNotIn("VM SMOKE SKIPPED", self.output)
        self.assert_not_latest()

    def test_draft_option_runs_gates_and_leaves_release_draft(self):
        result = self.run_release("--draft")
        self.assertEqual(result.returncode, 0, self.output)
        self.assert_ordered_gates()
        self.assert_not_latest()
        self.assertIn("remains draft (--draft)", self.output)

    def test_prepared_release_finalizes_and_runs_the_same_publication_gates(self):
        result = self.run_release("--publish-prepared")
        self.assertEqual(result.returncode, 0, self.output)
        self.assertFalse(self.calls("package-mac.sh"))
        self.assertFalse(self.calls("sign_update"))
        self.assertFalse(self.calls("build-extension.sh"))
        self.assertLess(self.position("release-approve.py", "finalize"), self.position("gh", "release", "create"))
        assets = self.calls("gh", "release", "create")[0]["args"]
        for name in ("manifest", "builder-sigs", "release-index"):
            self.assertIn("dist/EastSea-" + VERSION + "-" + name + ".json", assets)
        self.assert_gates_read_uploaded_bytes(LOCAL_DMG)
        self.assertLess(self.assert_ordered_gates(), self.position("gh", "release", "edit"))

    def test_failed_draft_retry_rechecks_exact_uploaded_dmg_before_publishing(self):
        failed = self.run_release("--publish-draft", SMOKE_TEST_CANARY_EXIT="8")
        self.assertNotEqual(failed.returncode, 0, self.output)
        self.assert_not_latest()
        result = self.run_release("--publish-draft", "--canary-host", "stub-retry-mac")
        self.assertEqual(result.returncode, 0, self.output)
        self.assertFalse(self.calls("package-mac.sh"))
        self.assertFalse(self.calls("sign_update"))
        self.assertFalse(self.calls("gh", "release", "create"))
        download = self.calls("gh", "release", "download")[0]
        self.assertEqual(download["args"][2], TAG)
        self.assertIn(DMG_NAME, download["args"])
        self.assertLess(self.position("gh", "release", "view"), self.position("gh", "release", "download"))
        self.assertLess(self.position("gh", "release", "download"), self.position("xcrun", "stapler", "validate"))
        self.assertNotEqual(hashlib.sha256(STAGED_DMG).hexdigest(), hashlib.sha256(LOCAL_DMG).hexdigest())
        self.assert_gates_read_uploaded_bytes(STAGED_DMG)
        self.assertEqual(self.calls("release-canary.sh")[0]["args"][1], "stub-retry-mac")
        self.assertLess(self.assert_ordered_gates(), self.position("gh", "release", "edit"))

    def test_non_draft_release_cannot_be_promoted_by_retry(self):
        result = self.run_release("--publish-draft", SMOKE_TEST_DRAFT_STATE="false")
        self.assertNotEqual(result.returncode, 0, self.output)
        self.assertIn("existing draft", self.output)
        self.assertFalse(self.calls("gh", "release", "download"))
        self.assertFalse(self.calls("xcrun"))
        self.assert_not_latest()

    def test_missing_staged_asset_blocks_retry(self):
        result = self.run_release("--publish-draft", SMOKE_TEST_MISSING_DOWNLOAD="1")
        self.assertNotEqual(result.returncode, 0, self.output)
        self.assertFalse(self.calls("xcrun"))
        self.assert_not_latest()

    def test_candidate_mutation_during_either_gate_blocks_promotion(self):
        for gate in ("VM", "CANARY"):
            with self.subTest(gate=gate):
                result = self.run_release("--publish-draft", **{"SMOKE_TEST_" + gate + "_MUTATE": "1"})
                self.assertNotEqual(result.returncode, 0, self.output)
                if gate == "VM":
                    self.assertFalse(self.calls("release-canary.sh"), self.events)
                self.assert_not_latest()

    def test_invalid_flags_fail_before_any_external_operation(self):
        for arguments in (("--skip-canary",), ("--canary-host",), ("--canary-host", "--dry-run"),
                          ("--canary-host", "stub mac"), ("--canary-host", "stub-mac;echo"),
                          ("--prepare", "--publish-prepared"), ("--prepare", "--draft")):
            with self.subTest(arguments=arguments):
                result = self.run_release(*arguments)
                self.assertNotEqual(result.returncode, 0, self.output)
                self.assertEqual(self.events, [], self.output)

    def test_real_gate_dry_runs_accept_future_dmg_and_stub_hosts_without_effects(self):
        self.real_gates = True
        for name in ("release-vm-smoke.sh", "release-canary.sh"):
            source = ROOT / "scripts" / name
            self.assertTrue(source.is_file(), str(source) + " is missing")
            shutil.copy2(source, self.fixture / "scripts" / name)
        # Even an installed Tart cannot be invoked in a dry run. Every other
        # remote/app operation is also a rejecting double on this isolated PATH.
        self.write_double(self.fixture / "bin/tart")
        for arguments in (("--dry-run", "--canary-host", "stub-mac"),
                          ("--dry-run", "--draft", "--canary-host", "stub-draft-mac"),
                          ("--dry-run", "--publish-prepared", "--canary-host", "stub-prepared-mac"),
                          ("--dry-run", "--publish-draft", "--canary-host", "stub-retry-mac")):
            with self.subTest(arguments=arguments):
                (self.fixture / "dist" / DMG_NAME).unlink(missing_ok=True)
                result = self.run_release(*arguments)
                self.assertEqual(result.returncode, 0, self.output)
                self.assertEqual(self.events, [], self.output)
                self.assertIn("DRY-RUN", self.output)
                self.assertNotIn("PASS:", self.output)
                self.assertIn(arguments[-1], self.output)
                vm = self.output.index("release-vm-smoke.sh")
                canary = self.output.index("release-canary.sh")
                self.assertLess(vm, canary)
                if "--draft" in arguments:
                    self.assertNotIn("--draft=false", self.output)
                else:
                    self.assertLess(canary, self.output.index("--draft=false"))


if __name__ == "__main__":
    unittest.main(verbosity=2)
