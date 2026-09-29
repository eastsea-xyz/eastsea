#!/usr/bin/env bash
# Tier-2 reproducible check for the macOS app bundle (gap G5). The shipped
# Aether.app cannot be byte-identical: Xcode re-signs it (Developer ID, with the
# notarization ticket stapled), and the signature covers the whole bundle. So
# compare what signing does not change: strip the signature from copies of each
# Mach-O in the bundle and compare SHA-256. Matching stripped hashes means the
# code and data are identical and only the signature (and the linker's UUID)
# differs.
#
# The UUID is zeroed before hashing (scripts/macho-uuid.py) and reported, not
# compared: -reproducible still leaves an LC_UUID that follows the link inputs,
# so the app binary — which Xcode links from a build directory — carries a
# different one when the checkout lives elsewhere. Xcode's own link keeps it
# because dSYM lookup matches on the UUID; the Rust binaries the bundle embeds
# (Helpers/) are rewritten to a content UUID by aether_repro_fix_uuid instead,
# so they match byte for byte and only Xcode's two binaries show "differs".
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
# signature, zero its UUID and write "path<TAB>sha256<TAB>uuid" lines to
# $manifest. The UUID is read before it is zeroed, so the manifest still shows
# what the linker produced.
collect() {
  local app="$1" dest="$2" manifest="$3" f rel out uuid
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
    uuid="$(uuid_of "$out")"
    python3 "$repo/scripts/macho-uuid.py" zero "$out"
    printf '%s\t%s\t%s\n' "$rel" "$(shasum -a 256 "$out" | awk '{print $1}')" "$uuid" >> "$manifest"
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
bad = uuid_differs = 0
print(f"{'mach-o in the bundle':<44} {'stripped sha256':<15} {'lc_uuid'}")
print(f"{'-'*44} {'-'*15} {'-'*9}")
if not names:
    print("no Mach-O files found — is this a built .app?")
    sys.exit(1)
for rel in names:
    name = rel if len(rel) <= 42 else "…" + rel[-41:]
    if rel not in left or rel not in right:
        print(f"{name:<44} {'missing':<15} (only in {name_a if rel in left else name_b})")
        bad += 1
        continue
    # The hash has the UUID zeroed out, so it is the code and data; the UUID
    # itself is informational (see the header: it follows the input paths).
    sha_ok = left[rel][0] == right[rel][0]
    if left[rel][1] == right[rel][1]:
        uuid_note = "same"
    elif "none" in (left[rel][1], right[rel][1]):
        uuid_note = "none"
    else:
        uuid_note = "differs"
        uuid_differs += 1
    print(f"{name:<44} {'ok' if sha_ok else 'DIFFER':<15} {uuid_note}")
    if not sha_ok:
        print(f"    {name_a}: {left[rel][0]}")
        print(f"    {name_b}: {right[rel][0]}")
        bad += 1

print()
if uuid_differs:
    print(f"note: {uuid_differs} of {len(names)} Mach-O files carry a different LC_UUID.")
    print("      That is the linker's, not the code's: ld64 derives it from the")
    print("      input paths, so an Xcode build in another directory always differs.")
if bad:
    print(f"NOT reproducible: {bad} of {len(names)} Mach-O files differ")
    print("(A bundle that differs only in its signature and UUID counts as matching above.)")
    sys.exit(1)
print(f"reproducible: all {len(names)} Mach-O files match once signatures are stripped")
print("              (UUID field zeroed; codesign/LD_UUID is not part of the comparison)")
PY
