import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const wallet = new URL('../../wallet/', import.meta.url);
test('both wallet targets register sea and retain existing link schemes', () => {
  for (const file of ['Info-mac.plist', 'Info-ios.plist']) {
    const plist = readFileSync(new URL(file, wallet), 'utf8');
    for (const scheme of ['sea', 'eastsea', 'aether']) {
      assert.match(plist, new RegExp(`<string>${scheme}</string>`), `${file}: ${scheme}`);
    }
  }
});

test('external DNS names have an explicit HTTPS refusal in five languages', () => {
  const catalog = JSON.parse(readFileSync(new URL('Resources/Localizable.xcstrings', wallet), 'utf8'));
  const key = "Web addresses (.com, etc.) aren't EastSea names. Open them with https://.";
  const entries = catalog.strings[key]?.localizations;
  for (const language of ['en', 'ko', 'ja', 'zh-Hans', 'es']) {
    assert.equal(entries?.[language]?.stringUnit?.state, 'translated', language);
    assert.match(entries[language].stringUnit.value, /\.com/);
    assert.match(entries[language].stringUnit.value, /https:\/\//);
  }
  assert.equal(entries.ko.stringUnit.value, '웹 주소(.com 등)는 동해 이름이 아니에요. https://로 여세요.');
});
