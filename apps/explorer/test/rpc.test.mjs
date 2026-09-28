// Units for the JSON-RPC client and endpoint persistence (js/rpc.js), against
// an injected fetch — no node, no network.

import test from 'node:test';
import assert from 'node:assert/strict';
import { DEFAULT_ENDPOINT, Node, RpcError, loadEndpoint, normalizeEndpoint, saveEndpoint } from '../js/rpc.js';

const jsonResponse = (body, status = 200) => ({ ok: status >= 200 && status < 300, status, json: async () => body });

function storage() {
  const m = new Map();
  return { getItem: (k) => m.get(k) ?? null, setItem: (k, v) => m.set(k, v), removeItem: (k) => m.delete(k) };
}

test('the envelope carries method and params; the result comes back', async () => {
  let seen;
  const node = new Node('http://127.0.0.1:18545', {
    fetch: async (url, opts) => {
      seen = { url, body: JSON.parse(opts.body) };
      return jsonResponse({ jsonrpc: '2.0', id: seen.body.id, result: 42 });
    },
  });
  assert.equal(await node.call('aether_getBlock', [5]), 42);
  assert.equal(seen.url, 'http://127.0.0.1:18545');
  assert.equal(seen.body.jsonrpc, '2.0');
  assert.equal(seen.body.method, 'aether_getBlock');
  assert.deepEqual(seen.body.params, [5]);
  assert.ok(Number.isInteger(seen.body.id));
});

test('a node error object becomes an RpcError with its code', async () => {
  const node = new Node('http://node.test', {
    fetch: async () => jsonResponse({ jsonrpc: '2.0', id: 1, error: { code: -32001, message: 'pruned: this node keeps blocks from height 99' } }),
  });
  await assert.rejects(() => node.call('aether_getFinalized', [5]), (e) => e instanceof RpcError && e.code === -32001 && /pruned/.test(e.message));
});

test('HTTP and transport failures are RpcErrors', async () => {
  const down = new Node('http://node.test', { fetch: async () => jsonResponse({}, 503) });
  await assert.rejects(() => down.call('aether_status'), (e) => /HTTP 503/.test(e.message));
  const gone = new Node('http://node.test', { fetch: async () => { throw new Error('ECONNREFUSED'); } });
  await assert.rejects(() => gone.call('aether_status'), (e) => /could not reach the node/.test(e.message));
  const notJson = new Node('http://node.test', { fetch: async () => ({ ok: true, status: 200, json: async () => { throw new Error('bad json'); } }) });
  await assert.rejects(() => notJson.call('aether_status'), (e) => /did not answer JSON/.test(e.message));
});

test('a silent node aborts at the timeout', async () => {
  const node = new Node('http://node.test', {
    timeoutMs: 25,
    fetch: (url, opts) => new Promise((_, reject) => opts.signal.addEventListener('abort', () => reject(Object.assign(new Error('aborted'), { name: 'AbortError' })))),
  });
  await assert.rejects(() => node.call('aether_status'), (e) => /did not answer in 25 ms/.test(e.message));
});

test('read() shapes an eth_call', async () => {
  let body;
  const node = new Node('http://node.test', { fetch: async (url, opts) => { body = JSON.parse(opts.body); return jsonResponse({ result: '0x01' }); } });
  assert.equal(await node.read('0xabc', '0x313ce567'), '0x01');
  assert.equal(body.method, 'eth_call');
  assert.deepEqual(body.params, [{ to: '0xabc', data: '0x313ce567' }, 'latest']);
});

test('normalizeEndpoint accepts http(s), strips slashes, rejects the rest', () => {
  assert.equal(normalizeEndpoint(' http://127.0.0.1:18545/ '), 'http://127.0.0.1:18545');
  assert.equal(normalizeEndpoint('http://localhost:8080///'), 'http://localhost:8080');
  assert.equal(normalizeEndpoint('https://node.example.com'), 'https://node.example.com');
  assert.throws(() => normalizeEndpoint('127.0.0.1:18545'), /not a URL/);
  assert.throws(() => normalizeEndpoint('ws://x'), /http/);
  assert.throws(() => normalizeEndpoint(''), /not a URL/);
});

test('endpoints persist when storage works and default when it does not', () => {
  const s = storage();
  assert.equal(loadEndpoint(s), DEFAULT_ENDPOINT);
  saveEndpoint('http://192.168.1.4:18545/', s);
  assert.equal(loadEndpoint(s), 'http://192.168.1.4:18545');
  assert.throws(() => saveEndpoint('nope', s), /not a URL/);
  assert.equal(loadEndpoint(null), DEFAULT_ENDPOINT); // no storage at all
  const broken = { getItem: () => { throw new Error('denied'); } };
  assert.equal(loadEndpoint(broken), DEFAULT_ENDPOINT); // private browsing, say
});
