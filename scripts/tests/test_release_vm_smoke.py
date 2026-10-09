#!/usr/bin/env python3
"""Test the real VM gate's helpers with local data and command doubles only.

The generated guest's module body and its app/node launch commands are never
executed. Function definitions are extracted with AST; monitoring uses a fake
clock, process table, and RPC. Every fixture lives under the repository's tmp/.
"""
import ast
import contextlib
import copy
import io
import json
import pathlib
import shlex
import subprocess
import sys
import tempfile
import types
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[2]
SCRIPT = (ROOT / "scripts/release-vm-smoke.sh").read_text()


def heredoc(marker):
    start = SCRIPT.index("\n", SCRIPT.index("<<'" + marker + "'")) + 1
    return SCRIPT[start:SCRIPT.index("\n" + marker, start)]


def guest_functions():
    constants = {
        "FIRST_SECONDS", "RELAUNCH_SECONDS", "SAMPLE_SECONDS", "HEAD_TOLERANCE",
        "CHAIN_ID", "SEEDED_BYTES", "LOCAL_RPC", "REFERENCE_RPC", "BUNDLE_ID", "TEAM_ID", "APP",
    }
    parsed = ast.parse(heredoc("GUEST_PYTHON"))
    nodes = []
    for node in parsed.body:
        if isinstance(node, (ast.Import, ast.ImportFrom, ast.FunctionDef)):
            nodes.append(node)
        elif isinstance(node, ast.Assign) and len(node.targets) == 1 \
                and isinstance(node.targets[0], ast.Name) and node.targets[0].id in constants:
            nodes.append(node)
    namespace = {}
    exec(compile(ast.Module(body=nodes, type_ignores=[]), "vm-guest-functions", "exec"), namespace)
    return namespace


class FixtureTest(unittest.TestCase):
    def setUp(self):
        (ROOT / "tmp").mkdir(exist_ok=True)
        self.temporary = tempfile.TemporaryDirectory(prefix="release-vm-tests.", dir=str(ROOT / "tmp"))
        self.addCleanup(self.temporary.cleanup)
        self.work = pathlib.Path(self.temporary.name)
        self.namespace = guest_functions()


class GuestProcessTests(FixtureTest):
    def test_physical_mac_refused_before_mutation(self):
        commands = []
        self.namespace["command"] = lambda args, **kw: commands.append(args) or types.SimpleNamespace(stdout="Mac14,3\n")
        with self.assertRaisesRegex(RuntimeError, "physical Mac"):
            self.namespace["main"]()
        self.assertEqual(commands, [["/usr/sbin/sysctl", "-n", "hw.model"]])

    def test_fixed_observation_and_chain_constants(self):
        for name, value in [("FIRST_SECONDS", 600), ("RELAUNCH_SECONDS", 60), ("CHAIN_ID", 7780),
                            ("SEEDED_BYTES", 64 * 1024 * 1024), ("HEAD_TOLERANCE", 12)]:
            self.assertEqual(self.namespace[name], value)

    def listener(self, table, stdout="p103\n"):
        self.namespace["subprocess"] = types.SimpleNamespace(
            run=lambda *args, **kwargs: types.SimpleNamespace(stdout=stdout, returncode=0))
        return self.namespace["listener_pid"](101, table)

    def table(self):
        return {101: (1, "/Applications/EastSea.app/Contents/MacOS/EastSea"),
                102: (101, "/Applications/EastSea.app/Contents/Helpers/aether"),
                103: (102, "/Applications/EastSea.app/Contents/Helpers/aether")}

    def test_node_listener_must_be_bundled_app_descendant(self):
        self.assertEqual(self.listener(self.table()), 103)

    def test_wrong_node_binary_refused(self):
        table = self.table()
        table[103] = (102, "/another/node")
        with self.assertRaisesRegex(RuntimeError, "bundled node"):
            self.listener(table)

    def test_unrelated_listener_refused(self):
        table = self.table()
        table[103] = (1, table[103][1])
        with self.assertRaisesRegex(RuntimeError, "bundled node"):
            self.listener(table)

    def test_ambiguous_rpc_listeners_refused(self):
        with self.assertRaisesRegex(RuntimeError, "ambiguous"):
            self.listener(self.table(), "p102\np103\n")

    def test_ancestry_cycles_do_not_match(self):
        self.assertFalse(self.namespace["belongs_to"](103, 101, {103: (102, "x"), 102: (103, "x")}))


class FakeClock:
    def __init__(self):
        self.now = 0.0

    def monotonic(self):
        return self.now

    def sleep(self, seconds):
        self.now += seconds


class GuestObservationTests(FixtureTest):
    def observe(self, stage="first-launch", seconds=600, pid=101, variant="pass"):
        clock = FakeClock()
        output = self.work / (stage + "-" + variant)
        output.mkdir(exist_ok=True)
        report = {"phases": []}
        self.namespace.update(time=clock, OUTPUT=output, report=report)
        self.namespace["process_table"] = lambda: {}
        self.namespace["app_pid"] = lambda table: pid + 10 if variant == "pid-restart" and clock.now >= 120 else pid
        self.namespace["listener_pid"] = lambda *args: None if variant == "node-absent" else 103

        def crashes():
            if variant == "crash-report" and clock.now >= 120:
                raise RuntimeError("new app/node crash report")

        self.namespace["check_crashes"] = crashes

        def rpc(url, method):
            local = url == self.namespace["LOCAL_RPC"]
            if method == "eth_chainId":
                wrong = (variant == "wrong-local-chain" and local) or (variant == "wrong-reference-chain" and not local)
                return 7777 if wrong else 7780
            now = clock.now
            if variant == "late-stall" and local:
                now = min(now, seconds - 30)
            if variant == "height-regression" and local and now >= 120:
                return 1000
            if variant == "reference-unavailable" and not local:
                raise RuntimeError("reference unavailable")
            return 1000 + int(now // 5)

        self.namespace["rpc"] = rpc
        with contextlib.redirect_stdout(io.StringIO()):
            self.namespace["observe"](stage, pid, seconds, 120 if seconds == 600 else 30)
        return report["phases"][0]

    def test_complete_first_launch_and_relaunch_windows(self):
        first = self.observe()
        second = self.observe("relaunch", 60, pid=201)
        self.assertGreaterEqual(first["observed_seconds"], 600)
        self.assertGreaterEqual(second["observed_seconds"], 60)
        self.assertNotEqual(first["app_pid"], second["app_pid"])

    def test_pid_restart_refused(self):
        with self.assertRaisesRegex(RuntimeError, "PID changed"):
            self.observe(variant="pid-restart")

    def test_missing_app_node_refused(self):
        with self.assertRaisesRegex(RuntimeError, "not listening"):
            self.observe(variant="node-absent")

    def test_wrong_local_chain_refused(self):
        with self.assertRaisesRegex(RuntimeError, "local RPC chain_id"):
            self.observe(variant="wrong-local-chain")

    def test_wrong_reference_chain_refused(self):
        with self.assertRaisesRegex(RuntimeError, "reference RPC chain_id"):
            self.observe(variant="wrong-reference-chain")

    def test_stall_after_early_catchup_refused(self):
        with self.assertRaisesRegex(RuntimeError, "stalled at the end"):
            self.observe(variant="late-stall")

    def test_local_height_regression_refused(self):
        with self.assertRaisesRegex(RuntimeError, "height regressed"):
            self.observe(variant="height-regression")

    def test_reference_required_at_final_sample(self):
        with self.assertRaisesRegex(RuntimeError, "final sample"):
            self.observe(variant="reference-unavailable")

    def test_new_crash_during_observation_refused(self):
        with self.assertRaisesRegex(RuntimeError, "new app/node crash"):
            self.observe(variant="crash-report")


class GuestCrashTests(FixtureTest):
    def setUp(self):
        super().setUp()
        home = self.work / "home"
        self.user_reports = home / "Library/Logs/DiagnosticReports"
        self.system_reports = self.work / "system-reports"
        self.output = self.work / "output"
        for directory in [self.user_reports, self.system_reports, self.output]:
            directory.mkdir(parents=True)
        self.namespace.update(HOME=home, OUTPUT=self.output, report={"crash_reports": []}, crash_baseline={})
        self.namespace["pathlib"] = types.SimpleNamespace(
            Path=lambda path: self.system_reports if str(path) == "/Library/Logs/DiagnosticReports" else pathlib.Path(path))

    def test_new_app_and_bundled_node_reports_fail(self):
        for name in ["EastSea-new.ips", "EastSea_classic.crash", "aether-new.ips", "aether_classic.crash", "aether-prover_new.ips"]:
            with self.subTest(name=name):
                path = self.user_reports / name
                path.write_text("fixture crash\n")
                with self.assertRaisesRegex(RuntimeError, "new or changed app/node"):
                    self.namespace["check_crashes"]()
                path.unlink()

    def test_changed_existing_report_fails(self):
        path = self.user_reports / "EastSea-existing.ips"
        path.write_text("old\n")
        self.namespace["crash_baseline"] = self.namespace["crash_state"]()
        path.write_text("changed report\n")
        with self.assertRaisesRegex(RuntimeError, "new or changed"):
            self.namespace["check_crashes"]()

    def test_system_node_report_fails(self):
        (self.system_reports / "aether-system.ips").write_text("fixture crash\n")
        with self.assertRaisesRegex(RuntimeError, "app/node"):
            self.namespace["check_crashes"]()


class HostCompletionTests(FixtureTest):
    def setUp(self):
        super().setUp()
        self.result = self.work / "result.json"
        self.status = self.work / "exit-status.json"
        self.validator = heredoc("PY_RESULT")

    @staticmethod
    def phase(stage, seconds, pid):
        samples = [{"elapsed_seconds": elapsed, "app_pid": pid, "node_pid": pid + 1,
                    "chain_id": 7780, "reference_chain_id": 7780,
                    "local_height": 1000 + elapsed // 5, "reference_height": 1002 + elapsed // 5,
                    "lag_blocks": 2} for elapsed in range(0, seconds + 1, 5)]
        return {"stage": stage, "required_seconds": seconds, "observed_seconds": float(seconds),
                "app_pid": pid, "samples": samples}

    def valid_result(self, profile="empty"):
        return {"schema": 1, "run_id": "vm-smoke.unit", "profile": profile, "dmg_sha256": "a" * 64,
                "result": "PASS", "chain_id": 7780, "reference_chain_id": 7780,
                "bundle_id": "com.pipln.eastsea", "team_id": "45WU468FZE",
                "reference_rpc": "https://rpc.eastsea.xyz", "head_tolerance": 12,
                "seeded_log_bytes": 64 * 1024 * 1024 if profile == "large-log" else 0,
                "crash_reports": [], "phases": [self.phase("first-launch", 600, 101), self.phase("relaunch", 60, 201)]}

    def validate(self, result=None, completion=None, profile="empty"):
        if result is not None:
            self.result.write_text(json.dumps(result))
        if completion is not None:
            self.status.write_text(json.dumps(completion))
        return subprocess.run([sys.executable, "-", str(self.result), str(self.status),
                               "vm-smoke.unit", profile, "a" * 64], input=self.validator,
                              capture_output=True, text=True, cwd=self.work)

    def test_complete_evidence_for_both_profiles(self):
        for profile in ["empty", "large-log"]:
            with self.subTest(profile=profile):
                self.assertEqual(self.validate(self.valid_result(profile), {"schema": 1, "exit_status": 0}, profile).returncode, 0)

    def test_pass_json_without_completion_is_rejected(self):
        self.assertNotEqual(self.validate(self.valid_result()).returncode, 0)

    def test_pass_json_with_helper_failure_is_rejected(self):
        self.assertNotEqual(self.validate(self.valid_result(), {"schema": 1, "exit_status": 17}).returncode, 0)

    def test_boolean_completion_is_rejected(self):
        self.assertNotEqual(self.validate(self.valid_result(), {"schema": 1, "exit_status": False}).returncode, 0)

    def test_incomplete_or_wrong_evidence_rejected(self):
        mutations = {
            "short-window": lambda row: row["phases"][0].update(observed_seconds=599),
            "partial-samples": lambda row: row["phases"][0].update(samples=row["phases"][0]["samples"][:2]),
            "new-crash": lambda row: row.update(crash_reports=["aether-new.ips"]),
            "wrong-artifact": lambda row: row.update(dmg_sha256="b" * 64),
            "reused-pid": lambda row: row["phases"][1].update(app_pid=101),
            "wrong-seed": lambda row: row.update(seeded_log_bytes=64 * 1024 * 1024),
            "wrong-reference-chain": lambda row: row.update(reference_chain_id=7777),
            "failed-helper-result": lambda row: row.update(result="FAIL", reason="fixture failure"),
        }
        for name, mutate in mutations.items():
            with self.subTest(name=name):
                row = copy.deepcopy(self.valid_result())
                mutate(row)
                self.assertNotEqual(self.validate(row, {"schema": 1, "exit_status": 0}).returncode, 0)

    def test_runner_publishes_actual_helper_exit_code(self):
        runner = heredoc("GUEST_RUNNER")
        start = runner.index("finish_runner() {")
        end = runner.index("\n}\n", start) + 3
        exit_function = runner[start:end]
        helper_call = runner.splitlines()[-1]
        for helper_exit in [0, 17]:
            with self.subTest(helper_exit=helper_exit):
                directory = self.work / ("runner-%d" % helper_exit)
                output = directory / "output"
                output.mkdir(parents=True)
                (directory / "config.json").write_text("{}\n")
                (directory / "guest-smoke.py").write_text(
                    "import json,pathlib,sys\n"
                    "out=pathlib.Path(__file__).parent/'output'\n"
                    "(out/'result.json').write_text(json.dumps({'result':'PASS'}))\n"
                    "sys.exit(%d)\n" % helper_exit)
                shell = "set -euo pipefail\nOUTPUT=%s\nRUN_DIR=%s\n%s\ntrap finish_runner EXIT\n%s\n" % (
                    shlex.quote(str(output)), shlex.quote(str(directory)), exit_function, helper_call)
                done = subprocess.run(["/bin/bash", "-s"], input=shell, capture_output=True, text=True, cwd=directory)
                self.assertEqual(done.returncode, helper_exit, done.stderr)
                self.assertEqual(json.loads((output / "exit-status.json").read_text()), {"schema": 1, "exit_status": helper_exit})
                self.assertFalse((output / "exit-status.json.part").exists())


if __name__ == "__main__":
    unittest.main(verbosity=2)
