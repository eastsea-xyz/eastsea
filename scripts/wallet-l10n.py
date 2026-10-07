#!/usr/bin/env python3
"""The Mac wallet's one-language rule, checked (founder review of 0.7.0, 2026-10-07).

  wallet-l10n.py check --stringsdata DIR   fail unless every string the compiler extracted is in
                                           Resources/Localizable.xcstrings with a Korean translation
                                           whose placeholders match the English key
  wallet-l10n.py sync --stringsdata DIR    add newly extracted keys to the catalog (untranslated),
                                           drop keys no source uses any more
  wallet-l10n.py lint                      fail on English sentences in Swift sources that never
                                           reach the catalog (a String a screen shows unlocalized)
  wallet-l10n.py missing                   list catalog keys still without Korean

The compiler's .stringsdata files (SWIFT_EMIT_LOC_STRINGS) are the source of truth for what
is localizable: every Text/Label/Button literal and every String(localized:). `lint` catches
the rest - an English sentence kept in a plain String and shown on screen.
"""
import json
import os
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
WALLET = ROOT / "apps" / "wallet"
CATALOG = WALLET / "Resources" / "Localizable.xcstrings"
SOURCES = WALLET / "Sources"
ALLOW = Path(__file__).resolve().parent / "wallet-l10n-allow.txt"

# printf-style specifiers as the compiler writes them into keys.
SPEC = re.compile(r"%(?:(\d+)\$)?([-+ #0]*\d*(?:\.\d+)?)(lld|llu|ld|lu|@|d|u|f|g|e|s|c|x|X|%)")


def load_catalog():
    with open(CATALOG, encoding="utf-8") as f:
        return json.load(f)


def save_catalog(cat):
    # Xcode's own layout (" : ", two-space indent, sorted keys) so its edits diff cleanly.
    text = json.dumps(cat, ensure_ascii=False, indent=2, sort_keys=True, separators=(",", " : "))
    with open(CATALOG, "w", encoding="utf-8") as f:
        f.write(text + "\n")


def extracted_keys(stringsdata_dir):
    keys = {}
    paths = list(Path(stringsdata_dir).rglob("*.stringsdata"))
    for p in paths:
        try:
            data = json.loads(p.read_text(encoding="utf-8"))
        except (OSError, ValueError):
            continue
        for table, entries in data.get("tables", {}).items():
            if table != "Localizable":
                continue
            for e in entries:
                keys.setdefault(e["key"], set()).add(Path(data.get("source", p.name)).name)
    return keys, len(paths)


def specs(s):
    """The placeholders of a format string: [(position or None, kind)], %% ignored."""
    out = []
    for m in SPEC.finditer(s):
        kind = m.group(3)
        if kind == "%":
            continue
        kind = {"lld": "int", "ld": "int", "d": "int", "llu": "int", "lu": "int", "u": "int", "x": "int", "X": "int",
                "@": "obj", "s": "obj", "f": "dbl", "g": "dbl", "e": "dbl", "c": "chr"}[kind]
        out.append((int(m.group(1)) if m.group(1) else None, kind))
    return out


def placeholders_match(key, value):
    want = [k for _, k in specs(key)]
    got = specs(value)
    if not got:
        return not want
    if all(p is None for p, _ in got):
        return [k for _, k in got] == want
    if any(p is None for p, _ in got):
        return False  # mixing positional and sequential is undefined
    try:
        return sorted(got) == sorted((i + 1, want[i]) for i in range(len(want))) or \
            all(want[p - 1] == k for p, k in got) and len({p for p, _ in got}) == len(want)
    except IndexError:
        return False


def has_letters(s):
    return re.search(r"[A-Za-z가-힣]", SPEC.sub("", s)) is not None


def ko_values(entry):
    """Every Korean string of a catalog entry (plural variants included)."""
    ko = entry.get("localizations", {}).get("ko")
    if not ko:
        return None
    vals = []
    if "stringUnit" in ko:
        u = ko["stringUnit"]
        if u.get("state") not in ("translated",) or not u.get("value"):
            return None
        vals.append(u["value"])
    for var in ko.get("variations", {}).values():
        for v in var.values():
            u = v.get("stringUnit", {})
            if u.get("state") != "translated" or not u.get("value"):
                return None
            vals.append(u["value"])
    return vals or None


def problems_for(key, entry):
    vals = ko_values(entry or {})
    if vals is None:
        return "no Korean translation"
    for v in vals:
        if not placeholders_match(key, v):
            return f"placeholders differ from the key: {v!r}"
    # An English word left as is in Korean is only fine for names and units
    # (DBLN, GB, Touch ID, CSV): a key with an ordinary English word needs Hangul.
    if re.search(r"\b[a-z]{3,}", SPEC.sub("", key)) and not any(re.search(r"[가-힣]", v) for v in vals):
        return f"the Korean has no Korean in it: {vals[0]!r}"
    return None


def cmd_check(args):
    d = arg(args, "--stringsdata")
    keys, n = extracted_keys(d)
    if n == 0:
        print(f"check-wallet-l10n: no .stringsdata under {d} (SWIFT_EMIT_LOC_STRINGS off?)", file=sys.stderr)
        return 1
    cat = load_catalog()["strings"]
    bad = []
    for key in sorted(keys):
        if not has_letters(key):
            continue  # "%@ · %@", "—", "#%lld": nothing to translate
        why = "not in the catalog" if key not in cat else problems_for(key, cat[key])
        if why:
            bad.append((key, why, sorted(keys[key])))
    for key, why, files in bad:
        loc = ", ".join(files)
        print(f"error: {loc}: \"{key}\" — {why} (apps/wallet/Resources/Localizable.xcstrings)")
    if bad:
        print(f"check-wallet-l10n: {len(bad)} user-visible string(s) have no usable Korean. "
              "Run scripts/check-wallet-l10n.sh --sync, translate them, and build again.", file=sys.stderr)
        return 1
    print(f"check-wallet-l10n: {sum(1 for k in keys if has_letters(k))} strings, all with Korean")
    return 0


def cmd_sync(args):
    d = arg(args, "--stringsdata")
    keys, n = extracted_keys(d)
    if n == 0:
        print(f"no .stringsdata under {d}", file=sys.stderr)
        return 1
    cat = load_catalog()
    strings = cat["strings"]
    added = [k for k in keys if k not in strings]
    for k in added:
        strings[k] = {}
        if not has_letters(k):
            strings[k] = {"localizations": {"ko": {"stringUnit": {"state": "translated", "value": k}}}}
    stale = [k for k, e in strings.items() if k not in keys and e.get("extractionState") != "manual"]
    for k in stale:
        del strings[k]
    save_catalog(cat)
    print(f"sync: {len(keys)} keys extracted, {len(added)} added, {len(stale)} stale removed")
    return 0


def cmd_missing(_args):
    for k, e in sorted(load_catalog()["strings"].items()):
        if has_letters(k) and problems_for(k, e):
            print(json.dumps(k, ensure_ascii=False))
    return 0


# ---------- lint: English sentences that never reach the catalog ----------

def literals(code):
    """The string literals of one line of Swift, interpolations kept as written
    (nested literals inside an interpolation are part of it, not literals of
    their own). Multi-line literals are skipped."""
    out, i, n = [], 0, len(code)
    while i < n:
        c = code[i]
        if code.startswith('"""', i):
            return out
        if c == "/" and code.startswith("//", i):
            return out
        if c != '"':
            i += 1
            continue
        j, buf = i + 1, []
        while j < n and code[j] != '"':
            if code[j] == "\\" and j + 1 < n and code[j + 1] == "(":
                depth, k, in_str = 1, j + 2, False
                while k < n and depth:
                    ch = code[k]
                    if in_str:
                        if ch == "\\":
                            k += 1
                        elif ch == '"':
                            in_str = False
                    elif ch == '"':
                        in_str = True
                    elif ch == "(":
                        depth += 1
                    elif ch == ")":
                        depth -= 1
                    k += 1
                buf.append(code[j:k])
                j = k
                continue
            if code[j] == "\\":
                buf.append(code[j:j + 2])
                j += 2
                continue
            buf.append(code[j])
            j += 1
        out.append("".join(buf))
        i = j + 1
    return out


INTERP = re.compile(r"\\\((?:[^()]|\((?:[^()]|\([^()]*\))*\))*\)")
# Calls whose text is for logs, developers or other programs, never a screen.
SKIP_CALL = re.compile(r"\b(note|print|NSLog|debugPrint|fatalError|precondition|preconditionFailure|assert|assertionFailure|"
                       r"ProviderError|logger\.\w+|os_log|Logger|URL|URLRequest|setValue|appendingPathComponent|"
                       r"appending|UserDefaults|forKey|DispatchQueue|beginActivity|SleepGuard|IOPM\w*|contains|hasPrefix|"
                       r"hasSuffix|range|logEvent|NodeStatusLog\.\w+|\w+Log\.(?:notice|info|debug|error|fault|log))\s*\(|\bmessage:\s*\"|\blabel:\s*\"|privacy:")


def strip_interp(s):
    """Interpolations out, whatever their nesting."""
    out, i = [], 0
    while i < len(s):
        if s.startswith("\\(", i):
            depth, k, in_str = 1, i + 2, False
            while k < len(s) and depth:
                ch = s[k]
                if in_str:
                    if ch == "\\":
                        k += 1
                    elif ch == '"':
                        in_str = False
                elif ch == '"':
                    in_str = True
                elif ch == "(":
                    depth += 1
                elif ch == ")":
                    depth -= 1
                k += 1
            out.append("\u0001")
            i = k
            continue
        out.append(s[i])
        i += 1
    return "".join(out)


def normalize_key(k):
    return SPEC.sub(lambda m: "%" if m.group(3) == "%" else "\u0001", k).replace('\\"', '"')


def normalize_literal(s):
    return strip_interp(s).replace('\\"', '"').replace("\\n", "\n")


def englishy(s):
    t = strip_interp(s)
    if re.search(r"[가-힣]", t) or re.search(r"[;=|{}_]|--|\w/\w|^\w+:\w", t):
        return False
    words = re.findall(r"[A-Za-z][a-z']+", t)
    return " " in t.strip() and len(words) >= 2


def load_allow():
    allow_files, allow_text = set(), set()
    if ALLOW.exists():
        for line in ALLOW.read_text(encoding="utf-8").splitlines():
            line = line.split("#", 1)[0].strip()
            if line.startswith("file:"):
                allow_files.add(line[5:].strip())
            elif line.startswith("text:"):
                allow_text.add(line[5:].strip())
    return allow_files, allow_text


def cmd_lint(_args):
    allow_files, allow_text = load_allow()
    keys = {normalize_key(k) for k in load_catalog()["strings"]}
    bad = []
    for path in sorted(SOURCES.glob("*.swift")):
        if path.name in allow_files:
            continue
        in_block_comment = False
        for n, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
            code = line
            if in_block_comment:
                if "*/" not in code:
                    continue
                code = code.split("*/", 1)[1]
                in_block_comment = False
            if code.lstrip().startswith("//"):
                continue
            if "/*" in code and "*/" not in code:
                in_block_comment = True
                code = code.split("/*", 1)[0]
            if SKIP_CALL.search(code) or re.search(r"\bko\b|\bkorean\b|AppLanguage\.korean|HealthCheck\.korean", code):
                continue
            for s in literals(code):
                if not englishy(s) or normalize_literal(s) in keys or s in allow_text:
                    continue
                bad.append(f"{path.relative_to(ROOT)}:{n}: \"{s}\"")
    for b in bad:
        print(f"error: {b} — an English sentence that is not localized (wrap it in String(localized:) "
              f"or add it to scripts/wallet-l10n-allow.txt with the reason it is never shown)")
    if bad:
        print(f"lint-wallet-l10n: {len(bad)} unlocalized English sentence(s)", file=sys.stderr)
        return 1
    print("lint-wallet-l10n: no unlocalized English sentences")
    return 0


def arg(args, name):
    if name not in args or args.index(name) + 1 >= len(args):
        print(f"missing {name}", file=sys.stderr)
        sys.exit(2)
    return args[args.index(name) + 1]


if __name__ == "__main__":
    cmds = {"check": cmd_check, "sync": cmd_sync, "lint": cmd_lint, "missing": cmd_missing}
    if len(sys.argv) < 2 or sys.argv[1] not in cmds:
        print(__doc__)
        sys.exit(2)
    sys.exit(cmds[sys.argv[1]](sys.argv[2:]))
