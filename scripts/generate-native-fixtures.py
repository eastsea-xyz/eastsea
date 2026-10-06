#!/usr/bin/env python3
"""Compile the native-plan B0-B2 head-to-head fixtures into per-project manifests.

Each project keeps its own `artifacts.json` (creation bytecode, runtime, ABI,
compiler profile and input SHA-256s) so licences stay separated:

- fixtures/native: eastsea-toolbox native/ B0-B2 templates (MIT), unmodified.
- fixtures/gpl/merkle-distributor: Uniswap MerkleDistributor (GPL-3.0-or-later)
  with its OpenZeppelin 4.7.0 imports (MIT), unmodified. Test use only.

The shared fixtures/artifacts.json is not touched. Run from anywhere:

    python3 scripts/generate-native-fixtures.py --offline
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess

ROOT = Path(__file__).resolve().parents[1]
FIXTURES = ROOT / "crates/contracts-onchain/fixtures"
PROJECTS = {
    "native": {
        "dir": FIXTURES / "native",
        "contracts": {
            "ClaimCampaigns": "claims/src/ClaimCampaigns.sol",
            "GrantLedger": "streams/src/GrantLedger.sol",
            "EscrowBook": "escrow/src/EscrowBook.sol",
        },
        "origin": "eastsea-toolbox native/ (public), main a7e8724b88aab9da64ae269dce9716d07238c872",
        "licence": "MIT (SPDX headers)",
        "profile": "solc 0.8.31, osaka, optimizer 200",
    },
    "gpl": {
        "dir": FIXTURES / "gpl/merkle-distributor",
        "contracts": {"MerkleDistributor": "MerkleDistributor.sol"},
        "origin": "Uniswap/merkle-distributor 25a79e8ec8c22076a735b1a675b961c8184e7931; OpenZeppelin/openzeppelin-contracts v4.7.0",
        "licence": "GPL-3.0-or-later (OpenZeppelin imports: MIT)",
        "profile": "solc 0.8.17, default EVM (london), optimizer 5000",
    },
}


def build(forge: Path, project: Path, offline: bool, env: dict[str, str]) -> None:
    cmd = [str(forge), "build", "--root", str(project), "--threads", "4", "--quiet", "--force"]
    if offline:
        cmd.append("--offline")
    subprocess.run(cmd, cwd=ROOT, env=env, check=True)


def manifest(namespace: str, spec: dict) -> dict:
    project: Path = spec["dir"]
    artifacts = {}
    for name, source in spec["contracts"].items():
        compiled = json.loads((project / "out" / Path(source).name / f"{name}.json").read_text())
        bytecode = compiled["bytecode"]["object"]
        if not bytecode or bytecode == "0x" or "__" in bytecode:
            raise ValueError(f"{namespace}/{name}: missing or unlinked bytecode")
        artifacts[f"{namespace}/{name}"] = {
            "bytecode": bytecode,
            "runtime": compiled["deployedBytecode"]["object"],
            "abi": compiled["abi"],
        }
    inputs = sorted([*project.rglob("*.sol"), *project.rglob("LICENSE"), project / "foundry.toml"])
    inputs = [p for p in inputs if "out" not in p.relative_to(project).parts and "cache" not in p.relative_to(project).parts]
    return {
        "schema": 1,
        "origin": spec["origin"],
        "licence": spec["licence"],
        "profile": spec["profile"],
        "sources": {str(p.relative_to(ROOT)): hashlib.sha256(p.read_bytes()).hexdigest() for p in inputs},
        "artifacts": artifacts,
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--forge", default=shutil.which("forge") or str(Path.home() / ".foundry/bin/forge"))
    parser.add_argument("--offline", action="store_true", help="require locally installed pinned solc versions")
    args = parser.parse_args()
    forge = Path(args.forge).expanduser().resolve()
    if not forge.is_file() or not os.access(forge, os.X_OK):
        parser.error(f"forge is not executable: {forge}")
    scratch = ROOT / "tmp"
    scratch.mkdir(exist_ok=True)
    env = {k: v for k, v in os.environ.items() if not k.startswith("FOUNDRY_")}
    env.update({"TMPDIR": str(scratch), "FOUNDRY_PROFILE": "default"})
    for namespace, spec in PROJECTS.items():
        print(f"Building {namespace} with 4 Foundry threads", flush=True)
        build(forge, spec["dir"], args.offline, env)
        out = spec["dir"] / "artifacts.json"
        out.write_text(json.dumps(manifest(namespace, spec), indent=1, sort_keys=True) + "\n")
        print(f"Wrote {out.relative_to(ROOT)}")


if __name__ == "__main__":
    main()
