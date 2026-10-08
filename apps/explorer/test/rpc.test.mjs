// Units for the JSON-RPC client and endpoint persistence (js/rpc.js), against
// an injected fetch — no node, no network.

import test from 'node:test';
import assert from 'node:assert/strict';
import {
  DEFAULT_ENDPOINT, DEFAULT_GATEWAY, FailoverNode, Node, RpcError,
  loadEndpoint, loadGateway, localBlockedText, normalizeEndpoint,
  orderedSources, saveEndpoint, saveGateway, sourceLabel,
} from '../js/rpc.js';

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

// ---- the ordered sources: the visitor's node, then the public gateway ----

test('ordered sources: the visitor node first, the gateway after, none when off', () => {
  assert.deepEqual(orderedSources(DEFAULT_ENDPOINT, 'https://gateway.example'), [
    { kind: 'node', url: 'http://127.0.0.1:18545' },
    { kind: 'gateway', url: 'https://gateway.example' },
  ]);
  assert.deepEqual(orderedSources('http://192.168.1.4:18545', null), [
    { kind: 'custom', url: 'http://192.168.1.4:18545' },
  ]);
  assert.equal(sourceLabel({ kind: 'node' }), "Your Mac's node");
  assert.equal(sourceLabel({ kind: 'gateway' }), 'Your gateway · not verified');
  assert.equal(sourceLabel({ kind: 'custom' }), 'Your node');
});

test('the gateway persists, defaults, and turns off with an empty value', () => {
  const s = storage();
  assert.equal(DEFAULT_GATEWAY, '');
  assert.equal(loadGateway(s), null);
  saveGateway('https://gw.example.com/', s);
  assert.equal(loadGateway(s), 'https://gw.example.com');
  saveGateway('', s); // the visitor asked for node-only reads
  assert.equal(loadGateway(s), null);
  assert.throws(() => saveGateway('ftp://x', s), /http/);
  const broken = { getItem: () => { throw new Error('denied'); } };
  assert.equal(loadGateway(broken), null);
});

test('the loopback help names the browser reasons and the ways out', () => {
  const t = localBlockedText();
  for (const word of ['127.0.0.1', 'Chrome', 'Safari', 'Install', 'Allow', 'gateway']) {
    assert.ok(t.includes(word), `the help should mention ${word}: ${t}`);
  }
});

test('failover moves to the gateway only when the node cannot be reached', async () => {
  const clock = { t: 1_000 };
  const wire = { loopbackUp: false };
  const asked = [];
  const changes = [];
  const node = new FailoverNode(
    [
      { kind: 'node', url: 'http://127.0.0.1:18545' },
      { kind: 'gateway', url: 'https://gateway.example' },
    ],
    {
      now: () => clock.t,
      onSource: (n, info) => changes.push(info),
      fetch: async (url) => {
        asked.push(url);
        if (url === 'http://127.0.0.1:18545') {
          if (!wire.loopbackUp) throw new Error('blocked by the browser');
          return jsonResponse({ result: 'from-node' });
        }
        return jsonResponse({ result: 'from-gateway' });
      },
    },
  );
  // The node is unreachable (the Chrome/Safari case): one transport failure,
  // then the gateway answers, and the badge hears about the switch.
  assert.equal(await node.call('aether_status'), 'from-gateway');
  assert.deepEqual(asked, ['http://127.0.0.1:18545', 'https://gateway.example']);
  assert.deepEqual(changes, [{ from: 'node', to: 'gateway' }]);
  assert.equal(node.kind, 'gateway');
  assert.equal(node.url, 'https://gateway.example');

  // While the failure is fresh the loopback is skipped: no second wait on it.
  asked.length = 0;
  assert.equal(await node.call('aether_status'), 'from-gateway');
  assert.deepEqual(asked, ['https://gateway.example']);

  // A minute later the node is tried again, and preferred once it answers.
  wire.loopbackUp = true;
  clock.t += 61_000;
  asked.length = 0;
  changes.length = 0;
  assert.equal(await node.call('aether_status'), 'from-node');
  assert.deepEqual(asked, ['http://127.0.0.1:18545']);
  assert.deepEqual(changes, [{ from: 'gateway', to: 'node' }]);
});

test('an answered error is that source’s answer — not routed around', async () => {
  const node = new FailoverNode(
    [
      { kind: 'node', url: 'http://127.0.0.1:18545' },
      { kind: 'gateway', url: 'https://gateway.example' },
    ],
    {
      now: () => 0,
      fetch: async (url) => (url === 'http://127.0.0.1:18545'
        ? jsonResponse({ error: { code: -32601, message: 'public read-only gateway: aether_sendTransaction is not a public read method' } })
        : jsonResponse({ result: 'must-not-be-used' })),
    },
  );
  await assert.rejects(() => node.call('aether_sendTransaction', []), (e) => e instanceof RpcError && e.code === -32601);
});

test('with every source down the call fails honestly', async () => {
  const node = new FailoverNode(
    [
      { kind: 'node', url: 'http://127.0.0.1:18545' },
      { kind: 'gateway', url: 'https://gateway.example' },
    ],
    { now: () => 0, fetch: async () => { throw new Error('down'); } },
  );
  await assert.rejects(() => node.call('aether_status'), (e) => e instanceof RpcError && /no source answered/.test(e.message));
});

test('read() goes through the failover too, and one source is enough', async () => {
  let body;
  const node = new FailoverNode(
    [{ kind: 'custom', url: 'http://nas.local:18545' }],
    {
      now: () => 0,
      fetch: async (url, opts) => { body = JSON.parse(opts.body); return jsonResponse({ result: '0x2a' }); },
    },
  );
  assert.equal(await node.read('0xabc', '0x1234'), '0x2a');
  assert.equal(body.method, 'eth_call');
  assert.throws(() => new FailoverNode([]), /at least one source/);
});
