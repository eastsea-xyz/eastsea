// Units for the search classifier and hash resolver (js/search.js), against a
// stub node.

import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import vm from 'node:vm';
import { classifySearch, resolveSearch, searchRoute, decodeSearchQuery } from '../js/search.js';
import { parseSeaURL } from '../js/sea-url.mjs';
import { appSearchView } from '../js/app-search.js';
import { seaLinkView } from '../js/pages.js';
import { h } from '../js/dom.js';

const H = '0x' + 'ab'.repeat(32);
const TX = '0x20979c6bed92c79a5e4ce14ffd2dcabdf96ce0d6afddd09ab2a884234672b78b';
const ADDR = '0xCe4F7dCEB0b51b83C7474713281eF2049f70D5CD';

/** A node whose tables answer; unknown methods throw like a broken transport. */
function stubNode({ receipt, blocks } = {}) {
  return {
    call: async (method, params) => {
      if (method === 'aether_getReceipt' && receipt !== undefined) return typeof receipt === 'function' ? receipt(params[0]) : receipt;
      if (method === 'aether_recentBlocks' && blocks !== undefined) return blocks;
      throw new Error('method not stubbed');
    },
  };
}

test('classifySearch reads heights, addresses and hashes', () => {
  assert.deepEqual(classifySearch('12345'), { kind: 'block', height: 12345 });
  assert.deepEqual(classifySearch(' 7 '), { kind: 'block', height: 7 });
  assert.deepEqual(classifySearch(ADDR), { kind: 'account', address: ADDR.toLowerCase() });
  assert.deepEqual(classifySearch(H), { kind: 'hash', hash: H });
  assert.deepEqual(classifySearch(H.slice(2)), { kind: 'hash', hash: H }); // bare digest works too
  assert.deepEqual(classifySearch(H.toUpperCase()), { kind: 'hash', hash: H });
  assert.equal(classifySearch(''), null);
  assert.equal(classifySearch('  '), null);
  assert.deepEqual(classifySearch('hello'), { kind: 'search', query: 'hello' });
  assert.deepEqual(classifySearch('0x1234'), { kind: 'search', query: '0x1234' });
  assert.equal(classifySearch('12345678901234567890123456789012345678901234567890123456789012345678'), null); // height beyond safe integers
});

test('resolveSearch routes a hash with a receipt to the transaction page', async () => {
  const node = stubNode({ receipt: { height: 5, receipt: { success: true } } });
  assert.deepEqual(await resolveSearch(TX, node), { page: 'tx', hash: TX });
});

test('a pending tx is still a transaction page', async () => {
  const node = stubNode({ receipt: { pending: true }, blocks: [] });
  assert.deepEqual(await resolveSearch(TX, node), { page: 'tx', hash: TX });
});

test('a hash with no receipt may be a recent block hash', async () => {
  const bare = H.slice(2);
  const node = stubNode({ receipt: null, blocks: [{ height: 104554, hash: bare }, { height: 104553, hash: 'ff'.repeat(32) }] });
  assert.deepEqual(await resolveSearch(H, node), { page: 'block', height: 104554 });
  const none = stubNode({ receipt: null, blocks: [] });
  assert.equal(await resolveSearch(H, none), null);
});

test('heights and addresses never hit the node', async () => {
  const broken = stubNode(); // every call throws
  assert.deepEqual(await resolveSearch('42', broken), { page: 'block', height: 42 });
  assert.deepEqual(await resolveSearch(ADDR, broken), { page: 'account', address: ADDR.toLowerCase() });
  assert.deepEqual(await resolveSearch('nonsense', broken), { page: 'search', query: 'nonsense' });
});

test('an unreadable node turns a hash into "not found", not a crash', async () => {
  assert.equal(await resolveSearch(H, stubNode()), null);
});

test('app titles retain indexed search routes without blockchain lookups', async () => {
  for (const query of ['  EastSea games  ', '바다', 'My.app title', 'a/b?c#d & 바다 <script>']) {
    assert.deepEqual(await resolveSearch(query, stubNode()), { page: 'search', query: query.trim() });
  }
});

test('sea names, subdomains and explicit links route to canonical name pages without blockchain lookups', async () => {
  for (const [query, url] of [['  harbor.sea  ', 'sea://harbor.sea'], ['shop.harbor.sea', 'sea://shop.harbor.sea'],
    ['sea://harbor.sea', 'sea://harbor.sea'], ['eastsea://harbor/path?q=%2f', 'eastsea://harbor/path?q=%2f'],
    ['harbor.aeth', 'sea://harbor.aeth']]) {
    const link = parseSeaURL(url, 7780);
    const route = await resolveSearch(query, { call() { assert.fail('name routing must not call the node'); } }, 7780);
    assert.deepEqual(route, { page: 'name', link });
    assert.equal(searchRoute(route), `#/name/${encodeURIComponent(link.canonicalURL)}`);
  }
  const action = 'sea://pay?to=0x1234&amount=10';
  assert.equal(searchRoute(await resolveSearch(action, stubNode(), 7780)), `#/name/${encodeURIComponent(action)}`);
  assert.throws(() => classifySearch('eastsea://example.com', 7780), { code: 'externalTLD' });
});

test('search routes preserve special characters and reject malformed encoding', () => {
  const query = 'a/b?c#d & 바다 <script>';
  const route = searchRoute({ page: 'search', query });
  assert.equal(route, `#/search/${encodeURIComponent(query)}`);
  assert.equal(decodeSearchQuery(route.slice('#/search/'.length)), query);
  assert.equal(decodeSearchQuery('%E0%A4%A'), null);
  assert.equal(searchRoute({ page: 'block', height: 42 }), '#/block/42');
  assert.equal(searchRoute({ page: 'account', address: ADDR.toLowerCase() }), `#/account/${ADDR.toLowerCase()}`);
  assert.equal(searchRoute({ page: 'tx', hash: H }), `#/tx/${H}`);
});

test('the explorer name page keeps indexed results beside the wallet handoff and actions remain consent links', async () => {
  class Element {
    constructor(tag) { this.tag = tag; this.children = []; this.attrs = {}; this.listeners = {}; }
    setAttribute(key, value) { this.attrs[key] = String(value); }
    set className(value) { this.attrs.class = value; }
    addEventListener(type, handler) { (this.listeners[type] ||= []).push(handler); }
    append(...children) { this.children.push(...children.map((child) => child instanceof Element ? child : String(child))); }
    replaceChildren(...children) { this.children = []; this.append(...children); }
  }
  const text = (element) => typeof element === 'string' ? element : element.children.map(text).join(' ');
  const find = (element, predicate) => element instanceof Element
    ? [...(predicate(element) ? [element] : []), ...element.children.flatMap((child) => find(child, predicate))] : [];
  const previousNode = globalThis.Node;
  const previousDocument = globalThis.document;
  globalThis.Node = Element;
  globalThis.document = { createElement: (tag) => new Element(tag) };
  try {
    const calls = [];
    const ctx = { chainId: 7780, locale: 'en', node: { url: 'https://read.example/', async call(method, args) {
      calls.push([method, args]);
      if (method === 'aether_search') return [{ name: 'harbor.sea', title: 'Harbor index result', description: 'Chain-published app',
        category: 'tools', publisher: ADDR, url: 'sea://harbor.sea', verified: true, usage_7d: 3, created_at: 1700000000 }];
      if (method === 'aether_searchInfo') return { usage_complete: true, history_complete: true, sources_configured: true, rejected_records: 0 };
      assert.fail(`unexpected name-page RPC ${method}`);
    } } };
    const app = await readFile(new URL('../js/app.js', import.meta.url), 'utf8');
    const routing = app.match(/const routes = \[[\s\S]*?\n\];/);
    assert.ok(routing, 'the app hash router must be present');
    const scope = { ctx, parseSeaURL, appSearchView, seaLinkView, decodeSearchQuery, h };
    vm.runInNewContext(`${routing[0]}\nglobalThis.routes = routes;`, scope);
    const route = scope.routes.find(([pattern]) => pattern.test('#/name/sea%3A%2F%2Fharbor.sea'));
    assert.ok(route, 'the name hash route must be installed');
    const nameURL = 'sea://harbor.sea/path?q=%2f';
    const encodedName = `#/name/${encodeURIComponent(nameURL)}`;
    const page = await route[1](encodedName.match(route[0]));
    assert.match(text(page), /Harbor index result/);
    assert.match(text(page), /Open in wallet/);
    assert.ok(find(page, (element) => element.tag === 'a' && element.attrs.href === nameURL).length);
    assert.deepEqual(JSON.parse(JSON.stringify(calls)), [['aether_search', ['harbor.sea', 50]], ['aether_searchInfo', []]]);
    calls.length = 0;
    const actionURL = 'sea://pay?to=0x1234&amount=10';
    const action = await route[1](`#/name/${encodeURIComponent(actionURL)}`.match(route[0]));
    assert.match(text(action), /Every payment still needs approval/);
    assert.ok(find(action, (element) => element.tag === 'a' && element.attrs.href === actionURL).length);
    assert.equal(calls.length, 0, 'a wallet action handoff must never invoke the node');
    const bookmark = `#/search/${encodeURIComponent('My.app title')}`;
    const indexedRoute = scope.routes.find(([pattern]) => pattern.test(bookmark));
    assert.ok(indexedRoute, 'existing indexed search bookmarks must remain installed');
    await indexedRoute[1](bookmark.match(indexedRoute[0]));
    assert.deepEqual(calls[0], ['aether_search', ['My.app title', 50]]);
  } finally {
    if (previousNode === undefined) delete globalThis.Node; else globalThis.Node = previousNode;
    if (previousDocument === undefined) delete globalThis.document; else globalThis.document = previousDocument;
  }
});
