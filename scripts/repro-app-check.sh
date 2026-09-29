#!/usr/bin/env bash
# Tier-2 reproducible check for the macOS app bundle (gap G5). The shipped
# Aether.app cannot be byte-identical: Xcode re-signs it (Developer ID, with the
# notarization ticket stapled), and the signature covers the whole bundle. So
# compare what signing does not change: strip the signature from copies of each
# Mach-O in the bundle and compare SHA-256 and LC_UUID. Matching LC_UUID means
# the linker produced the same image; matching stripped hashes means the code
# and data are identical and only the signature differs.
#
#   scripts/repro-app-check.sh                 # build the app twice, compare
#   scripts/repro-app-check.sh A.app B.app     # compare two built bundles
#
# Building needs the Developer ID certificate (see apps/wallet/project.yml) and
# the Jolt fork for the embedded prover; copying the tree twice needs disk.
#   AETHER_REPRO_WORK=<dir>  where to put the trees (default $TMPDIR)
#   AETHER_REPRO_KEEP=1      keep them
set -euo pipefail

repo="$(cd "$(dirname "$0")/.." && pwd)"

usage() {
  echo "usage: $0 [App.app App.app]" >&2
  exit 2
}

uuid_of() {
  local u
  u=$(dwarfdump --uuid "$1" 2>/dev/null | awk 'NR==1 {print $2}')
  # otool prints "cmd LC_UUID" / "cmdsize 24" / "uuid <value>", so skip two lines.
  [ -n "$u" ] || u=$(otool -l "$1" 2>/dev/null | awk '/cmd LC_UUID/ {getline; getline; print $2}')
  echo "${u:-none}"
}

# Copy every Mach-O in the bundle to $dest (same relative layout), strip its
# signature and write "path<TAB>sha256<TAB>uuid" lines to $manifest.
collect() {
  local app="$1" dest="$2" manifest="$3" f rel out
  rm -rf "$dest"
  mkdir -p "$dest"
  : > "$manifest"
  while IFS= read -r -d '' f; do
    case "$(file -b "$f")" in
      Mach-O*) ;;
      *) continue ;;
    esac
    rel="${f#"$app"/}"
    case "$rel" in _CodeSignature/*) continue ;; esac
    out="$dest/$rel"
    mkdir -p "$(dirname "$out")"
    cp -p "$f" "$out"
    # Removing the signature is not guaranteed to give back the pre-signing
    # bytes (the linker's ad-hoc signature is already part of the file), but it
    # is applied to both sides, so a difference still means different code.
    codesign --remove-signature "$out" >/dev/null 2>&1 || true
    printf '%s\t%s\t%s\n' "$rel" "$(shasum -a 256 "$out" | awk '{print $1}')" "$(uuid_of "$out")" >> "$manifest"
  done < <(find "$app" -type f ! -path '*.dSYM/*' -print0)
}

work=""
if [ $# -eq 2 ]; then
  for d in "$1" "$2"; do
    [ -d "$d" ] || { echo "not a bundle: $d" >&2; usage; }
  done
  a=$(cd "$1" && pwd)
  b=$(cd "$2" && pwd)
  work=
elif [ $# -eq 0 ]; then
  work="${AETHER_REPRO_WORK:-${TMPDIR:-/tmp}}"
  work="$(mktemp -d "$work/aether-repro-app-XXXXXX")"
  keep="${AETHER_REPRO_KEEP:-}"
  cleanup() {
    if [ -n "$keep" ]; then echo "kept $work" >&2; else rm -rf "$work"; fi
  }
  trap cleanup EXIT
  a="$work/a" b="$work/bbbbbbbbbb"

  # One timestamp for both sides (see repro-env.sh).
  . "$repo/scripts/repro-env.sh"
  echo "epoch      $SOURCE_DATE_EPOCH"
  echo "work       $work"

  for d in "$a" "$b"; do
    rsync -a --delete \
      --exclude '.git' --exclude 'target' --exclude 'target-*' \
      --exclude '.claude' --exclude 'dist' --exclude 'node_modules' \
      --exclude 'apps/wallet/build' --exclude '*.log' \
      "$repo/" "$d/"
  done

  build_side() {
    local dir="$1"
    if (
      set -euo pipefail
      cd "$dir"
      export CARGO_TARGET_DIR="$dir/target"
      scripts/build-wallet.sh macos >/dev/null
    ) >"$dir/build.log" 2>&1; then
      echo 0 > "$dir/status"
    else
      echo $? > "$dir/status"
    fi
  }

  echo "building $a ..."
  build_side "$a"
  echo "building $b ..."
  build_side "$b"
  for d in "$a" "$b"; do
    if [ "$(cat "$d/status")" != 0 ]; then
      echo "build failed in $d (see $d/build.log)" >&2
      tail -20 "$d/build.log" >&2
      exit 1
    fi
  done
  a="$a/apps/wallet/build/Build/Products/Release/Aether.app"
  b="$b/apps/wallet/build/Build/Products/Release/Aether.app"
else
  usage
fi

if [ -n "$work" ]; then out="$work"; else out="$(mktemp -d "${TMPDIR:-/tmp}/aether-repro-app-out-XXXXXX")"; fi
collect "$a" "$out/stripped-a" "$out/a.tsv"
collect "$b" "$out/stripped-b" "$out/b.tsv"

python3 - "$out/a.tsv" "$out/b.tsv" "$a" "$b" <<'PY'
import sys

def load(path):
    rows = {}
    with open(path) as fp:
        for line in fp:
            rel, sha, uuid = line.rstrip("\n").split("\t")
            rows[rel] = (sha, uuid)
    return rows

left, right, name_a, name_b = (load(sys.argv[1]), load(sys.argv[2]),
                              sys.argv[3], sys.argv[4])
names = sorted(set(left) | set(right))
bad = 0
print(f"{'mach-o in the bundle':<44} {'stripped sha256':<9} {'lc_uuid'}")
print(f"{'-'*44} {'-'*9} {'-'*9}")
if not names:
    print("no Mach-O files found — is this a built .app?")
    sys.exit(1)
for rel in names:
    name = rel if len(rel) <= 42 else "…" + rel[-41:]
    if rel not in left or rel not in right:
        print(f"{name:<44} {'missing':<9} (only in {name_a if rel in left else name_b})")
        bad += 1
        continue
    sha_ok = left[rel][0] == right[rel][0]
    uuid_ok = left[rel][1] == right[rel][1]
    print(f"{name:<44} {'ok' if sha_ok else 'DIFFER':<9} {'ok' if uuid_ok else 'DIFFER'}")
    if not sha_ok:
        print(f"    {name_a}: {left[rel][0]}")
        print(f"    {name_b}: {right[rel][0]}")
    if not uuid_ok:
        print(f"    {name_a}: {left[rel][1]}")
        print(f"    {name_b}: {right[rel][1]}")
    if not (sha_ok and uuid_ok):
        bad += 1

print()
if bad:
    print(f"NOT reproducible: {bad} of {len(names)} Mach-O files differ")
    print("(A bundle that differs only in its signature counts as matching above.)")
    sys.exit(1)
print(f"reproducible: all {len(names)} Mach-O files match once signatures are stripped")
PY
