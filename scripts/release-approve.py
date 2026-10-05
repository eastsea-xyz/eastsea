#!/usr/bin/env python3
"""Prepare exact release bytes, compare independent builds, and print calldata."""

import argparse
import hashlib
import json
import os
import pathlib
import plistlib
import shutil
import stat
import struct
import subprocess
import sys
import tempfile
import xml.etree.ElementTree as ET

ROOT = pathlib.Path(__file__).resolve().parent.parent
# The pinned production network.json (builder keys + release log contract).
# AETHER_RELEASE_NET repoints the pin at another network.json — a rehearsal
# (scripts/upgrade-drill.sh) needs its own builders and contract address; the
# default is untouched so production runs never read the environment.
NETWORK = pathlib.Path(os.environ.get("AETHER_RELEASE_NET", ROOT / "apps/wallet/Resources/network.json"))


def sha256(path):
    digest = hashlib.sha256()
    with open(path, "rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def executable_sha256(path):
    """Compare code bytes without each Mac's Mach-O signing envelope."""
    description = command("file", "-b", str(path))
    if "Mach-O" not in description:
        return sha256(path)
    (ROOT / "tmp").mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(dir=ROOT / "tmp") as directory:
        copy = pathlib.Path(directory) / "executable"
        shutil.copy2(path, copy)
        if "universal binary" in description:
            thin = pathlib.Path(directory) / "arm64"
            subprocess.run(["lipo", "-thin", "arm64", str(copy), "-output", str(thin)], check=True, capture_output=True)
            copy = thin
        signed = subprocess.run(["codesign", "-dv", str(copy)], capture_output=True).returncode == 0
        if signed:
            subprocess.run(["codesign", "--remove-signature", str(copy)], check=True, capture_output=True)
        binary = bytearray(copy.read_bytes())
        # `codesign --remove-signature` removes LC_CODE_SIGNATURE and truncates
        # the blob, but leaves __LINKEDIT size in its load command dependent on
        # the previous signing envelope. Normalize those two size words only;
        # hash every byte of the remaining executable, including LINKEDIT.
        if binary[:4] != b"\xcf\xfa\xed\xfe":
            raise ValueError("expected arm64 64-bit little-endian Mach-O executable")
        count = struct.unpack_from("<I", binary, 16)[0]
        offset = 32
        found = False
        for _ in range(count):
            if offset + 8 > len(binary):
                raise ValueError("truncated Mach-O load commands")
            kind, size = struct.unpack_from("<II", binary, offset)
            if size < 8 or offset + size > len(binary):
                raise ValueError("invalid Mach-O load command")
            if kind == 0x19 and binary[offset + 8:offset + 24].rstrip(b"\0") == b"__LINKEDIT":
                if size < 72:
                    raise ValueError("invalid __LINKEDIT segment command")
                binary[offset + 32:offset + 40] = b"\0" * 8
                binary[offset + 48:offset + 56] = b"\0" * 8
                found = True
            offset += size
        if not found:
            raise ValueError("Mach-O has no __LINKEDIT segment")
        return hashlib.sha256(binary).hexdigest()


def bundle_inventory(app):
    entries = []
    for path in sorted(app.rglob("*")):
        relative = path.relative_to(app)
        if "_CodeSignature" in relative.parts and relative.name == "CodeResources":
            continue
        name = "EastSea.app/" + relative.as_posix()
        if path.is_symlink():
            entries.append({"name": name, "kind": "symlink", "sha256": hashlib.sha256(("symlink:" + str(path.readlink())).encode()).hexdigest()})
        elif path.is_dir():
            continue
        elif path.is_file():
            entries.append({"name": name, "kind": "file", "mode": oct(stat.S_IMODE(path.stat().st_mode)), "sha256": executable_sha256(path)})
        else:
            raise ValueError(f"unexpected app-bundle entry: {path}")
    if not entries:
        raise ValueError("empty app bundle")
    return entries


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=True).encode()


def command(*argv):
    return subprocess.check_output(argv, text=True).strip()


def contract_hash(_args):
    bytecode = subprocess.check_output(["forge", "inspect", "ReleaseLog", "deployedBytecode"],
        cwd=ROOT / "contracts", text=True).strip()
    print("ReleaseLog runtime code hash: " + command("cast", "keccak", bytecode))


def pinned_builders(manifest):
    network = json.loads(NETWORK.read_bytes())
    keys = network.get("builder_keys")
    if network.get("chain_id") != manifest["chain_id"] or network.get("release_log", "").lower() != manifest["log_address"].lower():
        raise ValueError("bundled network.json does not pin this chain and ReleaseLog")
    if not isinstance(keys, list) or len(keys) != 3 or len(set(k.lower() for k in keys)) != 3:
        raise ValueError("bundled network.json must pin three distinct builder keys")
    code_hash = network.get("release_log_code_hash", "")
    if not isinstance(code_hash, str) or not code_hash.startswith("0x") or len(code_hash) != 66 or not all(c in "0123456789abcdefABCDEF" for c in code_hash[2:]):
        raise ValueError("bundled network.json must pin the ReleaseLog runtime code hash")
    if any(len(key) != 130 or not all(c in "0123456789abcdefABCDEF" for c in key) for key in keys):
        raise ValueError("builder keys must be three 65-byte P-256 public keys")
    return set(k.lower() for k in keys)


def prepare(args):
    source_tag = getattr(args, "source_tag", None)
    if source_tag and command("git", "-C", str(ROOT), "rev-parse", f"refs/tags/{source_tag}^{{commit}}") != command("git", "-C", str(ROOT), "rev-parse", "HEAD"):
        raise ValueError("release tag does not name the source commit being built")
    app = pathlib.Path(args.app)
    sparkle_info = app / "Contents/Frameworks/Sparkle.framework/Resources/Info.plist"
    sparkle_version = plistlib.loads(sparkle_info.read_bytes()).get("CFBundleShortVersionString", "") if sparkle_info.exists() else ""
    inventory = bundle_inventory(app)
    artifacts = [
        {"name": "EastSea.dmg", "sha256": sha256(args.dmg)},
        {"name": "EastSea.app/Contents/MacOS/EastSea", "sha256": executable_sha256(app / "Contents/MacOS/EastSea"), "normalization": "remove-codesign"},
        {"name": "EastSea.app/Contents/Helpers/aether", "sha256": executable_sha256(app / "Contents/Helpers/aether"), "normalization": "remove-codesign"},
        {"name": "EastSea.app/Contents/Helpers/aether-agent", "sha256": executable_sha256(app / "Contents/Helpers/aether-agent"), "normalization": "remove-codesign"},
    ]
    manifest = {
        "artifacts": sorted(artifacts, key=lambda item: item["name"]),
        "build": args.build,
        "bundle_inventory_sha256": hashlib.sha256(canonical(inventory)).hexdigest(),
        "chain_id": args.chain_id,
        "emergency": args.emergency,
        "log_address": args.log.lower(),
        "platform": "macos-arm64-dmg",
        "sparkle_ed_signature": args.sparkle_signature,
        "source_tag": source_tag or "",
        "toolchains": {
            "source_commit": command("git", "-C", str(ROOT), "rev-parse", "HEAD"),
            "source_date_epoch": int(command("git", "-C", str(ROOT), "show", "-s", "--format=%ct", "HEAD")),
            "rustc": command("rustc", "--version"),
            "xcode": command("xcodebuild", "-version"),
            "macos_sdk": command("xcrun", "--sdk", "macosx", "--show-sdk-version"),
            "sparkle": sparkle_version,
        },
        "version": args.version,
    }
    if len(args.log) != 42 or not args.log.startswith("0x") or int(args.log[2:], 16) == 0:
        raise ValueError("ReleaseLog address must be a nonzero 20-byte hex address")
    output = pathlib.Path(args.out)
    output.write_bytes(canonical(manifest))
    output.with_suffix(".inventory.json").write_bytes(canonical(inventory))
    print(f"manifest: {output} SHA-256 {sha256(output)}")
    for item in artifacts:
        print(f"{item['sha256']}  {item['name']}")
    print(f"Next: compare a second Mac's rebuild, then scripts/builder-sign sign {output} --local-manifest REBUILT.json")


def compare(args):
    base = json.loads(pathlib.Path(args.manifest).read_bytes())
    for other_path in args.other:
        other = json.loads(pathlib.Path(other_path).read_bytes())
        if other != base:
            print(f"ALARM: builder manifest mismatch: {other_path}", file=sys.stderr)
            a = {item["name"]: item["sha256"] for item in base["artifacts"]}
            b = {item["name"]: item["sha256"] for item in other["artifacts"]}
            for name in sorted(a.keys() | b.keys()):
                if a.get(name) != b.get(name):
                    print(f"  {name}: {a.get(name)} != {b.get(name)}", file=sys.stderr)
            if base.get("bundle_inventory_sha256") != other.get("bundle_inventory_sha256"):
                print("  app-bundle inventory hash differs", file=sys.stderr)
                files = []
                for path in (args.manifest, other_path):
                    inventory = pathlib.Path(path).with_suffix(".inventory.json")
                    files.append({item["name"]: item["sha256"] for item in json.loads(inventory.read_bytes())} if inventory.exists() else {})
                for name in sorted(files[0].keys() | files[1].keys()):
                    if files[0].get(name) != files[1].get(name):
                        print(f"  {name}: {files[0].get(name)} != {files[1].get(name)}", file=sys.stderr)
            raise SystemExit(1)
    print("builder manifests match")


def rebuild(args):
    original = json.loads(pathlib.Path(args.manifest).read_bytes())
    prep = argparse.Namespace(chain_id=original["chain_id"], log=original["log_address"],
        version=original["version"], build=original["build"], dmg=args.dmg,
        app=args.app, out=args.out, emergency=original["emergency"],
        sparkle_signature=original["sparkle_ed_signature"], source_tag=original["source_tag"])
    prepare(prep)
    compare(argparse.Namespace(manifest=args.manifest, other=[args.out]))


def combine(args):
    manifest_path = pathlib.Path(args.manifest)
    manifest = manifest_path.read_bytes()
    parsed = json.loads(manifest)
    if canonical(parsed) != manifest:
        raise ValueError("manifest is not canonical JSON")
    allowed = pinned_builders(parsed)
    signatures = []
    seen = set()
    for path in args.signatures:
        sig = json.loads(pathlib.Path(path).read_bytes())
        if sig["manifest_sha256"] != hashlib.sha256(manifest).hexdigest():
            raise ValueError(f"ALARM: {path} signs a different manifest")
        subprocess.run(["scripts/builder-sign", "verify", str(manifest_path), path], check=True)
        key = sig["public_key"].lower()
        if key in seen:
            raise ValueError(f"duplicate builder key: {sig['public_key']}")
        if key not in allowed:
            raise ValueError(f"builder key is not pinned: {sig['public_key']}")
        seen.add(key)
        signatures.append({"public_key": sig["public_key"], "signature": sig["signature"]})
    required = 3 if parsed["emergency"] else 2
    if len(signatures) < required or len(signatures) > 3:
        raise ValueError(f"need {required} to 3 distinct builder signatures")
    output = pathlib.Path(args.out)
    output.write_bytes(canonical(sorted(signatures, key=lambda item: item["public_key"])))
    archive = next(item["sha256"] for item in parsed["artifacts"] if item["name"] == "EastSea.dmg")
    print(f"builder signatures: {output} SHA-256 {sha256(output)}")
    print("Approve by submitting this calldata to ReleaseLog " + parsed["log_address"] + ":")
    print("cast calldata 'publish(bytes,bytes32,bytes,bool)' " +
          "0x" + manifest.hex() + " 0x" + archive + " 0x" + output.read_bytes().hex() +
          " " + str(parsed["emergency"]).lower())
    print("Wait for a finalized entry; ordinary releases remain pending for 72 hours.")


def finalize(args):
    if args.index < 0:
        raise ValueError("release index must be nonnegative")
    manifest_path = pathlib.Path(args.manifest)
    raw = manifest_path.read_bytes()
    manifest = json.loads(raw)
    if canonical(manifest) != raw:
        raise ValueError("manifest is not canonical JSON")
    archive = next(item["sha256"] for item in manifest["artifacts"] if item["name"] == "EastSea.dmg")
    if sha256(args.dmg) != archive:
        raise ValueError("ALARM: prepared DMG differs from builder-approved hash")
    inventory = manifest_path.with_suffix(".inventory.json")
    if not inventory.exists() or sha256(inventory) != manifest["bundle_inventory_sha256"]:
        raise ValueError("ALARM: prepared app inventory differs from builder-approved hash")
    if manifest["version"] != args.version or manifest["build"] != args.build:
        raise ValueError("ALARM: project version/build differs from the prepared manifest")
    tag = manifest.get("source_tag")
    if not tag or tag != "app-v" + args.version or command("git", "-C", str(ROOT), "rev-parse", f"refs/tags/{tag}^{{commit}}") != manifest["toolchains"]["source_commit"]:
        raise ValueError("ALARM: release tag differs from the prepared source")
    feed = ET.parse(args.appcast)
    enclosure = feed.find("./channel/item/enclosure")
    if enclosure is None or enclosure.get("{http://www.andymatuschak.org/xml-namespaces/sparkle}edSignature") != manifest["sparkle_ed_signature"]:
        raise ValueError("ALARM: Sparkle appcast signature differs from the builder-approved manifest")
    subprocess.run(["scripts/builder-sign", "verify-bundle", str(manifest_path), args.signatures], check=True)
    sigs = json.loads(pathlib.Path(args.signatures).read_bytes())
    if manifest["emergency"] and len(sigs) != 3:
        raise ValueError("emergency release needs all three builders")
    allowed = pinned_builders(manifest)
    if any(item["public_key"].lower() not in allowed for item in sigs):
        raise ValueError("builder signature is not from a pinned key")
    pathlib.Path(args.out).write_bytes(canonical({"index": args.index}))
    print(f"publication index {args.index}; archive SHA-256 {archive}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("contract-hash")
    prep = commands.add_parser("prepare")
    for flag in ("chain-id", "log", "version", "build", "dmg", "app", "out", "sparkle-signature"):
        prep.add_argument("--" + flag, required=True, type=int if flag == "chain-id" else str)
    prep.add_argument("--emergency", action="store_true")
    prep.add_argument("--source-tag")
    check = commands.add_parser("compare")
    check.add_argument("manifest")
    check.add_argument("other", nargs="+")
    rebuilt = commands.add_parser("rebuild")
    rebuilt.add_argument("manifest")
    for flag in ("dmg", "app", "out"):
        rebuilt.add_argument("--" + flag, required=True)
    join = commands.add_parser("combine")
    join.add_argument("manifest")
    join.add_argument("signatures", nargs="+")
    join.add_argument("--out", required=True)
    done = commands.add_parser("finalize")
    for flag in ("manifest", "signatures", "dmg", "appcast", "version", "build", "out"):
        done.add_argument("--" + flag, required=True)
    done.add_argument("--index", type=int, required=True)
    args = parser.parse_args()
    try:
        {"contract-hash": contract_hash, "prepare": prepare, "compare": compare, "rebuild": rebuild,
         "combine": combine, "finalize": finalize}[args.command](args)
    except (OSError, ValueError, KeyError, subprocess.CalledProcessError) as error:
        parser.exit(1, f"ALARM: {error}\n")


if __name__ == "__main__":
    main()
