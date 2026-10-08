#!/usr/bin/env python3
"""Check fixture screenshots and visible text for the wallet's complete language matrix.

wallet-screens.sh exports each PNG with a .text.json sidecar from Vision OCR.
Every renderer screen is required in en/ko/ja/zh-Hans/zh-Hant light, and en/ko
dark. No screen is skipped. Legal English is allowed only for catalog entries
marked legal-English on sheet-terms; its title/buttons still follow the locale.

Script detection cannot distinguish all Han-only Japanese/Chinese sentences.
Foreign-only catalog phrases catch that case, and catalog/compiler/source gates
verify that a selected key always has a translation in the selected locale.
"""
import argparse
import importlib.util
import json
import re
import sys
import unicodedata
from pathlib import Path

sys.dont_write_bytecode = True

ROOT = Path(__file__).resolve().parent.parent
CATALOG = ROOT / "apps/wallet/Resources/Localizable.xcstrings"
RENDERER = ROOT / "apps/wallet/Screens/WalletScreens.swift"
LANGUAGES = ("en", "ko", "ja", "zh-Hans", "zh-Hant")
HANGUL = re.compile(r"[\u1100-\u11ff\u3130-\u318f\uac00-\ud7af]")
KANA = re.compile(r"[\u3040-\u30ff\u31f0-\u31ff]")
HAN = re.compile(r"[\u3400-\u4dbf\u4e00-\u9fff\uf900-\ufaff\U00020000-\U0002ffff]")
LATIN = re.compile(r"[A-Za-z]+")
SPEC = re.compile(r"%(?:\d+\$)?[-+ #0]*\d*(?:\.\d+)?(?:lld|llu|ld|lu|@|d|u|f|g|e|s|c|x|X|%)")

# Brand/protocol names, fixture ticker symbols and measurements, not ordinary
# English UI words. The declaration is explicit so a new exception is reviewed.
NAMES = ("EastSea", "Doubloon", "Aether", "DBLN", "Mac", "Touch ID", "Face ID", "Secure Enclave", "Apple",
         "DeviceCheck", "Pipln", "Sparkle", "Safari", "WebKit", "Ledger", "Trezor", "Samsung T7",
         "Finder", "FileVault", "iCloud", "iPhone", "macOS", "iOS", "Claude Code", "Codex", "Metal",
         "AI", "SSD", "BLS", "EIP-7864", "SHA-256", "ID", "PATH", "APFS", "Mac OS Extended", "OS",
         "http", "https", "eastsea-earnings.csv",
         "CSV", "GPU", "CPU", "RAM", "API", "RPC", "DHT", "EVM", "ERC-20", "HTTP", "HTTPS",
         "TCP", "UDP", "IP", "PID", "JSON", "ZIP", "USDX", "VVDBLN", "NEB", "ORB", "CMT", "WAETH", "AETH")
NAME_RE = re.compile(r"(?<![A-Za-z])(?:" + "|".join(re.escape(name) for name in sorted(NAMES, key=len, reverse=True)) + r")(?![A-Za-z])")
UNIT_RE = re.compile(r"(?<![A-Za-z])(?:[KMGT]i?B|[km]?s|Hz|GHz|MHz|MB/s|GB/s|UTC|W|kWh)(?![A-Za-z])")
DATA_RE = re.compile(r"(?:https?://[^\s]+|(?:[A-Za-z0-9-]+\.)+(?:xyz|com|org|net)(?:/[^\s]*)?"
                     r"|0x[0-9a-fA-F…\.]*|(?<!\w)[0-9a-fA-F]{8,}(?!\w)"
                     r"|~?/\.local/bin|(?:~?/Applications/|~?/Library/Application Support/)(?:EastSea|Aether)(?:\.app|/[^\s]*)?)")
# Human supplied data is not app copy. Exceptions are confined to the screens
# that actually show these exact fixtures (DesignPreview / WalletScreens).
TOKEN_NAME_SCREENS = {"home", "home-empty", "home-paused", "home-verifying", "home-alerts", "home-narrow",
                      "window", "window-network", "window-narrow", "two-accounts", "sheet-assets", "sheet-send-token", "developer"}
TOKEN_NAMES = ("Test Nebula", "Test Orbit", "Test Comet", "Test Dollar", "Doubloon Cash", "Wrapped AETH")
MEMOS = {"sheet-send-link": ("Coffee beans · order 1042",), "sheet-call": ("Swap on the EastSea DEX",)}
# The legacy-app alert displays this path as data, not instructions.
FIXTURE_DATA = {"alert-legacy-aether": ("/Applications/Aether.app",),
                "developer": ("Shop", "localhost", "127.0.0.1")}
NATIVE_NAMES = {"en": "English", "ko": "한국어", "ja": "日本語", "zh-Hans": "简体中文", "zh-Hant": "繁體中文"}
LEGAL_SCREENS = {"sheet-terms"}
# These are the nonsecret address fixtures DesignPreview places in each real
# AccountStore. QR payloads come from Vision decoding the exact saved pixels.
RECEIVE_ADDRESSES = {"sheet-receive": "0x5397a1c0de4b1b8f6a3cb2d1e0f9c7a6b5d4e502",
                     "menubar-qr": "0x71b4000000000000000000000000000000002a2b"}
FEATURE_COPY = {
    "switcher": ("Accounts", "Create account", "A small one-time fee on first use"),
    "retire-blocked": ("Retire account", "Check balance again",
                       "This account still has funds. Move its balance and tokens before retiring it so you can keep using them."),
    "menubar-qr": ("Receive", "Copy address", "Share"),
}
FEATURE_ACCOUNTS = {"switcher": (1, 2), "two-accounts": (2,), "retire-blocked": (2,), "menubar-qr": (2,)}


def normalize(value):
    return "".join(character.casefold() for character in unicodedata.normalize("NFKC", value)
                   if character.isalnum())


def mask_allowed(text, screen, language):
    # Strip exact full phrases before component names (Wrapped AETH, EastSea DEX).
    phrases = list(MEMOS.get(screen, ())) + list(FIXTURE_DATA.get(screen, ()))
    if screen in TOKEN_NAME_SCREENS:
        phrases += list(TOKEN_NAMES)
    if screen.startswith("settings"):
        phrases.append(NATIVE_NAMES[language])
    for phrase in sorted(phrases, key=len, reverse=True):
        text = text.replace(phrase, "")
    text = re.sub(r"[⇧⌘⌥⌃]+[A-Za-z]", "", text)
    text = DATA_RE.sub("", text)
    text = NAME_RE.sub("", text)
    return UNIT_RE.sub("", text)


def units(entry, language):
    local = entry.get("localizations", {}).get(language)
    if local is None:
        return []
    if "stringUnit" in local:
        return [local["stringUnit"].get("value", "")]
    values = []
    def walk(value):
        if isinstance(value, dict):
            if "stringUnit" in value:
                values.append(value["stringUnit"].get("value", ""))
            else:
                for child in value.values():
                    walk(child)
    walk(local)
    return values


def legal_corpus(catalog):
    return [key for key, entry in catalog["strings"].items() if "legal-English" in entry.get("comment", "")]


def legal_allowed(text, screen, language, legal):
    if screen not in LEGAL_SCREENS or language not in ("ja", "zh-Hans", "zh-Hant"):
        return False
    fragment = normalize(re.sub(r"\d+", "", mask_allowed(text, screen, language)))
    # Only a fragment of an actual English legal body qualifies; no screen,
    # heading, button or unrestricted English exemption exists.
    if len(fragment) < 4:
        return False
    for sentence in legal:
        body = normalize(re.sub(r"\d+", "", mask_allowed(SPEC.sub("", sentence), screen, language)))
        if fragment in body:
            return True
    return False


def foreign_phrases(catalog, language):
    own_values = [value for entry in catalog["strings"].values() for value in units(entry, language)]
    own = "\n".join(normalize(SPEC.sub("", value)) for value in own_values)
    foreign = set()
    for other in ("ja", "zh-Hans", "zh-Hant"):
        if other == language:
            continue
        for entry in catalog["strings"].values():
            for value in units(entry, other):
                # Distinguish Han-only phrases; kana/Hangul are checked directly.
                for phrase in re.split(r"[。！？.!?;:\n]|%(?:\d+\$)?(?:lld|llu|ld|lu|@|d|u|f|s)", value):
                    phrase = normalize(phrase)
                    if (len(HAN.findall(phrase)) >= 2 and not KANA.search(phrase)
                            and not HANGUL.search(phrase) and phrase not in own):
                        foreign.add(phrase)
    return sorted(foreign, key=len, reverse=True)


def text_problems(text, screen, language, legal, foreign):
    remainder = mask_allowed(text, screen, language)
    if legal_allowed(text, screen, language, legal):
        return []
    problems = []
    if language == "en":
        if HANGUL.search(remainder) or KANA.search(remainder) or HAN.search(remainder):
            problems.append("non-English script")
    elif language == "ko":
        if KANA.search(remainder) or HAN.search(remainder):
            problems.append("Japanese/Chinese script in Korean")
        if LATIN.search(remainder):
            problems.append("English words in Korean")
    elif language == "ja":
        if HANGUL.search(remainder):
            problems.append("Korean script in Japanese")
        if LATIN.search(remainder):
            problems.append("English words in Japanese")
    else:
        if HANGUL.search(remainder) or KANA.search(remainder):
            problems.append("Korean/Japanese script in Chinese")
        if LATIN.search(remainder):
            problems.append("English words in Chinese")
    normalized = normalize(remainder)
    if language in ("ja", "zh-Hans", "zh-Hant"):
        match = next((phrase for phrase in foreign if phrase in normalized), None)
        if match:
            problems.append(f"foreign-only catalog phrase {match!r}")
    return problems


def catalog_script_problems(catalog):
    """The catalog uses the same explicit name/unit rules as rendered text.

    Legal-only entries retain English verbatim. Reusable legal-on-terms entries
    still require real translations because nonlegal screens use them too.
    """
    problems = []
    for key, entry in sorted(catalog["strings"].items()):
        for language in LANGUAGES:
            values = units(entry, language) or ([key] if language == "en" else [])
            for value in values:
                if entry.get("comment", "").startswith("legal-English:") and language in ("ja", "zh-Hans", "zh-Hant"):
                    if value != key:
                        problems.append(f"{key!r}: {language} legal body must retain the reviewed English")
                    continue
                reasons = text_problems(SPEC.sub("", value), "catalog", language, [], [])
                if reasons:
                    problems.append(f"{key!r}: {language} {'; '.join(reasons)}: {value!r}")
    return problems


def renderer_screens(path):
    text = path.read_text(encoding="utf-8")
    return set(re.findall(r'\b(?:page|stagePage|window|alert)\(\s*"([^"\n]+)"', text))


def feature_problems(payload, screen, language, catalog):
    visible = [line.get("text", "") if isinstance(line, dict) else line for line in payload.get("lines", [])]
    joined = normalize(" ".join(text for text in visible if isinstance(text, str)))
    problems = []
    for key in FEATURE_COPY.get(screen, ()):
        translated = units(catalog["strings"].get(key, {}), language) or ([key] if language == "en" else [])
        if not translated or normalize(translated[0]) not in joined:
            problems.append(f"missing visible translated {key!r}")
    for account_id in FEATURE_ACCOUNTS.get(screen, ()):
        key = "Account %lld"
        translated = units(catalog["strings"].get(key, {}), language) or ([key] if language == "en" else [])
        labels = [re.sub(r"%(?:\d+\$)?lld", str(account_id), value) for value in translated]
        if not labels or not any(normalize(label) in joined for label in labels):
            problems.append(f"missing visible selected/listed account {account_id}")
    if screen in RECEIVE_ADDRESSES:
        observed = payload.get("qrPayloads", [])
        if observed != [RECEIVE_ADDRESSES[screen]]:
            problems.append("receive QR does not decode to the fixture's selected account address")
    return problems


def run(args):
    catalog = json.loads(args.catalog.read_text(encoding="utf-8"))
    problems = []
    # Reuse the exact catalog/source gate that the build uses, without invoking
    # a compiler or allowing screenshot-only QA to bypass missing translations.
    spec = importlib.util.spec_from_file_location("wallet_l10n", ROOT / "scripts/wallet-l10n.py")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    problems += module.catalog_problems(catalog)
    if not args.renderer_fixture:
        problems += module.source_key_problems(catalog)
    screens = renderer_screens(args.renderer)
    if args.only:
        screens = {screen for screen in screens if screen.startswith(args.only)}
    if not screens:
        problems.append("renderer has no matching screens")
    legal = legal_corpus(catalog)
    foreign = {language: foreign_phrases(catalog, language) for language in LANGUAGES}
    checked, lines_checked = 0, 0
    for screen in sorted(screens):
        for language in LANGUAGES:
            modes = ("light", "dark") if language in ("en", "ko") else ("light",)
            for mode in modes:
                stem = f"{screen}-{language}-{mode}"
                png, sidecar = args.out / f"{stem}.png", args.out / f"{stem}.text.json"
                if not png.is_file():
                    problems.append(f"{stem}: missing PNG")
                    continue
                if png.read_bytes()[:8] != b"\x89PNG\r\n\x1a\n":
                    problems.append(f"{stem}: invalid PNG")
                if not sidecar.is_file():
                    problems.append(f"{stem}: missing visible-text sidecar")
                    continue
                try:
                    payload = json.loads(sidecar.read_text(encoding="utf-8"))
                except (OSError, ValueError) as exc:
                    problems.append(f"{stem}: invalid text sidecar: {exc}")
                    continue
                if (payload.get("screen"), payload.get("language"), payload.get("appearance")) != (screen, language, mode):
                    problems.append(f"{stem}: text metadata does not match screenshot")
                lines = payload.get("lines", [])
                if not lines:
                    problems.append(f"{stem}: no visible text was extracted")
                checked += 1
                for line in lines:
                    text = line.get("text", "") if isinstance(line, dict) else line
                    if not isinstance(text, str) or not text.strip():
                        problems.append(f"{stem}: empty or invalid text observation")
                        continue
                    lines_checked += 1
                    reasons = text_problems(text, screen, language, legal, foreign[language])
                    if reasons:
                        problems.append(f"{stem}: {'; '.join(reasons)}: {text!r}")
                problems += [f"{stem}: {problem}" for problem in feature_problems(payload, screen, language, catalog)]
                if screen == "sheet-terms" and language in ("ja", "zh-Hans", "zh-Hant"):
                    notice = catalog["strings"].get("This translation is for reference; the English text governs.", {})
                    values = units(notice, language)
                    joined = normalize(" ".join(line.get("text", "") if isinstance(line, dict) else line for line in lines))
                    if not values or normalize(values[0]) not in joined:
                        problems.append(f"{stem}: missing translated English-governs legal notice")
    for problem in problems:
        print(f"error: wallet-screens-language: {problem}")
    if problems:
        print(f"wallet-screens-language: FAIL ({len(problems)} problems; {checked} screens checked)", file=sys.stderr)
        return 1
    print(f"wallet-screens-language: OK {checked} screens ({len(screens)} views × 7 language/appearance pairs), "
          f"{lines_checked} visible text lines; catalog en/ko/ja/zh-Hans/zh-Hant complete")
    return 0


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, default=ROOT / "tmp/screens")
    parser.add_argument("--only", default="")
    parser.add_argument("--catalog", type=Path, default=CATALOG)
    parser.add_argument("--renderer", type=Path, default=RENDERER)
    parser.add_argument("--renderer-fixture", action="store_true", help=argparse.SUPPRESS)
    args = parser.parse_args()
    if args.renderer_fixture:
        # Only parser/detector unit fixtures may omit the production source scan.
        try:
            args.renderer.resolve().relative_to(ROOT / "tmp")
            args.catalog.resolve().relative_to(ROOT / "tmp")
            args.out.resolve().relative_to(ROOT / "tmp")
        except ValueError:
            parser.error("renderer fixtures must be entirely under this workspace's tmp")
    try:
        return run(args)
    except (OSError, ValueError, KeyError) as exc:
        print(f"error: wallet-screens-language: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
