#!/usr/bin/env bash
# Carry a verified previous node and its prover into a clean app build.
# Private dylibs are deliberately unsupported: reject the release rather than
# silently bind an old helper to libraries from the replacement app or host.
# The runtime additionally checks the chain's scheduled protocol before rollback.
set -euo pipefail
cd "$(dirname "$0")/.."
prev=${1:?previous app required}
app=${2:?new app required}
team=45WU468FZE
signer_requirement='anchor apple generic and certificate leaf[subject.OU] = "45WU468FZE" and certificate leaf[field.1.2.840.113635.100.6.1.13] exists'
refuse() { echo "rollback package: $*" >&2; exit 1; }
[ -d "$prev" ] || refuse "previous app is missing: $prev"
id=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$prev/Contents/Info.plist" 2>/dev/null || true)
[ "$id" = com.pipln.eastsea ] || refuse "previous app has an unexpected bundle id: $id"
codesign --verify --deep --strict -R="$signer_requirement" "$prev" || refuse "previous app signature is invalid"

verify_code() {
  local file=$1 signed_team
  codesign --verify --strict -R="$signer_requirement" "$file" \
    || refuse "previous helper signature is invalid: $file"
  signed_team=$(codesign -dv "$file" 2>&1 | sed -n 's/^TeamIdentifier=//p')
  [ "$signed_team" = "$team" ] || refuse "previous helper has unexpected signing team: $signed_team"
}
inspect_dependencies() {
  local helper=$1 libraries dependencies dependency
  libraries=$(otool -L "$helper") || refuse "cannot inspect previous helper dependencies: $helper"
  dependencies=$(printf '%s\n' "$libraries" | awk '/^[[:space:]]+[^[:space:]]/ {print $1}')
  [ -n "$dependencies" ] || refuse "no previous helper dependency information: $helper"
  while IFS= read -r dependency; do
    case "$dependency" in
      /usr/lib/*|/System/Library/*) : ;;
      *) refuse "unsupported dependency '$dependency' in previous helper $helper" ;;
    esac
  done <<< "$dependencies"
}
inspect_platform() {
  # Running `protocol` on the builder is insufficient: Rosetta can hide an
  # absent native slice, and the builder may run a newer macOS than customers.
  python3 - "$app" "$1" <<'PY'
import plistlib, re, subprocess, sys
from pathlib import Path

app, previous = Path(sys.argv[1]), Path(sys.argv[2])
replacement = app / "Contents/Helpers/aether"

def refuse(message):
    print("rollback package: " + message, file=sys.stderr)
    sys.exit(1)

def output(arguments, description):
    try:
        return subprocess.run(arguments, check=True, capture_output=True, text=True).stdout
    except (OSError, subprocess.CalledProcessError, UnicodeError):
        refuse("cannot read " + description)

def architectures(binary):
    values = output(["lipo", "-archs", str(binary)], "architecture metadata for " + str(binary)).split()
    known = {"arm64", "arm64e", "x86_64", "x86_64h", "i386"}
    if not values or len(values) != len(set(values)) or any(value not in known for value in values):
        refuse("missing or malformed architecture metadata for " + str(binary))
    return values

def version(value):
    if not isinstance(value, str) or not re.fullmatch(r"[0-9]+(?:\.[0-9]+){0,2}", value):
        refuse("missing or malformed minimum OS metadata: " + repr(value))
    raw = value.split(".")
    if len(raw[0]) > 5 or any(len(part) > 3 for part in raw[1:]):
        refuse("invalid macOS minimum OS metadata: " + repr(value))
    parts = tuple(int(part) for part in raw)
    parts += (0,) * (3 - len(parts))
    if not 10 <= parts[0] <= 65535 or any(part > 255 for part in parts[1:]):
        refuse("invalid macOS minimum OS metadata: " + repr(value))
    return parts

def minimum_os(binary, architecture):
    data = output(["otool", "-arch", architecture, "-l", str(binary)],
                  "minimum OS metadata for " + str(binary) + " (" + architecture + ")")
    commands = list(re.finditer(r"(?m)^\s*cmd (LC_[A-Z0-9_]+)\s*$", data))
    minimum = []
    for i, command in enumerate(commands):
        name = command.group(1)
        if name not in {"LC_BUILD_VERSION", "LC_VERSION_MIN_MACOSX"}:
            continue
        end = commands[i + 1].start() if i + 1 < len(commands) else len(data)
        block = data[command.end():end]
        def field(key):
            values = re.findall(r"(?m)^\s*" + key + r"\s+([^\s]+)\s*$", block)
            if len(values) != 1:
                refuse("missing or malformed minimum OS metadata for " + str(binary))
            return values[0]
        if name == "LC_BUILD_VERSION" and field("platform").upper() not in {"1", "MACOS"}:
            refuse("previous/replacement helper has a non-macOS platform: " + str(binary))
        minimum.append(version(field("minos" if name == "LC_BUILD_VERSION" else "version")))
    if len(minimum) != 1:
        refuse("missing or ambiguous minimum OS load command for " + str(binary))
    return minimum[0]

try:
    with (app / "Contents/Info.plist").open("rb") as stream:
        info = plistlib.load(stream)
except (OSError, plistlib.InvalidFileException, ValueError):
    refuse("cannot read replacement app minimum OS metadata")
if not isinstance(info, dict):
    refuse("malformed replacement app minimum OS metadata")
app_minimum = version(info["LSMinimumSystemVersion"]) if "LSMinimumSystemVersion" in info else None
required = architectures(replacement)
available = architectures(previous)
for architecture in required:
    if architecture not in available:
        refuse("previous helper is missing required architecture " + architecture + ": " + str(previous))
    replacement_minimum = minimum_os(replacement, architecture)
    supported_minimum = app_minimum if app_minimum is not None else replacement_minimum
    if replacement_minimum > supported_minimum:
        refuse("replacement node minimum OS exceeds the app minimum OS")
    if minimum_os(previous, architecture) > supported_minimum:
        refuse("previous helper minimum OS exceeds the replacement-supported minimum OS: " + str(previous))
PY
}
previous_team=$(codesign -dv "$prev" 2>&1 | sed -n 's/^TeamIdentifier=//p')
[ "$previous_team" = "$team" ] || refuse "previous app has unexpected signing team: $previous_team"
node="$prev/Contents/Helpers/aether"
prover="$prev/Contents/Helpers/aether-prover"
[ -x "$node" ] && [ ! -L "$node" ] || refuse "previous node is missing or a symlink"
[ -x "$prover" ] && [ ! -L "$prover" ] || refuse "previous prover is missing or a symlink"
for helper in "$node" "$prover"; do
  verify_code "$helper"
  inspect_dependencies "$helper"
  inspect_platform "$helper"
done

destination="$app/Contents/Helpers/NodeRollback.bundle"
[ -d "$app/Contents/Helpers" ] || refuse "new app helper directory is missing"
[ ! -e "$destination" ] && [ ! -L "$destination" ] || refuse "rollback destination already exists"
mkdir -p "$PWD/tmp"
work=$(mktemp -d "$PWD/tmp/package-rollback.XXXXXX")
trap 'rm -rf "${work:?}"' EXIT
bundle="$work/NodeRollback.bundle"
mkdir -p "$bundle/Contents/MacOS"
cp -p "$node" "$bundle/Contents/MacOS/aether.prev"
cp -p "$prover" "$bundle/Contents/MacOS/aether-prover"
# Verify the copies before executing either. A source changed during the copy
# cannot turn the earlier signature check into permission to run unsigned code.
verify_code "$bundle/Contents/MacOS/aether.prev"
verify_code "$bundle/Contents/MacOS/aether-prover"
inspect_dependencies "$bundle/Contents/MacOS/aether.prev"
inspect_dependencies "$bundle/Contents/MacOS/aether-prover"
inspect_platform "$bundle/Contents/MacOS/aether.prev"
inspect_platform "$bundle/Contents/MacOS/aether-prover"
protocol=$("$bundle/Contents/MacOS/aether.prev" protocol) || refuse "cannot read previous node protocol"
[[ "$protocol" =~ ^[0-9]+$ ]] || refuse "invalid previous node protocol: $protocol"
# Both versions must prove against the network carried by the replacement app.
# This keeps the old prover isolated without weakening the release prover gate.
scripts/prover-gate.sh --prover "$bundle/Contents/MacOS/aether-prover" \
  --node "$app/Contents/Helpers/aether" --network "$app/Contents/Resources/network.json"
python3 - "$prev/Contents/Info.plist" "$bundle/Contents/Info.plist" <<'PY'
import plistlib, sys
with open(sys.argv[1], "rb") as f:
    previous = plistlib.load(f)
with open(sys.argv[2], "wb") as f:
    plistlib.dump({
        "CFBundleIdentifier": "com.pipln.eastsea.node-rollback",
        "CFBundleName": "NodeRollback",
        "CFBundlePackageType": "BNDL",
        "CFBundleExecutable": "aether.prev",
        "CFBundleVersion": previous["CFBundleVersion"],
    }, f)
PY
mv "$bundle" "$destination"
echo "rollback package: verified previous node (protocol $protocol) and adjacent prover"
