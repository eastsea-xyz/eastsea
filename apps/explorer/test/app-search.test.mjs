import test from 'node:test';
import assert from 'node:assert/strict';
import { appSearchView, fetchAppSearch, SEARCH_LIMIT, SEARCH_QUERY_BYTES } from '../js/app-search.js';
import { searchCatalog, resolveSearchLocale, searchText } from '../js/search-catalog.js';
import { Node as RpcNode } from '../js/rpc.js';

// A text-only DOM with native replaceChildren coercion. No HTML parser or
// dependency: assertions inspect the actual tree built by the production UI.
class Element {
  constructor(tag) { this.tagName = tag; this.children = []; this.attrs = {}; this.listeners = {}; }
  setAttribute(key, value) { this.attrs[key] = String(value); }
  get className() { return this.attrs.class || ''; }
  set className(value) { this.attrs.class = value; }
  addEventListener(type, handler) { (this.listeners[type] ||= []).push(handler); }
  append(...children) { this.children.push(...children.map((c) => c instanceof Element ? c : String(c))); }
  replaceChildren(...children) { this.children = []; this.append(...children); }
}
globalThis.Node = Element;
globalThis.document = { createElement: (tag) => new Element(tag) };

function text(element) {
  return typeof element === 'string' ? element : element.children.map(text).join(' ');
}
function find(element, predicate) {
  if (!(element instanceof Element)) return [];
  return [...(predicate(element) ? [element] : []), ...element.children.flatMap((child) => find(child, predicate))];
}
const publisher = `0x${'ab'.repeat(20)}`;
const record = (fields = {}) => ({
  name: 'harbor.sea', title: 'Harbor', description: 'A chain-published app', category: 'tools',
  publisher, url: 'sea://harbor.sea', verified: true, usage_7d: 3, created_at: 1700000000,
  lookalike: null, ...fields,
});
const indexInfo = (fields = {}) => ({ usage_complete: true, history_complete: true, sources_configured: true, rejected_records: 0, ...fields });
function node(results = [], info = indexInfo()) {
  const calls = [];
  return {
    url: 'http://127.0.0.1:28545/', calls,
    async call(method, params) {
      calls.push([method, params]);
      if (method === 'aether_search') return typeof results === 'function' ? results() : results;
      if (method === 'aether_searchInfo') {
        if (info instanceof Error) throw info;
        return info;
      }
      throw new Error(`Unexpected method: ${method}`);
    },
  };
}

test('search RPC sends a trimmed query and clamps result limits to 1..50', async () => {
  const stub = node([record()]);
  for (const [input, expected] of [[undefined, 50], [500, 50], [0, 1], [-2, 1], [2.8, 2], [NaN, 50]]) {
    assert.deepEqual(await fetchAppSearch(stub, '  harbor.sea  ', input), [record()]);
    assert.deepEqual(stub.calls.at(-1), ['aether_search', ['harbor.sea', expected]]);
  }
  assert.equal(SEARCH_LIMIT, 50);
});

test('search RPC rejects long UTF-8 queries before reading and skips empty queries', async () => {
  const stub = node();
  assert.equal(SEARCH_QUERY_BYTES, 256);
  assert.deepEqual(await fetchAppSearch(stub, ' '.repeat(400)), []);
  assert.equal(stub.calls.length, 0);
  await assert.rejects(fetchAppSearch(stub, '바'.repeat(86)), { name: 'RangeError', message: 'queryTooLong' });
  assert.equal(stub.calls.length, 0);
  assert.deepEqual(await fetchAppSearch(stub, 'x'.repeat(256)), []);
  assert.deepEqual(stub.calls.at(-1), ['aether_search', ['x'.repeat(256), 50]]);
});

test('the real JSON-RPC transport preserves query/limit wire parameters', async () => {
  const transport = new RpcNode('http://127.0.0.1:28545', {
    fetch: async (url, request) => {
      assert.equal(url, 'http://127.0.0.1:28545');
      const wire = JSON.parse(request.body);
      assert.equal(wire.method, 'aether_search');
      assert.deepEqual(wire.params, ['shop.harbor.sea', 7]);
      return { ok: true, json: async () => ({ jsonrpc: '2.0', id: wire.id, result: [record()] }) };
    },
  });
  assert.deepEqual(await fetchAppSearch(transport, 'shop.harbor.sea', 7), [record()]);
});

test('results preserve the node order even when later rows have more usage and older age', async () => {
  const results = [record({ name: 'harbor.sea', title: 'First', usage_7d: 0 }),
    record({ name: 'prefix.harbor.sea', title: 'Second', usage_7d: 999, created_at: 1, verified: false })];
  const stub = node(results);
  const page = await appSearchView({ node: stub, locale: 'en' }, 'harbor');
  const items = find(page, (el) => el.tagName === 'li');
  assert.equal(items.length, 2);
  assert.match(text(items[0]), /First/);
  assert.match(text(items[1]), /Second/);
  assert.match(text(items[0]), /Content hash present/);
  assert.match(text(items[1]), /No content hash/);
  assert.match(text(page), /does not endorse the publisher/);
  assert.match(text(page), new RegExp(publisher));
  assert.match(text(page), /sea:\/\/harbor.sea/);
  assert.deepEqual(stub.calls, [['aether_search', ['harbor', 50]], ['aether_searchInfo', []]]);
  assert.equal(find(page, (el) => el.attrs['aria-busy'] === 'false').length, 1);
  assert.doesNotMatch(text(page), /null/);
});

test('confusable names carry the node warning next to their publisher and URL', async () => {
  const page = await appSearchView({ node: node([record({ name: 'hаrbor.sea', lookalike: 'harbor.sea' })]), locale: 'ko-KR' }, 'harbor');
  const item = find(page, (el) => el.tagName === 'li')[0];
  const warning = find(item, (el) => el.className === 'msg warn')[0];
  assert.match(text(warning), /유사 이름 \/ 피싱 주의/);
  assert.match(text(warning), /harbor.sea/);
  assert.match(text(item), new RegExp(publisher));
  assert.equal(page.attrs.lang, 'ko');
});

test('chain content, RPC errors and queries cannot inject markup or executable links', async () => {
  const attack = '<img src=x onerror="globalThis.pwned=1">';
  const malicious = record({ name: attack, title: attack, description: attack, publisher: 'javascript:alert(1)',
    category: attack, url: 'javascript:alert(1)', lookalike: attack });
  const page = await appSearchView({ node: node([malicious]), locale: 'en' }, attack);
  assert.match(text(page), /<img src=x onerror=/);
  assert.equal(find(page, (el) => ['img', 'script'].includes(el.tagName)).length, 0);
  assert.equal(find(page, (el) => el.attrs.href?.startsWith('javascript:')).length, 0);
  assert.match(text(page), /javascript:alert\(1\)/); // malformed URLs stay visible
  const failure = await appSearchView({ node: node(() => { throw new Error(attack); }), locale: 'en' }, 'harbor');
  assert.doesNotMatch(text(failure), /onerror/);
});

test('incomplete usage is visible and never represented as zero usage', async () => {
  const page = await appSearchView({ node: node([record()], indexInfo({ usage_complete: false, rejected_records: 7 })), locale: 'en' }, 'harbor');
  assert.match(text(page), /bounded usage limit/);
  assert.match(text(page), /bounded subset/);
  assert.match(text(page), /could not index 7 records/);
  const item = find(page, (el) => el.tagName === 'li')[0];
  assert.match(text(item), /Unavailable/);
  const perRow = await appSearchView({ node: node([record({ usage_complete: false })]), locale: 'en' }, 'harbor');
  assert.match(text(perRow), /bounded usage limit/);
});

test('incomplete event history is visible without claiming an index-capacity event', async () => {
  const page = await appSearchView({ node: node([record()], indexInfo({ history_complete: false })), locale: 'en' }, 'harbor');
  assert.match(text(page), /does not retain the full registry event history/);
  assert.doesNotMatch(text(page), /could not index/);
  assert.equal(find(page, (el) => el.tagName === 'li').length, 1);
});

test('an older node without searchInfo still renders all results', async () => {
  const page = await appSearchView({ node: node([record()], new Error('method not found')), locale: 'en' }, 'harbor');
  assert.equal(find(page, (el) => el.tagName === 'li').length, 1);
  assert.match(text(page), /Harbor/);
  assert.doesNotMatch(text(page), /Unable to read/);
  assert.match(text(page), /did not return readable search index status/);
  assert.match(text(find(page, (el) => el.tagName === 'li')[0]), /Unavailable/);
});

test('absent and invalid index status stays visible without hiding or reordering results', async () => {
  const results = [record({ title: 'First' }), record({ title: 'Second' })];
  const statuses = [null, {}, [], true, 'invalid', indexInfo({ usage_complete: 'true' }), indexInfo({ history_complete: null }),
    indexInfo({ sources_configured: null }), indexInfo({ rejected_records: -1 }), indexInfo({ rejected_records: '0' })];
  for (const info of statuses) {
    const page = await appSearchView({ node: node(results, info), locale: 'en' }, 'harbor');
    assert.match(text(page), /did not return readable search index status/);
    const items = find(page, (el) => el.tagName === 'li');
    assert.deepEqual(items.map((item) => text(find(item, (el) => el.tagName === 'h2')[0])), ['First', 'Second']);
    assert.ok(items.every((item) => text(item).includes('Unavailable')));
  }
});

test('usage numbers require an explicit result or valid index completeness flag', async () => {
  for (const [rowFlag, info, available] of [
    [undefined, null, false], [false, null, false], [true, null, true],
    [undefined, indexInfo(), true], [false, indexInfo(), false], [true, indexInfo({ usage_complete: false }), true],
    [undefined, indexInfo({ usage_complete: false }), false], [undefined, indexInfo({ history_complete: 'true' }), false],
  ]) {
    const page = await appSearchView({ node: node([record({ usage_complete: rowFlag })], info), locale: 'en' }, 'harbor');
    const values = find(find(page, (el) => el.tagName === 'li')[0], (el) => el.tagName === 'dd');
    assert.equal(text(values[3]), available ? '3' : 'Unavailable');
  }
});

test('unconfigured registry sources have a five-language notice and preserve returned rows', async () => {
  for (const locale of Object.keys(searchCatalog)) {
    const page = await appSearchView({ node: node([record()], indexInfo({ sources_configured: false })), locale }, 'harbor');
    assert.ok(text(page).includes(searchText(locale, 'sourcesUnconfigured')));
    assert.equal(find(page, (el) => el.tagName === 'li').length, 1);
  }
});

test('empty, malformed and overlong input have localized states and no fabricated fallback', async () => {
  const empty = await appSearchView({ node: node(), locale: 'es' }, 'harbor');
  assert.match(text(empty), /No hay apps ni nombres/);
  assert.equal(find(empty, (el) => el.tagName === 'li').length, 0);
  const blankNode = node();
  const blank = await appSearchView({ node: blankNode, locale: 'ja' }, '');
  assert.match(text(blank), /検索欄/);
  assert.equal(blankNode.calls.length, 0);
  assert.ok(!text(blank).includes(searchText('ja', 'statusUnavailable')));
  const invalidNode = node();
  const invalid = await appSearchView({ node: invalidNode, locale: 'zh-Hans' }, null);
  assert.match(text(invalid), /文字编码无效/);
  assert.equal(invalidNode.calls.length, 0);
  const long = await appSearchView({ node: node(), locale: 'ko' }, '바'.repeat(86));
  assert.match(text(long), /256바이트/);
});

test('transport and malformed response errors offer a working retry without hidden records', async () => {
  for (const bad of [null, { results: [] }, [null], [record({ verified: 'true' })], [record({ usage_7d: -1 })], Array.from({ length: 51 }, () => record())]) {
    const page = await appSearchView({ node: node(bad), locale: 'en' }, 'harbor');
    assert.match(text(page), /Unable to read search results/);
    assert.equal(find(page, (el) => el.tagName === 'li').length, 0);
  }
  let reads = 0;
  const stub = node(() => { if (!reads++) throw new Error('offline'); return [record()]; });
  const page = await appSearchView({ node: stub, locale: 'en' }, 'harbor');
  const retry = find(page, (el) => el.tagName === 'button')[0];
  await retry.listeners.click[0]();
  assert.equal(find(page, (el) => el.tagName === 'li').length, 1);
  assert.doesNotMatch(text(page), /Unable to read search results/);
});

test('all search labels have the same interpolation tokens in five languages', () => {
  assert.deepEqual(Object.keys(searchCatalog), ['en', 'ko', 'ja', 'zh-Hans', 'es']);
  const keys = Object.keys(searchCatalog.en).sort();
  for (const [locale, catalog] of Object.entries(searchCatalog)) {
    assert.deepEqual(Object.keys(catalog).sort(), keys, locale);
    for (const key of keys) {
      assert.ok(catalog[key].trim(), `${locale}.${key}`);
      assert.deepEqual(catalog[key].match(/\{\w+\}/g), searchCatalog.en[key].match(/\{\w+\}/g), `${locale}.${key}`);
    }
    assert.equal(searchText(locale, 'resultsFor', { query: 'harbor.sea' }).includes('harbor.sea'), true);
  }
  assert.equal(resolveSearchLocale(['fr-FR', 'ko-KR']), 'ko');
  assert.equal(resolveSearchLocale('zh-CN'), 'zh-Hans');
  assert.equal(resolveSearchLocale('es-MX'), 'es');
  assert.equal(resolveSearchLocale(['fr']), 'en');
});
