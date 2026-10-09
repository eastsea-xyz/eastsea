import test from 'node:test';
import assert from 'node:assert/strict';
let api = {};
try { api = await import('../src/lib/i18n.js'); } catch {}

test('new dApp signing strings cover English, Korean, Japanese and both Chinese scripts', () => {
  assert.ok(api.STRINGS, 'signing translations must exist');
  const keys = Object.keys(api.STRINGS.en).sort();
  assert.ok(keys.includes('simulationUnavailable'));
  assert.ok(keys.includes('typedRequest'));
  for (const locale of ['en', 'ko', 'ja', 'zh-Hans', 'zh-Hant']) {
    assert.deepEqual(Object.keys(api.STRINGS[locale]).sort(), keys);
    for (const key of keys) {
      assert.ok(api.STRINGS[locale][key].trim().length > 0, `${locale}.${key}`);
      assert.deepEqual(api.STRINGS[locale][key].match(/\{\w+\}/g)?.sort() || [], api.STRINGS.en[key].match(/\{\w+\}/g)?.sort() || [], `${locale}.${key} placeholders`);
    }
  }
});

test('signing localization selects the browser language and substitutes readable values', () => {
  assert.equal(typeof api.language, 'function', 'language routing must exist');
  assert.equal(api.language('ko-KR'), 'ko');
  assert.equal(api.language('ja-JP'), 'ja');
  assert.equal(api.language('zh-TW'), 'zh-Hant');
  assert.equal(api.language('zh-HK'), 'zh-Hant');
  assert.equal(api.language('zh-CN'), 'zh-Hans');
  assert.equal(api.language('zh-Hans-SG'), 'zh-Hans');
  assert.equal(api.language('fr-FR'), 'en');
  assert.equal(api.t('chainLabel', { chain: 7780 }, 'en'), 'Chain 7780');
  assert.notEqual(api.t('simulationUnavailable', {}, 'ko'), api.t('simulationUnavailable', {}, 'en'));
});
