import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { parseSeaURL, browserInput, suggestedHTTPS, externalNameMessage } from '../js/sea-url.mjs';
import { classifySearch, resolveSearch } from '../js/search.js';

const fixture = JSON.parse(readFileSync(new URL('../../../tests/fixtures/sea-urls.json', import.meta.url)));
for (const row of fixture.cases) {
  test(`sea URL golden: ${row.id}`, () => {
    let actual;
    try {
      actual = (row.operation === 'browser' ? browserInput : parseSeaURL)(row.input, row.chainID);
    } catch (error) {
      actual = { error: error.code };
    }
    assert.deepEqual(actual, row.expected);
    if (row.suggestedHTTPS) assert.equal(suggestedHTTPS(row.input), row.suggestedHTTPS);
  });
}

test('explorer ships the single canonical browser parser source', () => {
  assert.equal(readFileSync(new URL('../js/sea-url.mjs', import.meta.url), 'utf8'),
    readFileSync(new URL('../../shared/sea-url.mjs', import.meta.url), 'utf8'));
});

test('explorer search routes a sea name without submitting a transaction', async () => {
  const link = parseSeaURL('sea://harbor/path?q=%2f');
  assert.deepEqual(classifySearch('sea://harbor/path?q=%2f'), { kind: 'name', link });
  assert.deepEqual(await resolveSearch('harbor.sea', { call() { throw new Error('must not call RPC'); } }),
    { page: 'name', link: parseSeaURL('sea://harbor.sea') });
  assert.throws(() => classifySearch('eastsea://example.com'), { code: 'externalTLD' });
});

test('external TLD refusal is available in all five wallet languages', () => {
  assert.equal(externalNameMessage('ko'), '웹 주소(.com 등)는 동해 이름이 아니에요. https://로 여세요.');
  for (const language of ['en', 'ko', 'ja', 'zh-Hans', 'es']) assert.match(externalNameMessage(language), /https:\/\//);
});

test('HTTPS offers do not reinterpret userinfo, ports or backslashes', () => {
  for (const raw of ['sea://user@harbor.com', 'sea://harbor.com:443', 'sea://harbor.com\\evil', 'sea://harbor.com/#pay']) {
    assert.equal(suggestedHTTPS(raw), null);
  }
});
