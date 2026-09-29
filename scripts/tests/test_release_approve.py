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

    def test_only_pinned_builder_keys_are_accepted(self):
        (ROOT / "tmp").mkdir(exist_ok=True)
        with tempfile.TemporaryDirectory(dir=ROOT / "tmp") as directory:
            network = pathlib.Path(directory) / "network.json"
            network.write_bytes(release.canonical({
                "chain_id": 9001, "release_log": "0x" + "7" * 40,
                "release_log_code_hash": "0x" + "a" * 64,
                "builder_keys": ["04" + str(i) * 128 for i in range(1, 4)],
            }))
            previous = release.NETWORK
            release.NETWORK = network
            try:
                keys = release.pinned_builders({"chain_id": 9001, "log_address": "0x" + "7" * 40})
                self.assertEqual(len(keys), 3)
                with self.assertRaises(ValueError):
                    release.pinned_builders({"chain_id": 9002, "log_address": "0x" + "7" * 40})
            finally:
                release.NETWORK = previous

    def test_prepare_is_canonical_and_detects_builder_mismatch(self):
        (ROOT / "tmp").mkdir(exist_ok=True)
        with tempfile.TemporaryDirectory(dir=ROOT / "tmp") as directory:
            directory = pathlib.Path(directory)
            app = directory / "Aether.app"
            executable = app / "Contents/MacOS/Aether"
            executable.parent.mkdir(parents=True)
            executable.write_bytes(b"wallet")
            helpers = app / "Contents/Helpers"
            helpers.mkdir()
            (helpers / "aether").write_bytes(b"node")
            (helpers / "aether-agent").write_bytes(b"agent")
            dmg = directory / "Aether.dmg"
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
