#!/usr/bin/env python3
"""Check public privacy boundaries and complete, consistent five-language copy.

No build, app launch, network request, or third-party Python dependency is needed.
"""

import json
import os
from pathlib import Path
import plistlib
import re
import shutil
import subprocess
import unittest
from html.parser import HTMLParser


ROOT = Path(__file__).resolve().parents[2]
LANGUAGES = ("en", "ko", "ja", "zh-Hans", "es")
PUBLIC_KEY = "Private keys stay on this device. Addresses, balances, transactions, rewards and registration records are public on chain indefinitely, even after you stop using the app."
REGISTRAR_KEY = "Joining encrypts a DeviceCheck token to Pipln's registrar; only it can decrypt it and send it to Apple (USA), at registration and for daily checks. The registrar keeps the voting key, operator and beacon addresses, node ID and registration time without automatic expiry."
SERVICES_KEY = "Peers and relays see connection IP addresses; RPC nodes see queried addresses. Cloudflare hosts the site and gateway; GitHub receives update requests made by Sparkle, including IP address and app version. Ask privacy@eastsea.xyz to delete removable service data; public chain copies cannot be recalled."
COUNTRY_KEYS = (
    "Country sharing",
    "EastSea can use the country in your Mac's Region setting to choose a broad region bucket. The country stays on this Mac. No country preference is sent until you answer here.",
    "Country sharing is selected below. You can turn it off before continuing.",
    "Choose whether to share a country. Declining keeps all wallet and node features available.",
    "The choice contributes only to broad regional counts. Public observations hide groups smaller than three. This choice is not saved on chain. Stop future sharing anytime in Settings; already received aggregate copies may remain.",
    "Country sharing is optional. Your selected country stays on this Mac and chooses a broad region bucket. Public observations show only counts for groups of at least three. Turning it off stops future use of the country preference. Your relay's region and connection IP address remain visible to peers.",
    "Choose country sharing…",
    "Continue",
    "Don't share country",
    "Share country",
    "%lld node observations",
    "All regions",
    "Counts withheld for privacy",
    "Unverified cohort observations, not a count of distinct Macs. Broad regions use a local country choice or relays; small groups are folded together.",
)


def normalized(text):
    return " ".join(text.split())


class PolicyHTML(HTMLParser):
    def __init__(self):
        super().__init__(convert_charrefs=True)
        self.articles = {}
        self.language = None
        self.block = None
        self.text = []
        self.options = []
        self.ids = []

    def handle_starttag(self, tag, attrs):
        attrs = dict(attrs)
        if "id" in attrs:
            self.ids.append(attrs["id"])
        if tag == "article" and "data-privacy-lang" in attrs:
            self.language = attrs["data-privacy-lang"]
            if attrs.get("lang") != self.language or self.language in self.articles:
                raise AssertionError("Policy language must be unique and match its lang attribute")
            self.articles[self.language] = []
        if self.language and tag in ("h1", "h2", "p"):
            self.block = tag
            self.text = []
        if tag == "option":
            self.options.append(attrs.get("value"))

    def handle_data(self, data):
        if self.block:
            self.text.append(data)

    def handle_endtag(self, tag):
        if tag == self.block:
            self.articles[self.language].append((tag, normalized("".join(self.text))))
            self.block = None
        if tag == "article":
            self.language = None


def markdown_policies(text):
    policies = {}
    for language in LANGUAGES:
        match = re.search(r"^## .* \(" + re.escape(language) + r"\)\n(.*?)(?=^## |\Z)", text, re.M | re.S)
        if not match:
            raise AssertionError(f"Missing canonical {language} policy")
        blocks = []
        for line in match.group(1).splitlines():
            if not line:
                continue
            if line.startswith("**") and line.endswith("**"):
                blocks.append(("h1", normalized(line[2:-2])))
            elif line.startswith("### "):
                blocks.append(("h2", normalized(line[4:])))
            else:
                blocks.append(("p", normalized(line)))
        policies[language] = blocks
    return policies


class PrivacyTextTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.policy = (ROOT / "docs/ops/privacy-policy.md").read_text()
        cls.site = (ROOT / "site/privacy.html").read_text()
        cls.disclaimer = (ROOT / "DISCLAIMER.md").read_text()
        cls.catalog = json.loads((ROOT / "apps/wallet/Resources/Localizable.xcstrings").read_text())["strings"]
        cls.onboarding = (ROOT / "apps/wallet/Sources/Onboarding.swift").read_text()
        cls.settings = (ROOT / "apps/wallet/Sources/SettingsView.swift").read_text()
        cls.parsed = PolicyHTML()
        cls.parsed.feed(cls.site)
        cls.canonical = markdown_policies(cls.policy)

    def test_site_matches_complete_canonical_policy_in_each_language(self):
        self.assertEqual(set(self.parsed.articles), set(LANGUAGES))
        self.assertEqual(self.parsed.options, list(LANGUAGES))
        self.assertEqual(len(self.parsed.ids), len(set(self.parsed.ids)), "Duplicate HTML IDs")
        for language in LANGUAGES:
            with self.subTest(language=language):
                self.assertEqual(self.parsed.articles[language], self.canonical[language])
                self.assertEqual(sum(tag == "h1" for tag, _ in self.canonical[language]), 1)
                self.assertEqual(sum(tag == "h2" for tag, _ in self.canonical[language]), 7)

    def test_disclaimer_contains_the_same_processing_and_deletion_boundaries(self):
        for language, blocks in self.canonical.items():
            for index, (tag, text) in enumerate(blocks):
                # Each language's lead may be omitted by the existing terms section.
                if tag == "p" and index > 1:
                    with self.subTest(language=language, paragraph=index):
                        self.assertIn(text, normalized(self.disclaimer))

    def test_app_privacy_and_both_country_modes_are_translated(self):
        for key in (PUBLIC_KEY, REGISTRAR_KEY, SERVICES_KEY, "Read the privacy policy", "Privacy", *COUNTRY_KEYS):
            with self.subTest(key=key):
                localizations = self.catalog[key]["localizations"]
                self.assertEqual(set(localizations), set(LANGUAGES))
                expected_formats = sorted(re.findall(r"%(?:@|lld|ld|d|f)", key))
                for language in LANGUAGES:
                    unit = localizations[language]["stringUnit"]
                    self.assertEqual(unit["state"], "translated")
                    self.assertTrue(unit["value"].strip())
                    if language != "en":
                        self.assertNotEqual(unit["value"], key, "Missing translation")
                    self.assertEqual(sorted(re.findall(r"%(?:@|lld|ld|d|f)", unit["value"])), expected_formats)

    def test_notices_are_used_in_terms_registration_and_settings(self):
        for key in (PUBLIC_KEY, REGISTRAR_KEY, SERVICES_KEY, "Read the privacy policy"):
            self.assertIn(key, self.onboarding)
        self.assertGreaterEqual(self.onboarding.count(REGISTRAR_KEY), 2)
        self.assertIn('static let privacyURL = URL(string: "https://eastsea.xyz/privacy")!', self.onboarding)
        for key in (PUBLIC_KEY, SERVICES_KEY, "Read the privacy policy"):
            self.assertIn(key, self.settings)
        for key in COUNTRY_KEYS[:10]:
            self.assertIn(key, self.onboarding + self.settings)

    def test_no_old_false_local_only_or_deletion_promises_remain(self):
        surfaces = "\n".join((self.policy, self.site, self.disclaimer, json.dumps(self.catalog, ensure_ascii=False), self.onboarding, self.settings,
                               (ROOT / "site/index.html").read_text(), (ROOT / "site/README.md").read_text()))
        obsolete = (
            "balances, transaction history and reward records stay on your device",
            "잔액, 거래 기록, 보상 내역은 여러분의 기기에만",
            "kept up to 30 days after registration",
            "최대 30일간 보관한 뒤 파기",
            "remain public while the node stays registered",
            "노드가 등록된 동안 공개",
            "we collect no personal data through the Software",
            "등록 외 개인정보 수집 없음",
            "Apart from what Mac registration needs, it collects no personal data",
            "No IP address, city or coordinates are shared.",
        )
        for text in obsolete:
            with self.subTest(text=text):
                self.assertNotIn(text, surfaces)

    def test_all_languages_explain_providers_contact_and_indefinite_public_records(self):
        lifetime = {"en": "indefinitely", "ko": "무기한", "ja": "無期限", "zh-Hans": "无限期", "es": "indefinidamente"}
        for language, blocks in self.canonical.items():
            text = " ".join(value for _, value in blocks)
            with self.subTest(language=language):
                for token in ("Apple", "DeviceCheck", "Cloudflare", "GitHub", "Sparkle", "Google", "Gmail", "privacy@eastsea.xyz", lifetime[language]):
                    self.assertIn(token, text)
                self.assertIn("privacy@eastsea.xyz", self.catalog[SERVICES_KEY]["localizations"][language]["stringUnit"]["value"])

    def test_sparkle_optional_profiling_is_disabled_before_first_check(self):
        with (ROOT / "apps/wallet/Info-mac.plist").open("rb") as file:
            plist = plistlib.load(file)
        self.assertIs(plist["SUEnableSystemProfiling"], False)
        self.assertIn("github.com", plist["SUFeedURL"])
        app = (ROOT / "apps/wallet/Sources/AetherWalletApp.swift").read_text()
        start = app.index("SPUStandardUpdaterController(startingUpdater: false")
        disable = app.index("controller.updater.sendsSystemProfile = false", start)
        run = app.index("controller.startUpdater()", start)
        self.assertLess(start, disable)
        self.assertLess(disable, run)

    @unittest.skipUnless(shutil.which("node"), "Node is needed only for the language-selection execution check")
    def test_language_selector_executes_with_saved_browser_and_blocked_storage(self):
        setup = re.search(r"<script data-privacy-language-setup>(.*?)</script>", self.site, re.S).group(1)
        picker = re.search(r"<script data-privacy-language-picker>(.*?)</script>", self.site, re.S).group(1)
        program = r'''
const fs = require("node:fs");
const vm = require("node:vm");
const assert = require("node:assert/strict");
const {setup, picker} = JSON.parse(fs.readFileSync(0, "utf8"));
for (const [saved, browser, expected, blocked] of [
  ["en", "ko-KR", "en", false], ["ko", "en-US", "ko", false],
  ["ja", "en-US", "ja", false], ["zh-Hans", "en-US", "zh-Hans", false],
  ["es", "en-US", "es", false], ["invalid", "ja-JP", "ja", false],
  [null, "zh-TW", "zh-Hans", false], [null, "es-MX", "es", false],
  [null, "fr-FR", "en", false], ["en", "ko-KR", "ko", true],
]) {
  let change;
  let stored;
  const select = {value: "", addEventListener: (event, callback) => {
    assert.equal(event, "change"); change = callback;
  }};
  const document = {documentElement: {lang: "en"}, getElementById: id => {
    assert.equal(id, "privacy-language"); return select;
  }};
  const context = vm.createContext({document, navigator: {language: browser}, localStorage: {
    getItem: key => {assert.equal(key, "eastsea-lang"); if (blocked) throw Error("blocked"); return saved;},
    setItem: (key, value) => {assert.equal(key, "eastsea-lang"); if (blocked) throw Error("blocked"); stored = value;},
  }, fetch: () => {throw Error("Privacy language choice must not send a network request");}});
  vm.runInContext(setup, context);
  assert.equal(document.documentElement.lang, expected);
  vm.runInContext(picker, context);
  assert.equal(select.value, expected);
  for (const language of ["en", "ko", "ja", "zh-Hans", "es"]) {
    select.value = language; change();
    assert.equal(document.documentElement.lang, language);
    if (!blocked) assert.equal(stored, language);
  }
}
'''
        result = subprocess.run([shutil.which("node"), "-e", program], input=json.dumps({"setup": setup, "picker": picker}),
                                text=True, capture_output=True, cwd=ROOT, env={**os.environ, "TMPDIR": str(ROOT / "tmp")})
        self.assertEqual(result.returncode, 0, result.stderr)


if __name__ == "__main__":
    unittest.main(verbosity=2)
