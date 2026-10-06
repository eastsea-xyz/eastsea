#!/usr/bin/env python3
"""Compile local Solidity sources and write the executor-suite fixture manifest.

No toolbox checkout is needed: its source snapshot and transitive dependencies
are checked into crates/contracts-onchain/fixtures/toolbox.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess

ROOT = Path(__file__).resolve().parents[1]
FIXTURES = ROOT / "crates/contracts-onchain/fixtures"
PROJECTS = {
    "core": ROOT / "contracts",
    "toolbox": FIXTURES / "toolbox",
    "support": FIXTURES / "support",
}
CONTRACT = re.compile(r"^\s*contract\s+(\w+)\b", re.MULTILINE)
TOOLBOX_REVISION = "093f4f2109424189ac58cc004a7ae611faf8f96f"


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--forge", default=shutil.which("forge") or str(Path.home() / ".foundry/bin/forge"))
    parser.add_argument("--offline", action="store_true", help="require locally installed pinned solc versions")
    args = parser.parse_args()
    forge = Path(args.forge).expanduser().resolve()
    if not forge.is_file() or not os.access(forge, os.X_OK):
        parser.error(f"forge is not executable: {forge}")
    scratch = ROOT / "tmp"
    scratch.mkdir(exist_ok=True)
    env = {**os.environ, "TMPDIR": str(scratch)}
    # Avoid ambient Foundry profiles or output overrides changing fixture code.
    for key in list(env):
        if key.startswith("FOUNDRY_"):
            del env[key]
    env["FOUNDRY_PROFILE"] = "default"
    artifacts = {}
    source_hashes = {}
    for namespace, project in PROJECTS.items():
        cmd = [str(forge), "build", "--root", str(project), "--threads", "4", "--quiet", "--skip", "test", "--skip", "script"]
        if args.offline:
            cmd.append("--offline")
        print(f"Building {namespace} with 4 Foundry threads", flush=True)
        subprocess.run(cmd, cwd=ROOT, env=env, check=True)
        for source in sorted((project / "src").rglob("*.sol")):
            for name in CONTRACT.findall(source.read_text()):
                artifact = project / "out" / source.name / f"{name}.json"
                compiled = json.loads(artifact.read_text())
                bytecode = compiled["bytecode"]["object"]
                runtime = compiled["deployedBytecode"]["object"]
                if not bytecode or bytecode == "0x":
                    raise ValueError(f"concrete contract has no bytecode: {namespace}/{name}")
                if "__" in bytecode:
                    raise ValueError(f"unlinked library in {namespace}/{name}")
                key = f"{namespace}/{name}"
                if key in artifacts:
                    raise ValueError(f"ambiguous fixture name: {key}")
                artifacts[key] = {"bytecode": bytecode, "runtime": runtime, "abi": compiled["abi"]}
        # Include all compiler inputs, configurations, and the vendored license.
        inputs = list((project / "src").rglob("*.sol"))
        if (project / "base").exists():
            inputs.extend((project / "base").rglob("*.sol"))
        if (project / "lib").exists():
            inputs.extend((project / "lib").rglob("*.sol"))
            inputs.extend((project / "lib").rglob("LICENSE"))
        inputs.append(project / "foundry.toml")
        for source in sorted(inputs):
            source_hashes[str(source.relative_to(ROOT))] = hashlib.sha256(source.read_bytes()).hexdigest()
    encoded = json.dumps(dict(sorted(artifacts.items())), separators=(",", ":")) + "\n"
    (FIXTURES / "artifacts.json").write_text(encoded)
    provenance = {
        "schema": 1,
        "toolbox_revision": TOOLBOX_REVISION,
        "toolbox_origin": "/Volumes/workspace/eastsea-toolbox/contracts",
        "toolbox_dependency": "OpenZeppelin Contracts 5.1.0 (MIT), recursive imports only",
        "openzeppelin_revision": "69c8def5f222ff96f2b5beff05dfba996368aa79",
        "profiles": {"core": "solc 0.8.19, optimizer 200, existing contracts/foundry.toml", "toolbox": "solc 0.8.24, Paris, optimizer 200", "support": "solc 0.8.24, Paris, optimizer 200"},
        "local_fixes": [
            "core/TokenVesting: avoid overflowing intermediate multiplication for valid uint256 grants",
            "toolbox/Editions1155: reject packed wallet-cap and price narrowing overflow",
            "toolbox/SubscriptionManager: reject packed expiry and contributed-principal overflow",
            "toolbox/SimpleMultisig: accept native deposits for authorized payouts",
            "toolbox/TokenTimeLock: permit replacing fully withdrawn grants",
            "toolbox/FixedPriceMarket: ignore royalties without a payable receiver",
        ],
        "artifacts_sha256": hashlib.sha256(encoded.encode()).hexdigest(),
        "sources": dict(sorted(source_hashes.items())),
    }
    (FIXTURES / "provenance.json").write_text(json.dumps(provenance, indent=2) + "\n")
    print(f"Wrote {len(artifacts)} deployable contracts to {FIXTURES.relative_to(ROOT)}/artifacts.json")


if __name__ == "__main__":
    main()
