// Units for the search classifier and hash resolver (js/search.js), against a
// stub node.

import test from 'node:test';
import assert from 'node:assert/strict';
import { classifySearch, resolveSearch, searchRoute, decodeSearchQuery } from '../js/search.js';

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

test('app titles, .sea names and subdomains route without blockchain lookups', async () => {
  for (const query of ['  harbor.sea  ', 'shop.harbor.sea', 'EastSea games', '바다', 'sea://harbor.sea']) {
    assert.deepEqual(await resolveSearch(query, stubNode()), { page: 'search', query: query.trim() });
  }
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
