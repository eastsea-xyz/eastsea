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
import difflib
import importlib.util
import json
import re
import struct
import sys
import unicodedata
from pathlib import Path

sys.dont_write_bytecode = True

ROOT = Path(__file__).resolve().parent.parent
CATALOG = ROOT / "apps/wallet/Resources/Localizable.xcstrings"
RENDERER = ROOT / "apps/wallet/Screens/WalletScreens.swift"
LANGUAGES = ("en", "ko", "ja", "zh-Hans", "zh-Hant")
HANGUL = re.compile(r"[\u1100-\u11ff\u3130-\u318f\uac00-\ud7af]")
# Unicode's kana blocks also contain middle dots and dash-like marks that
# Vision emits for separators in Chinese/Latin text. Detect letters, not the
# entire block; actual Japanese words still contain these letter ranges.
KANA = re.compile(r"[\u3041-\u3096\u309d-\u309f\u30a1-\u30fa\u30fd-\u30ff\u31f0-\u31ff]")
HAN = re.compile(r"[\u3400-\u4dbf\u4e00-\u9fff\uf900-\ufaff\U00020000-\U0002ffff]")
LATIN = re.compile(r"[A-Za-z]+")
SPEC = re.compile(r"%(?:\d+\$)?[-+ #0]*\d*(?:\.\d+)?(?:lld|llu|ld|lu|@|d|u|f|g|e|s|c|x|X|%)")

# Brand/protocol names, fixture ticker symbols and measurements, not ordinary
# English UI words. The declaration is explicit so a new exception is reviewed.
NAMES = ("EastSea", "Doubloon", "Aether", "DBLN", "Mac", "Touch ID", "Face ID", "Secure Enclave", "Apple",
         "DeviceCheck", "Pipln", "Sparkle", "Cloudflare", "GitHub", "Safari", "WebKit", "Ledger", "Trezor", "Samsung T7",
         "Finder", "FileVault", "iCloud", "iPhone", "macOS", "iOS", "Claude Code", "Codex", "Metal",
         "AI", "SSD", "BLS", "EIP-7864", "SHA-256", "ID", "PATH", "APFS", "Mac OS Extended", "OS",
         "http", "https", "sea", "DuckDuckGo", "Google", "Bing", "Naver", "Brave", "eastsea-earnings.csv",
         "CSV", "GPU", "CPU", "RAM", "API", "RPC", "DHT", "EVM", "ERC-20", "HTTP", "HTTPS",
         "TCP", "UDP", "IP", "PID", "JSON", "ZIP", "USDX", "VVDBLN", "NEB", "ORB", "CMT", "WAETH", "AETH")
NAME_RE = re.compile(r"(?<![A-Za-z])(?:" + "|".join(re.escape(name) for name in sorted(NAMES, key=len, reverse=True)) + r")(?![A-Za-z])", re.IGNORECASE)
UNIT_RE = re.compile(r"(?<![A-Za-z])(?:[KMGT]i?B|[km]?s|Hz|GHz|MHz|MB/s|GB/s|UTC|W|kWh)(?![A-Za-z])")
DATA_RE = re.compile(r"(?:(?:https?|sea|eastsea-page)://[^\s]+|(?:[A-Za-z0-9-]+\.)+(?:xyz|com|org|net|sea)(?:/[^\s]*)?"
                     r"|0x[0-9a-fA-F…\.]*|(?<!\w)[0-9a-fA-F]{8,}(?!\w)"
                     r"|~?/\.local/bin|(?:~?/Applications/|~?/Library/Application Support/)(?:EastSea|Aether)(?:\.app|/[^\s]*)?)", re.IGNORECASE)
# Human supplied data is not app copy. Exceptions are confined to the screens
# that actually show these exact fixtures (DesignPreview / WalletScreens).
TOKEN_NAME_SCREENS = {"home", "home-empty", "home-paused", "home-verifying", "home-alerts", "home-narrow",
                      "window", "window-network", "window-narrow", "two-accounts", "sheet-assets", "sheet-send-token", "developer"}
TOKEN_NAMES = ("Test Nebula", "Test Orbit", "Test Comet", "Test Dollar", "Doubloon Cash", "Wrapped AETH")
MEMOS = {"sheet-send-link": ("Coffee beans · order 1042",), "sheet-call": ("Swap on the EastSea DEX",)}
MEMO_LINES = {"sheet-send-link": re.compile(r"(?<![A-Za-z])(?:Coffee\s+beans|order\s+1042)(?![A-Za-z])", re.IGNORECASE)}
# The legacy-app alert displays this path as data, not instructions.
FIXTURE_DATA = {"alert-legacy-aether": ("/Applications/Aether.app",),
                "sea-search-web": ("ocean weather",),
                "security": ("Shop",),
                "developer": ("Shop", "localhost", "127.0.0.1")}
RECOVERY_CODE_RE = re.compile(r"(?<![A-Za-z0-9])ae1[0-9a-z]{8,}(?:…|\.*)", re.IGNORECASE)
# This is applied only to bounded Vision observations, not catalog/native
# copy. An address-shaped token must have hex content or a placeholder; a
# nearby ordinary word ("Send", "Copy") is never consumed.
OCR_ADDRESS_RE = re.compile(r"(?<![A-Za-z0-9])(?:[0-9a-zø@日][x×])(?:[0-9a-fgiloq]{4,}(?:[.…⋯]+[0-9a-fgiloq]*)?|[.…⋯]{2,})", re.IGNORECASE)
ICON_GLYPHS = {"く", "っ", "ㄱ", "么", "刁", "30e", "7I", "ロロ", "ロ：", "ロ:", "ロ円", "ロ3", "ロ口", "口口", "口0", "口：", "口:", "谷", "凸", "仚", "㕣", "园", "跆"}
ICON_LETTERS = set("ACDFGNOQSUVYacmnouv")
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
    "browser-start": ("EastSea Search", "Built-in"),
    "browser-tabs": ("EastSea Search", "Private tab"),
    "browser-tabs-narrow": ("EastSea Search", "Private tab"),
    "browser-permissions": ("Site permissions", "Connected account", "Requested permissions",
                            "Read your address", "Request transactions"),
    "browser-find": ("Find in page",),
}
FEATURE_ACCOUNTS = {"switcher": (1, 2), "two-accounts": (2,), "retire-blocked": (2,), "menubar-qr": (2,)}


def normalize(value):
    return "".join(character.casefold() for character in unicodedata.normalize("NFKC", value)
                   if character.isalnum())


def mask_allowed(text, screen, language):
    text = unicodedata.normalize("NFKC", text)
    # Strip exact full phrases before component names (Wrapped AETH, EastSea DEX).
    phrases = list(MEMOS.get(screen, ())) + list(FIXTURE_DATA.get(screen, ()))
    if screen in TOKEN_NAME_SCREENS:
        phrases += list(TOKEN_NAMES)
    if screen.startswith("settings"):
        phrases.append(NATIVE_NAMES[language])
    for phrase in sorted(phrases, key=len, reverse=True):
        text = re.sub(re.escape(phrase), "", text, flags=re.IGNORECASE)
    if screen in MEMO_LINES:
        text = MEMO_LINES[screen].sub("", text)
    text = re.sub(r"[⇧⌘⌥⌃]+[A-Za-z]", "", text)
    if screen in ("security", "developer"):
        text = RECOVERY_CODE_RE.sub("", text)
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
    # A wrapped OCR line can contain the end of one reviewed paragraph and
    # the start of the next. Every sentence fragment must still be present
    # in the declared legal corpus; this does not exempt headings/buttons.
    parts = [part.strip() for part in re.split(r"(?<!\d)\.(?!\d)|[;；!?]", text) if part.strip()]
    if len(parts) > 1:
        return all(legal_allowed(part, screen, language, legal) for part in parts)
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
    # Removing punctuation may otherwise invent a foreign phrase across a
    # label/value boundary: Japanese "目的：API使用料" is not Chinese "的使用".
    normalized_parts = [normalize(part) for part in re.split(r"[:：;；。！？.!?\n]", remainder)]
    if language in ("ja", "zh-Hans", "zh-Hant"):
        match = next((phrase for phrase in foreign if any(phrase in part for part in normalized_parts)), None)
        if match:
            problems.append(f"foreign-only catalog phrase {match!r}")
    return problems


def observation_box(line):
    if not isinstance(line, dict):
        return None
    box = line.get("box")
    if (not isinstance(box, list) or len(box) != 4
            or any(not isinstance(value, (int, float)) for value in box)):
        return None
    x, y, width, height = box
    return box if x >= 0 and y >= 0 and width > 0 and height > 0 and x + width <= 1.01 and y + height <= 1.01 else None


def overlapping(first, second):
    a, b = observation_box(first), observation_box(second)
    if a is None or b is None:
        return False
    width = max(0, min(a[0] + a[2], b[0] + b[2]) - max(a[0], b[0]))
    height = max(0, min(a[1] + a[3], b[1] + b[3]) - max(a[1], b[1]))
    # Both observations must cover essentially the same text, rather than
    # an unrelated nearby label in the same card.
    return width * height >= 0.65 * max(a[2] * a[3], b[2] * b[3])


def catalog_fragments(catalog, language):
    return [re.sub(r"\d", "", normalize(mask_allowed(SPEC.sub("", value), "catalog", language))) for entry in catalog["strings"].values()
            for value in units(entry, language)]


def catalog_fragment(text, fragments):
    value = re.sub(r"\d", "", normalize(mask_allowed(text, "catalog", "en")))
    return bool(value) and any(value in fragment for fragment in fragments)


def chinese_variants(catalog, language):
    variants = {}
    if language not in ("zh-Hans", "zh-Hant"):
        return variants
    other = "zh-Hant" if language == "zh-Hans" else "zh-Hans"
    for entry in catalog["strings"].values():
        for foreign in units(entry, other):
            for own in units(entry, language):
                if HAN.search(foreign) and HAN.search(own):
                    variants.setdefault(normalize(foreign), set()).add(normalize(own))
    return variants


def icon_observation(line, image_size):
    box = observation_box(line)
    if box is None:
        return False
    text = line.get("text", "").strip(" .…・")
    if text not in ICON_GLYPHS and text not in ICON_LETTERS:
        return False
    width, height = box[2] * image_size[0], box[3] * image_size[1]
    # SF symbols in the fixtures are at most 32 pt (64 retina pixels).
    # Short words, full labels and large text cannot qualify as icons.
    return max(width, height) <= 64 and 0.35 <= width / height <= 2.5


def observation_text(line, screen, fragments):
    text = line.get("text", "") if isinstance(line, dict) else line
    if not isinstance(text, str) or observation_box(line) is None:
        return text
    original = unicodedata.normalize("NFKC", text)
    if screen.startswith("sea-search-"):
        # The typed query/registry metadata is fixture data, not UI copy.
        # Vision can join the search glyph to it or split a long hex address.
        data = re.sub(r"^[Qqą@]_?\s*", "", original)
        compact = re.sub(r"\s+", "", data).casefold()
        address = "0x5397a1c0de4b1b8f6a3cb2d1e0f9c7a6b5d4e502"
        if screen == "sea-search-address" and difflib.SequenceMatcher(None, compact, address).ratio() >= 0.9:
            return ""
        if screen == "sea-search-tx" and re.fullmatch(r"(?:[0o]x)?b?(?:ab){3,}a?[bkt]?", compact):
            return ""
        if screen == "sea-search-name" and compact in {"harbor.sea", "harborsea"}:
            return ""
        remainder = mask_allowed(data, screen, "en").strip()
        if not remainder or (screen == "sea-search-block" and remainder.isdecimal()):
            return ""
    text = OCR_ADDRESS_RE.sub("", original)
    if text != original and text.strip() in ICON_LETTERS:
        return ""
    # Vision sometimes joins an SF warning/home glyph to its adjacent label.
    # Remove only one isolated known glyph, and only when the remaining label
    # is actual selected-locale catalog copy. "A Send" in Korean still fails.
    prefix, separator, rest = text.partition(" ")
    if separator and (prefix in ICON_GLYPHS or prefix in ICON_LETTERS) and catalog_fragment(rest, fragments):
        return rest
    # Japanese/Chinese OCR can join the symbol directly to a quoted title.
    if (text[:1] in ICON_LETTERS and len(text) > 1 and
            (HAN.match(text[1]) or KANA.match(text[1]) or HANGUL.match(text[1]) or text[1] in "‘’'\"「“")
            and catalog_fragment(text[1:], fragments)):
        return text[1:]
    if text[-1:] in ICON_GLYPHS and catalog_fragment(text[:-1], fragments):
        return text[:-1]
    return text


def corroborated_observation(line, others, screen, language, legal, foreign, fragments, variants):
    text = observation_text(line, screen, fragments)
    value = normalize(text)
    if len(value) < 2:
        return False
    for other in others:
        if not overlapping(line, other):
            continue
        candidate = observation_text(other, screen, fragments)
        if not isinstance(candidate, str) or text_problems(candidate, screen, language, legal, foreign):
            continue
        # The independent pass must resolve to actual selected-locale copy,
        # and differ only slightly from this OCR read. This does not bless
        # arbitrary low-confidence English or a different Japanese sentence.
        normalized = normalize(candidate)
        if normalized in variants.get(value, ()):
            return True
        if not catalog_fragment(candidate, fragments):
            continue
        remainder = mask_allowed(text, screen, language)
        words = LATIN.findall(remainder)
        # A slightly misread brand can be corroborated. An actual English
        # word such as "Send", including inside a long translated paragraph,
        # cannot be excused by overall sentence similarity.
        brands = [normalize(name) for name in NAMES if len(name) >= 3]
        english_only = all(reason.startswith("English words in")
                           for reason in text_problems(text, screen, language, legal, foreign))
        brand_noise = (words and english_only
                       and all(any(difflib.SequenceMatcher(None, word.casefold(), brand).ratio() >= 0.8
                                   for brand in brands) for word in words))
        legal_noise = (screen in LEGAL_SCREENS and len(value) >= 20
                       and legal_allowed(candidate, screen, language, legal))
        if ((brand_noise or legal_noise)
                and difflib.SequenceMatcher(None, value, normalized).ratio() >= 0.9):
            return True
    return False


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
    literal = set(re.findall(r'\b(?:page|stagePage|window|alert)\(\s*"([^"\n]+)"', text))
    return literal | set(re.findall(r'\(\s*"(sea-search-[^"\n]+)"\s*,', text))


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
    fragments = {language: catalog_fragments(catalog, language) for language in LANGUAGES}
    variants = {language: chinese_variants(catalog, language) for language in LANGUAGES}
    checked, lines_checked = 0, 0
    for screen in sorted(screens):
        for language in LANGUAGES:
            modes = ("light", "dark") if language in ("en", "ko") or screen.startswith("sea-search-") else ("light",)
            for mode in modes:
                stem = f"{screen}-{language}-{mode}"
                png, sidecar = args.out / f"{stem}.png", args.out / f"{stem}.text.json"
                if not png.is_file():
                    problems.append(f"{stem}: missing PNG")
                    continue
                pixels = png.read_bytes()
                if len(pixels) < 24 or pixels[:8] != b"\x89PNG\r\n\x1a\n":
                    problems.append(f"{stem}: invalid PNG")
                    continue
                image_size = struct.unpack(">II", pixels[16:24])
                if min(image_size) <= 0:
                    problems.append(f"{stem}: invalid PNG dimensions")
                    continue
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
                multilingual = payload.get("multilingualLines", [])
                if not lines:
                    problems.append(f"{stem}: no visible text was extracted")
                checked += 1
                for observations, other in ((lines, multilingual), (multilingual, lines)):
                    for line in observations:
                        text = line.get("text", "") if isinstance(line, dict) else line
                        if not isinstance(text, str) or not text.strip():
                            problems.append(f"{stem}: empty or invalid text observation")
                            continue
                        lines_checked += 1
                        vision = payload.get("engine") == "Vision" and observation_box(line) is not None
                        if vision and icon_observation(line, image_size):
                            continue
                        observed = observation_text(line, screen, fragments[language]) if vision else text
                        reasons = text_problems(observed, screen, language, legal, foreign[language])
                        if reasons and not (vision and corroborated_observation(
                                line, other, screen, language, legal, foreign[language], fragments[language], variants[language])):
                            problems.append(f"{stem}: {'; '.join(reasons)}: {text!r}")
                problems += [f"{stem}: {problem}" for problem in feature_problems(payload, screen, language, catalog)]
                if screen == "sheet-terms" and language in ("ja", "zh-Hans", "zh-Hant"):
                    notice = catalog["strings"].get("This translation is for reference; the English text governs.", {})
                    values = units(notice, language)
                    joined = normalize(" ".join(line.get("text", "") if isinstance(line, dict) else line for line in lines))
                    if not values or normalize(values[0]) not in joined:
                        problems.append(f"{stem}: missing translated English-governs legal notice")
    # The two passes may report the same faulty text. Check both, then report
    # each distinct screenshot/copy failure once.
    problems = list(dict.fromkeys(problems))
    for problem in problems:
        print(f"error: wallet-screens-language: {problem}")
    if problems:
        print(f"wallet-screens-language: FAIL ({len(problems)} problems; {checked} screens checked)", file=sys.stderr)
        return 1
    print(f"wallet-screens-language: OK {checked} screens ({len(screens)} views; complete required language/appearance pairs), "
          f"{lines_checked} visible text lines; catalog en/ko/ja/zh-Hans/zh-Hant complete")
    return 0


def self_test():
    # These are detector regressions, not screenshot exceptions. Catalog and
    # native-copy checks continue to reject actual foreign letters/words.
    clean = [
        ("+12（今天）・24筆獎勵・最近一筆 剛剛", "zh-Hant", []),
        ("正在證明區塊・本次執行 57 份", "zh-Hant", []),
        ("0X77c4…1d2e에게 보냄", "ko", []),
        ("０ｘ７７ｃ４…１ｄ２ｅ에게 보냄", "ko", []),
        ("目的：API使用料", "ja", ["的使用"]),
    ]
    for text, language, foreign in clean:
        assert not text_problems(text, "home", language, [], foreign), (language, text)
    wrong_copy = [
        ("Send", "ko", []),
        ("Ｓｅｎｄ", "ko", []),
        ("くり返す", "zh-Hant", []),
        ("ｶﾀｶﾅ", "zh-Hant", []),
        ("发送", "en", []),
        ("已暂停", "zh-Hant", ["已暂停"]),
        ("英文的使用", "ja", ["的使用"]),
        ("0X77c4…1d2e Send", "ko", []),
    ]
    for text, language, foreign in wrong_copy:
        assert text_problems(text, "home", language, [], foreign), (language, text)
    assert not text_problems("받는 곳: Shop", "security", "ko", [], [])
    assert text_problems("Shop", "home", "ko", [], [])
    assert not text_problems("이 기기의 코드 ae1q7m3kx9w2c8v4…", "security", "ko", [], [])
    assert text_problems("ae1q7m3kx9w2c8v4…", "home", "ko", [], [])
    assert not text_problems("Coffee beans・", "sheet-send-link", "ja", [], [])
    assert not text_problems("order 1042", "sheet-send-link", "ja", [], [])
    assert text_problems("Send", "sheet-send-link", "ja", [], [])
    assert text_problems("Coffee beans", "home", "ja", [], [])
    reviewed = ["The rules change only by a committee-signed upgrade.",
                "No token sale, no premine and no founder allocation; the rules are the same.",
                "The floor is 0.1 DBLN a block.", "Testnet DBLN does not carry over.",
                "Nothing here promises a price."]
    assert legal_allowed("upgrade. No token sale, no premine and no founder allocation;", "sheet-terms", "ja", reviewed)
    assert legal_allowed("0.1 DBLN a block. Testnet DBLN does not carry over. Nothing here", "sheet-terms", "zh-Hant", reviewed)
    assert not legal_allowed("Cancel", "sheet-terms", "ja", reviewed)
    assert not legal_allowed("Nothing here promises a price.", "home", "zh-Hant", reviewed)
    keys = (*FEATURE_COPY["menubar-qr"], "Account %lld")
    catalog = {"strings": {key: {"localizations": {"en": {"stringUnit": {"value": key}}}} for key in keys}}
    payload = {"lines": [{"text": "Account 2 Receive Copy address Share"}],
               "qrPayloads": [RECEIVE_ADDRESSES["menubar-qr"]]}
    assert not feature_problems(payload, "menubar-qr", "en", catalog)
    assert feature_problems({**payload, "qrPayloads": [RECEIVE_ADDRESSES["sheet-receive"]]},
                            "menubar-qr", "en", catalog)
    assert feature_problems({**payload, "lines": [{"text": "Account 1 Receive Copy address Share"}]},
                            "menubar-qr", "en", catalog)
    assert feature_problems({**payload, "lines": [{"text": "Account 2 Receive Copy address"}]},
                            "menubar-qr", "en", catalog)
    def observation(text, box=None):
        return {"text": text, "confidence": 0.3, "box": box or [0.1, 0.2, 0.05, 0.025]}
    assert icon_observation(observation("く"), (1000, 1000))
    assert icon_observation(observation("G"), (1000, 1000))
    assert not icon_observation(observation("く", [0.1, 0.2, 0.2, 0.1]), (1000, 1000))
    assert not icon_observation(observation("Send"), (1000, 1000))
    assert not icon_observation(observation("資產"), (1000, 1000))
    assert not icon_observation({"text": "く"}, (1000, 1000))
    assert observation_text(observation("Q https://eastsea.xyz"), "sea-search-url", []) == ""
    assert observation_text(observation("Q Ocean weather"), "sea-search-web", []) == ""
    assert observation_text(observation("Oxabababababababak"), "sea-search-tx", []) == ""
    assert text_problems(observation_text(observation("Q Send"), "sea-search-web", []), "sea-search-web", "ko", [], [])
    assert text_problems(observation_text(observation("0xabababab Send"), "sea-search-tx", []), "sea-search-tx", "ja", [], [])
    assert not icon_observation(observation("ㄱ", [0.1, 0.2, 0.2, 0.1]), (1000, 1000))
    assert text_problems("く", "catalog", "zh-Hant", [], [])
    for value in ("Øx5397..e502", "8x5397.e502", "0×5397.e502", "日x5397.e502", "0x5397a1clde4b1b8f6a3cb2d1e0f9c7a6b5d4e502"):
        assert not text_problems(observation_text(observation(value), "home", []), "home", "ko", [], [])
    assert text_problems(observation_text(observation("Øx5397..e502 Send"), "home", []), "home", "ko", [], [])
    assert text_problems(observation_text(observation("BxSend"), "home", []), "home", "ko", [], [])
    copy = {"strings": {key: {"localizations": {
        "zh-Hans": {"stringUnit": {"value": hans}}, "zh-Hant": {"stringUnit": {"value": hant}}
    }} for key, hans, hant in [("Paused", "已暂停", "已暫停"), ("Assets", "资产", "資產"),
                              ("Send", "发送", "傳送")]}}
    fragments = catalog_fragments(copy, "zh-Hant")
    variants = chinese_variants(copy, "zh-Hant")
    foreign = ["已暂停", "资产"]
    def corroborated(text, primary, box=None):
        return corroborated_observation(observation(text), [observation(primary, box)], "home", "zh-Hant",
                                        [], foreign, fragments, variants)
    assert corroborated("已暂停", "已暫停")
    assert corroborated("资产", "資產")
    assert not corroborated("已暂停", "已暫停", [0.7, 0.7, 0.05, 0.025])
    assert not corroborated("已暂停", "已暂停")
    assert not corroborated("Send", "傳送")
    print("wallet-screens-language self-test: OK punctuation, bounded OCR/icons, dual script evidence, foreign copy, QR, account and features")
    return 0


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, default=ROOT / "tmp/screens")
    parser.add_argument("--only", default="")
    parser.add_argument("--catalog", type=Path, default=CATALOG)
    parser.add_argument("--renderer", type=Path, default=RENDERER)
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--renderer-fixture", action="store_true", help=argparse.SUPPRESS)
    args = parser.parse_args()
    if args.self_test:
        return self_test()
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
