import importlib.util
import json
import pathlib
import os
import shutil
import subprocess
import tempfile
import unittest
from argparse import Namespace


ROOT = pathlib.Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("release_approve", ROOT / "scripts/release-approve.py")
release = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(release)


class ReleaseApprovalScriptTests(unittest.TestCase):
    def test_executable_hash_ignores_only_the_signing_envelope(self):
        (ROOT / "tmp").mkdir(exist_ok=True)
        with tempfile.TemporaryDirectory(dir=ROOT / "tmp") as directory:
            directory = pathlib.Path(directory)
            source = directory / "main.swift"
            source.write_text('print("release code")\n')
            executable = directory / "unsigned"
            environment = os.environ.copy()
            environment["TMPDIR"] = str(ROOT / "tmp")
            environment["CLANG_MODULE_CACHE_PATH"] = str(ROOT / "tmp/clang")
            environment["SWIFT_MODULECACHE_PATH"] = str(ROOT / "tmp/swift")
            subprocess.run(["swiftc", "-target", "arm64-apple-macos14.0", str(source), "-o", str(executable)],
                check=True, env=environment, capture_output=True)
            resigned = directory / "resigned"
            shutil.copy2(executable, resigned)
            subprocess.run(["codesign", "-s", "-", "-f", str(resigned)], check=True, capture_output=True)
            self.assertEqual(release.executable_sha256(executable), release.executable_sha256(resigned))
            data = bytearray(resigned.read_bytes())
            data[8192] ^= 1
            resigned.write_bytes(data)
            self.assertNotEqual(release.executable_sha256(executable), release.executable_sha256(resigned))

    def pin_network(self, fields):
        (ROOT / "tmp").mkdir(exist_ok=True)
        directory = tempfile.TemporaryDirectory(dir=ROOT / "tmp")
        self.addCleanup(directory.cleanup)
        network = pathlib.Path(directory.name) / "network.json"
        network.write_bytes(release.canonical(fields))
        previous = release.NETWORK
        release.NETWORK = network
        self.addCleanup(setattr, release, "NETWORK", previous)

    LOG = "0x0000000000000000000000000000000000007705"
    KEYS = ["04" + str(i) * 128 for i in range(1, 4)]

    def release_pin(self, **change):
        pin = {"log": self.LOG, "code_hash": "0x" + "a" * 64, "builder_keys": self.KEYS,
               "threshold": 2, "emergency_threshold": 3}
        pin.update(change)
        return pin

    def test_only_pinned_builder_keys_are_accepted(self):
        self.pin_network({"chain_id": 9001, "release": self.release_pin()})
        keys = release.pinned_builders({"chain_id": 9001, "log_address": self.LOG})
        self.assertEqual(keys, set(self.KEYS))
        with self.assertRaises(ValueError):
            release.pinned_builders({"chain_id": 9002, "log_address": self.LOG})
        with self.assertRaises(ValueError):
            release.pinned_builders({"chain_id": 9001, "log_address": "0x" + "7" * 40})

    def test_the_release_object_is_the_only_pin(self):
        manifest = {"chain_id": 9001, "log_address": self.LOG}
        # The pre-B6 flat fields are not a pin: no fallback.
        self.pin_network({"chain_id": 9001, "release_log": self.LOG,
                          "release_log_code_hash": "0x" + "a" * 64, "builder_keys": self.KEYS})
        with self.assertRaisesRegex(ValueError, "no \"release\" pin"):
            release.pinned_builders(manifest)
        # The shipped 7780 file has no pin at all.
        release.NETWORK = ROOT / "apps/wallet/Resources/network.json"
        with self.assertRaisesRegex(ValueError, "no \"release\" pin"):
            release.pinned_builders({"chain_id": 7780, "log_address": self.LOG})
        for bad in (self.release_pin(threshold=1), self.release_pin(emergency_threshold=2),
                    self.release_pin(builder_keys=self.KEYS[:2]),
                    self.release_pin(builder_keys=[self.KEYS[0], self.KEYS[1], self.KEYS[0].upper()]),
                    self.release_pin(builder_keys=["05" + "1" * 128] + self.KEYS[1:]),
                    self.release_pin(code_hash="0x1234")):
            self.pin_network({"chain_id": 9001, "release": bad})
            with self.assertRaises(ValueError):
                release.pinned_builders(manifest)

    def test_contract_hash_refuses_a_runtime_other_than_the_predeploy(self):
        embedded = "0x" + release.RUNTIME_HEX.read_text().strip()
        calls = []

        def fake_check_output(argv, **_kwargs):
            calls.append(argv[0])
            return outputs[argv[0]]

        previous = release.subprocess.check_output
        release.subprocess.check_output = fake_check_output
        self.addCleanup(setattr, release.subprocess, "check_output", previous)
        outputs = {"forge": embedded + "\n", "cast": "0x" + "ab" * 32}
        release.contract_hash(None)
        outputs = {"forge": embedded[:-2] + "00\n", "cast": "0x" + "ab" * 32}
        with self.assertRaisesRegex(ValueError, "differs from crates/execution/src/release_log.bin.hex"):
            release.contract_hash(None)
        self.assertEqual(calls, ["forge", "cast", "forge", "cast"])

    def test_prepare_is_canonical_and_detects_builder_mismatch(self):
        (ROOT / "tmp").mkdir(exist_ok=True)
        with tempfile.TemporaryDirectory(dir=ROOT / "tmp") as directory:
            directory = pathlib.Path(directory)
            app = directory / "EastSea.app"
            executable = app / "Contents/MacOS/EastSea"
            executable.parent.mkdir(parents=True)
            executable.write_bytes(b"wallet")
            helpers = app / "Contents/Helpers"
            helpers.mkdir()
            (helpers / "aether").write_bytes(b"node")
            (helpers / "aether-agent").write_bytes(b"agent")
            dmg = directory / "EastSea.dmg"
            dmg.write_bytes(b"archive")
            manifest = directory / "manifest.json"
            args = Namespace(app=str(app), dmg=str(dmg), chain_id=9001,
                log="0x" + "7" * 40, version="1.2.3", build="42",
                sparkle_signature="signature", emergency=False, out=str(manifest))
            release.prepare(args)
            data = manifest.read_bytes()
            self.assertEqual(data, release.canonical(json.loads(data)))
            inventory = manifest.with_suffix(".inventory.json")
            self.assertEqual(release.sha256(inventory), json.loads(data)["bundle_inventory_sha256"])
            release.compare(Namespace(manifest=str(manifest), other=[str(manifest)]))
            resource = app / "Contents/Resources/unexpected.sh"
            resource.parent.mkdir()
            resource.write_text("#!/bin/sh\necho unexpected\n")
            rebuilt = directory / "rebuilt.json"
            args.out = str(rebuilt)
            release.prepare(args)
            with self.assertRaises(SystemExit):
                release.compare(Namespace(manifest=str(manifest), other=[str(rebuilt)]))
            other = directory / "other.json"
            changed = json.loads(data)
            changed["artifacts"][0]["sha256"] = "0" * 64
            other.write_bytes(release.canonical(changed))
            with self.assertRaises(SystemExit):
                release.compare(Namespace(manifest=str(manifest), other=[str(other)]))


if __name__ == "__main__":
    unittest.main()
