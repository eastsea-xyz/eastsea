#!/usr/bin/env python3
"""Regression checks for affected selection without compiling Rust."""
import importlib.util
import json
from pathlib import Path
import subprocess
import shutil
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("affected", ROOT / "scripts/affected-crates.py")
affected = importlib.util.module_from_spec(spec)
spec.loader.exec_module(affected)


class SelectionTests(unittest.TestCase):
    def setUp(self):
        (ROOT / "tmp").mkdir(exist_ok=True)
        self.temp = tempfile.TemporaryDirectory(dir=ROOT / "tmp")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.git("init", "-q")
        self.git("config", "user.email", "fixture@example.invalid")
        self.git("config", "user.name", "Fixture")
        packages = []
        for name, dependencies in (("types", []), ("node", ["types"]), ("cli", ["node"]), ("other", [])):
            directory = self.root / "crates" / name
            directory.mkdir(parents=True)
            (directory / "lib.rs").write_text("// baseline\n")
            packages.append({"id": name, "name": name, "manifest_path": str(directory / "Cargo.toml"),
                             "dependencies": [{"path": str(self.root / "crates" / dep)} for dep in dependencies]})
        self.metadata = {"workspace_members": [p["id"] for p in packages], "packages": packages}
        self.git("add", ".")
        self.git("commit", "-qm", "baseline")
        self.git("branch", "lead-merge")

    def git(self, *args):
        return subprocess.check_output(["git", "-C", str(self.root), *args])

    def selection(self):
        return affected.select(self.root, self.metadata, affected.changed_paths(self.root, "lead-merge"))

    def test_reverse_dependents_unstaged(self):
        (self.root / "crates/types/lib.rs").write_text("// changed\n")
        self.assertEqual(self.selection(), ["cli", "node", "types"])

    def test_staged_and_untracked(self):
        (self.root / "crates/node/lib.rs").write_text("// changed\n")
        self.git("add", ".")
        (self.root / "crates/other/new.rs").write_text("// new\n")
        self.assertEqual(self.selection(), ["cli", "node", "other"])

    def test_staged_change_reverted_only_in_worktree_is_still_selected(self):
        source = self.root / "crates/types/lib.rs"
        source.write_text("// staged\n")
        self.git("add", ".")
        source.write_text("// baseline\n")
        self.assertEqual(self.selection(), ["cli", "node", "types"])

    def test_committed_rename_and_delete(self):
        self.git("mv", "crates/types/lib.rs", "crates/other/moved.rs")
        self.git("rm", "crates/node/lib.rs")
        self.git("commit", "-qm", "rename and delete")
        self.assertEqual(self.selection(), ["cli", "node", "other", "types"])

    def test_workspace_configuration(self):
        for path in ("Cargo.toml", "Cargo.lock", "rust-toolchain.toml", ".cargo/config.toml", ".config/nextest.toml", "vendor/source.rs"):
            with self.subTest(path=path):
                self.assertEqual(affected.select(self.root, self.metadata, {path}), ["cli", "node", "other", "types"])

    def test_docs_and_guest_do_not_select_workspace(self):
        self.assertEqual(affected.select(self.root, self.metadata, {"docs/readme.md", "apps/prover/src/main.rs"}), [])

    def test_cli_reads_metadata_without_cargo(self):
        metadata_file = self.root / "metadata.json"
        metadata_file.write_text(json.dumps(self.metadata))
        (self.root / "crates/types/lib.rs").write_text("// changed\n")
        result = subprocess.check_output(["python3", str(ROOT / "scripts/affected-crates.py"),
                                          "--root", str(self.root), "--metadata-file", str(metadata_file)])
        self.assertEqual(result.decode().splitlines(), ["cli", "node", "types"])

    def test_wrapper_empty_args_and_dry_run(self):
        scripts = self.root / "scripts"
        scripts.mkdir()
        for name in ("test-affected.sh", "affected-crates.py"):
            shutil.copy2(ROOT / "scripts" / name, scripts / name)
        metadata_file = self.root / "metadata.json"
        metadata_file.write_text(json.dumps(self.metadata))
        command = ["bash", str(scripts / "test-affected.sh"), "--metadata-file", str(metadata_file)]
        result = subprocess.check_output(command)
        self.assertIn(b"skipping compilation", result)
        (self.root / "crates/types/lib.rs").write_text("// changed\n")
        result = subprocess.check_output(command + ["--dry-run"])
        self.assertIn(b"run-rust-tests.sh -- -p cli -p node -p types", result)
        self.metadata['packages'][1]['name'] = 'aether-node'
        metadata_file.write_text(json.dumps(self.metadata))
        result = subprocess.check_output(command + ["--dry-run"])
        self.assertIn(b"run-rust-tests.sh --ram -- -p aether-node", result)
        blocked = subprocess.run(command + ["--", "--release"], capture_output=True)
        self.assertEqual(blocked.returncode, 2)


if __name__ == "__main__":
    unittest.main()
