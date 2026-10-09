#!/usr/bin/env python3
"""Validate the wallet's English-base String Catalog and every supported language.

check [--stringsdata DIR]   catalog coverage, Swift localized keys, compiler keys
lint                       sentences bypassing localization and old language switches
sync --stringsdata DIR     add extracted keys (preserve all existing translations)
missing                    list absent or unusable translations by language
prepare-tests --out DIR    generate locale bundles FROM the catalog under root/tmp
self-test                  parser, placeholder and missing-translation regressions

The Swift scanner understands comments, raw/multiline literals and nested
interpolations. The compiler's stringsdata supplies the definitive format types.
"""
import importlib.util
import json
import plistlib
import re
import sys
from dataclasses import dataclass
from pathlib import Path

sys.dont_write_bytecode = True

ROOT = Path(__file__).resolve().parent.parent
WALLET = ROOT / "apps" / "wallet"
CATALOG = WALLET / "Resources" / "Localizable.xcstrings"
SOURCES = WALLET / "Sources"
ALLOW = ROOT / "scripts" / "wallet-l10n-allow.txt"
LANGUAGES = ("en", "ko", "ja", "zh-Hans", "zh-Hant")
SPEC = re.compile(r"%(?:(\d+)\$)?([-+ #0]*\d*(?:\.\d+)?)(lld|llu|ld|lu|@|d|u|f|g|e|s|c|x|X|%)")


def option(args, name, default=None):
    if name not in args:
        return default
    i = args.index(name)
    if i + 1 == len(args):
        raise ValueError(f"missing value for {name}")
    return args[i + 1]


def load_catalog(path=CATALOG):
    return json.loads(Path(path).read_text(encoding="utf-8"))


def save_catalog(cat):
    # Match Xcode's own JSON style to keep catalog edits reviewable.
    CATALOG.write_text(json.dumps(cat, ensure_ascii=False, indent=2, sort_keys=True,
                                  separators=(",", " : ")) + "\n", encoding="utf-8")


def extracted_keys(directory):
    keys, bad = {}, []
    paths = sorted(Path(directory).rglob("*.stringsdata"))
    for path in paths:
        try:
            data = json.loads(path.read_text(encoding="utf-8"))
        except (OSError, ValueError) as exc:
            bad.append(f"cannot read {path}: {exc}")
            continue
        for table, entries in data.get("tables", {}).items():
            if table == "Localizable":
                for entry in entries:
                    keys.setdefault(entry["key"], set()).add(Path(data.get("source", path.name)).name)
    return keys, len(paths), bad


def specs(value):
    """Printf arguments, with explicit positions retained; %% is not an argument."""
    kinds = {"lld": "int", "ld": "int", "d": "int", "llu": "int", "lu": "int", "u": "int",
             "x": "int", "X": "int", "@": "obj", "s": "obj", "f": "dbl", "g": "dbl",
             "e": "dbl", "c": "chr"}
    return [(int(m.group(1)) if m.group(1) else None, kinds[m.group(3)])
            for m in SPEC.finditer(value) if m.group(3) != "%"]


def placeholders_match(key, value):
    want = [kind for _, kind in specs(key)]
    got = specs(value)
    if all(position is None for position, _ in got):
        return [kind for _, kind in got] == want
    if any(position is None for position, _ in got):
        return False
    if any(position < 1 or position > len(want) for position, _ in got):
        return False
    return (all(want[position - 1] == kind for position, kind in got)
            and {position for position, _ in got} == set(range(1, len(want) + 1)))


def string_units(localization):
    """Catalog leaves, including plural/device/substitution variations."""
    if not isinstance(localization, dict):
        return []
    if "stringUnit" in localization:
        return [localization["stringUnit"]]
    result = []
    for name, value in localization.items():
        if isinstance(value, dict):
            result.extend(string_units(value))
    return result


def values_for(key, entry, language):
    local = entry.get("localizations", {}).get(language)
    if local is None and language == "en":
        return [key]  # English is the key language, including empty/punctuation keys.
    units = string_units(local)
    if not units or any(unit.get("state") != "translated" or "value" not in unit
                        or (key and not unit["value"]) for unit in units):
        return None
    return [unit["value"] for unit in units]


def language_rules():
    name = "wallet_language_rules"
    if name not in sys.modules:
        spec = importlib.util.spec_from_file_location(name, ROOT / "scripts/check-wallet-screens-language.py")
        module = importlib.util.module_from_spec(spec)
        sys.modules[name] = module
        spec.loader.exec_module(module)
    return sys.modules[name]


def catalog_problems(cat):
    problems = []
    if cat.get("sourceLanguage") != "en":
        problems.append("sourceLanguage must be en")
    for key, entry in sorted(cat["strings"].items()):
        for language in LANGUAGES:
            values = values_for(key, entry, language)
            if values is None:
                problems.append(f"{key!r}: no {language} translation")
            elif language != "en":
                for value in values:
                    if not placeholders_match(key, value):
                        problems.append(f"{key!r}: {language} placeholders differ: {value!r}")
    return problems + language_rules().catalog_script_problems(cat)


@dataclass
class SwiftLiteral:
    start: int
    end: int
    value: str
    line: int


def literal_open(code, start):
    i = start
    while i < len(code) and code[i] == "#":
        i += 1
    if i < len(code) and code[i] == '"':
        width = 3 if code.startswith('"""', i) else 1
        return i - start, width, i + width
    return None


def skip_comment(code, start):
    if code.startswith("//", start):
        end = code.find("\n", start)
        return len(code) if end < 0 else end
    if code.startswith("/*", start):
        depth, i = 1, start + 2
        while i < len(code) and depth:
            if code.startswith("/*", i):
                depth += 1
                i += 2
            elif code.startswith("*/", i):
                depth -= 1
                i += 2
            else:
                i += 1
        return i
    return start


def skip_expression(code, start):
    """Skip a Swift interpolation, including nested strings and comments."""
    depth, i = 1, start
    while i < len(code) and depth:
        end = skip_comment(code, i)
        if end != i:
            i = end
            continue
        opening = literal_open(code, i)
        if opening:
            i = read_literal(code, i)[0].end
        elif code[i] == "(":
            depth += 1
            i += 1
        elif code[i] == ")":
            depth -= 1
            i += 1
        else:
            i += 1
    return i


def read_literal(code, start):
    hashes, width, i = literal_open(code, start)
    close = '"' * width + "#" * hashes
    escape = "\\" + "#" * hashes
    result, nested = [], []
    while i < len(code) and not code.startswith(close, i):
        if code.startswith(escape + "(", i):
            begin = i + len(escape) + 1
            end = skip_expression(code, begin)
            nested.extend(swift_literals(code[begin:end - 1], offset=begin, full_code=code))
            result.append("\u0001")  # type supplied by stringsdata, not guessed from the expression
            i = end
        elif code.startswith(escape, i):
            j = i + len(escape)
            if j == len(code):
                break
            char = code[j]
            if char == "u" and j + 1 < len(code) and code[j + 1] == "{":
                end = code.find("}", j + 2)
                if end >= 0:
                    try:
                        result.append(chr(int(code[j + 2:end], 16)))
                    except ValueError:
                        result.append(code[i:end + 1])
                    i = end + 1
                    continue
            result.append({"n": "\n", "r": "\r", "t": "\t", "0": "\0",
                           '"': '"', "'": "'", "\\": "\\"}.get(char, char))
            i = j + 1
        else:
            result.append(code[i])
            i += 1
    value = "".join(result)
    if width == 3:
        # Swift drops the opening newline and the closing delimiter's indent.
        value = value.removeprefix("\n")
        indent = re.search(r"\n([ \t]*)$", value)
        if indent:
            padding = indent.group(1)
            value = value[:indent.start()]
            if padding:
                value = "\n".join(line[len(padding):] if line.startswith(padding) else line
                                   for line in value.split("\n"))
    return SwiftLiteral(start, i + len(close), value, code.count("\n", 0, start) + 1), nested


def swift_literals(code, offset=0, full_code=None):
    """All Swift literals, including literals inside interpolation expressions."""
    result, i = [], 0
    while i < len(code):
        end = skip_comment(code, i)
        if end != i:
            i = end
        elif literal_open(code, i):
            literal, nested = read_literal(code, i)
            if offset:
                literal = SwiftLiteral(literal.start + offset, literal.end + offset, literal.value,
                                       full_code.count("\n", 0, literal.start + offset) + 1)
                nested = [SwiftLiteral(n.start + offset, n.end + offset, n.value,
                                       full_code.count("\n", 0, n.start + offset) + 1) for n in nested]
            result.extend([literal, *nested])
            i = literal.end - offset
        else:
            i += 1
    return result


def uncomment(code):
    """Mask comments, preserving offsets and source line numbers."""
    chars, i = list(code), 0
    while i < len(code):
        end = skip_comment(code, i)
        if end != i:
            for j in range(i, end):
                if chars[j] != "\n":
                    chars[j] = " "
            i = end
        elif literal_open(code, i):
            i = read_literal(code, i)[0].end
        else:
            i += 1
    return "".join(chars)


def normalize_key(key):
    return SPEC.sub(lambda m: "%" if m.group(3) == "%" else "\u0001", key)


LOCALIZED_CALL = re.compile(r"(?:\bString\s*\(\s*localized\s*:|\bLocalizedStringResource\s*\()\s*$")


def localized_keys(path):
    code = uncomment(path.read_text(encoding="utf-8"))
    return [(literal.value, literal.line) for literal in swift_literals(code)
            if LOCALIZED_CALL.search(code[:literal.start])]


def source_key_problems(cat, sources=SOURCES):
    known = {normalize_key(key) for key in cat["strings"]}
    problems = []
    for path in sorted(Path(sources).rglob("*.swift")):
        for key, line in localized_keys(path):
            if key not in known:
                problems.append(f"{path.name}:{line}: localized key {key!r} is not in the catalog")
    return problems


def cmd_check(args):
    cat = load_catalog(option(args, "--catalog", CATALOG))
    problems = catalog_problems(cat) + source_key_problems(cat, option(args, "--sources", SOURCES))
    directory = option(args, "--stringsdata")
    extracted = 0
    if directory:
        keys, count, failures = extracted_keys(directory)
        problems.extend(failures)
        if not count:
            problems.append(f"no .stringsdata under {directory} (SWIFT_EMIT_LOC_STRINGS off?)")
        extracted = len(keys)
        for key, files in sorted(keys.items()):
            if key not in cat["strings"]:
                problems.append(f"{', '.join(sorted(files))}: {key!r} is not in the catalog")
    for problem in problems:
        print(f"error: wallet-l10n: {problem}")
    if problems:
        print(f"check-wallet-l10n: FAIL ({len(problems)} problems)", file=sys.stderr)
        return 1
    counts = ", ".join(f"{language}={len(cat['strings'])}" for language in LANGUAGES)
    suffix = f"; {extracted} compiler keys" if directory else ""
    print(f"check-wallet-l10n: OK {counts}{suffix}")
    return 0


def cmd_sync(args):
    directory = option(args, "--stringsdata")
    if not directory:
        raise ValueError("sync requires --stringsdata DIR")
    keys, count, problems = extracted_keys(directory)
    if not count or problems:
        raise ValueError("no usable compiler stringsdata: " + "; ".join(problems))
    cat = load_catalog()
    added = [key for key in keys if key not in cat["strings"]]
    for key in added:
        cat["strings"][key] = {}
    # A compiler pass can omit pure-logic/manual resources or another target's
    # keys. Never destroy reviewed translations from a partial extraction.
    save_catalog(cat)
    print(f"sync: {len(keys)} compiler keys, {len(added)} added, existing translations preserved")
    return 0


def cmd_missing(args):
    for problem in catalog_problems(load_catalog(option(args, "--catalog", CATALOG))):
        print(problem)
    return 0


# English sentence lint retains documented machine-data exceptions, while
# localized-key coverage and retired language switches are ALWAYS checked.
SKIP_CALL = re.compile(r"\b(note|print|NSLog|debugPrint|fatalError|precondition|preconditionFailure|assert|assertionFailure|"
                       r"ProviderError|logger\.\w+|os_log|Logger|URL|URLRequest|setValue|appendingPathComponent|"
                       r"appending|UserDefaults|forKey|DispatchQueue|beginActivity|SleepGuard|IOPM\w*|contains|hasPrefix|"
                       r"hasSuffix|range|logEvent|NodeStatusLog\.\w+|\w+Log\.(?:notice|info|debug|error|fault|log))\s*\(|"
                       r"\bmessage:\s*\"|\blabel:\s*\"|privacy:")
OLD_LANGUAGE_SWITCH = re.compile(r"\b(?:HealthCheck|AppLanguage)\.korean\b|\b(?:ko|korean)\s*:\s*Bool\b")


def load_allow():
    files, texts = set(), set()
    if ALLOW.exists():
        for line in ALLOW.read_text(encoding="utf-8").splitlines():
            line = line.split("#", 1)[0].strip()
            if line.startswith("file:"):
                files.add(line[5:].strip())
            elif line.startswith("text:"):
                # The historical allowlist spells interpolations, unlike lexer values.
                value = line[5:].strip()
                texts.add(swift_literals('"' + value + '"')[0].value)
    return files, texts


def englishy(value):
    if re.search(r"[가-힣]|[;=|{}_]|--|\w/\w|^\w+:\w", value):
        return False
    return " " in value.strip() and len(re.findall(r"[A-Za-z][a-z']+", value)) >= 2


def cmd_lint(args):
    allow_files, allow_text = load_allow()
    cat = load_catalog(option(args, "--catalog", CATALOG))
    sources = Path(option(args, "--sources", SOURCES))
    known = {normalize_key(key) for key in cat["strings"]}
    problems = source_key_problems(cat, sources)
    for path in sorted(sources.rglob("*.swift")):
        code = uncomment(path.read_text(encoding="utf-8"))
        # Do not match names inside comments or string data.
        masked = list(code)
        for literal in swift_literals(code):
            masked[literal.start:literal.end] = " " * (literal.end - literal.start)
        for match in OLD_LANGUAGE_SWITCH.finditer("".join(masked)):
            problems.append(f"{path.name}:{code.count(chr(10), 0, match.start()) + 1}: retired boolean language switch")
        if path.name in allow_files:
            continue
        for literal in swift_literals(code):
            line = code.splitlines()[literal.line - 1]
            if (englishy(literal.value) and literal.value not in known and literal.value not in allow_text
                    and not SKIP_CALL.search(line)):
                problems.append(f"{path.name}:{literal.line}: {literal.value!r} is an unlocalized English sentence")
    for problem in problems:
        print(f"error: wallet-l10n: {problem}")
    if problems:
        print(f"lint-wallet-l10n: FAIL ({len(problems)} problems)", file=sys.stderr)
        return 1
    print("lint-wallet-l10n: OK no unlocalized English sentences or boolean language switches")
    return 0


def strings_quote(value):
    # JSON's escaped string syntax is also valid in an Apple .strings file.
    return json.dumps(value, ensure_ascii=False)


def cmd_prepare_tests(args):
    destination = Path(option(args, "--out", ROOT / "tmp" / "wallet-languages" / "WalletLocalizations.bundle")).resolve()
    try:
        destination.relative_to(ROOT / "tmp")
    except ValueError:
        raise ValueError("test localization resources must be under this workspace's tmp directory")
    cat = load_catalog(option(args, "--catalog", CATALOG))
    destination.mkdir(parents=True, exist_ok=True)
    info = {"CFBundleIdentifier": "com.pipln.eastsea.localization-tests", "CFBundleDevelopmentRegion": "en",
            "CFBundleLocalizations": list(LANGUAGES), "CFBundlePackageType": "BNDL"}
    (destination / "Info.plist").write_bytes(plistlib.dumps(info))
    for language in LANGUAGES:
        directory = destination / f"{language}.lproj"
        directory.mkdir(exist_ok=True)
        lines, plurals = [], {}
        for key, entry in sorted(cat["strings"].items()):
            values = values_for(key, entry, language)
            if values is None:
                # Tests must see a missing translation as a fallback, not a fabricated value.
                continue
            lines.append(f"{strings_quote(key)} = {strings_quote(values[0])};")
            variation = entry.get("localizations", {}).get(language, {}).get("variations", {}).get("plural")
            if variation:
                count = next((m for m in SPEC.finditer(key) if m.group(3) in ("lld", "llu", "ld", "lu", "d", "u")), None)
                if count:
                    rules = {"NSStringFormatSpecTypeKey": "NSStringPluralRuleType",
                             "NSStringFormatValueTypeKey": count.group(3)}
                    rules.update({category: leaf["stringUnit"]["value"] for category, leaf in variation.items()})
                    plurals[key] = {"NSStringLocalizedFormatKey": "%#@count@", "count": rules}
        (directory / "Localizable.strings").write_text("\n".join(lines) + "\n", encoding="utf-8")
        (directory / "Localizable.stringsdict").write_bytes(plistlib.dumps(plurals))
    print(f"wallet-test-localizations: {len(cat['strings'])} catalog keys in {destination}")
    return 0


def cmd_self_test(_args):
    code = r'''// String(localized: "Comment only")
let a = String(localized: "Ready \(formatter.call("data", inner(2))) now")
let b = String(localized: #"Raw \#(count) and "quotes""#)
let c = String(localized: """
    First line
    Second line
    """)
/* nested /* String(localized: "No") */ comment */
let d = "Outer \(String(localized: "Inner key"))"
'''
    pairs = [(literal.value, literal.line) for literal in swift_literals(uncomment(code))
             if LOCALIZED_CALL.search(uncomment(code)[:literal.start])]
    assert pairs == [("Ready \u0001 now", 2), ('Raw \u0001 and "quotes"', 3),
                     ("First line\nSecond line", 4), ("Inner key", 9)], pairs
    assert placeholders_match("%lld %@", "%2$@ %1$lld")
    assert not placeholders_match("%lld %@", "%0$lld %2$@")
    assert not placeholders_match("%lld %@", "%@ %lld")
    entry = {"localizations": {language: {"stringUnit": {"state": "translated", "value": value}}
                               for language, value in {"en": "OK", "ko": "확인", "ja": "確認", "zh-Hans": "好"}.items()}}
    bad = catalog_problems({"sourceLanguage": "en", "strings": {"OK": entry}})
    assert bad == ["'OK': no zh-Hant translation"], bad
    entry["localizations"]["zh-Hant"] = {"stringUnit": {"state": "translated", "value": "好"}}
    assert not catalog_problems({"sourceLanguage": "en", "strings": {"OK": entry}})
    print("wallet-l10n self-test: OK Swift literals, interpolation keys, placeholders, missing zh-Hant")
    return 0


if __name__ == "__main__":
    commands = {"check": cmd_check, "sync": cmd_sync, "lint": cmd_lint, "missing": cmd_missing,
                "prepare-tests": cmd_prepare_tests, "self-test": cmd_self_test}
    if len(sys.argv) < 2 or sys.argv[1] not in commands:
        print(__doc__)
        sys.exit(2)
    try:
        sys.exit(commands[sys.argv[1]](sys.argv[2:]))
    except (OSError, ValueError, KeyError) as exc:
        print(f"error: wallet-l10n: {exc}", file=sys.stderr)
        sys.exit(1)
