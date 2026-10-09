// Units for the three-way verifier (js/verify.js): the app's native bridge
// when the page is inside the Explore tab, else the wasm module, else none —
// every kind answering the same {verified, height, reason} and never
// rejecting. No wasm binary here: the module and its network file are
// injected, as the node is in rpc.test.mjs.

import test from 'node:test';
import assert from 'node:assert/strict';
import { NOT_COMMITTED, NOT_VERIFIED, detectVerifier, loadWasmVerifier, nativeVerifier, verdict, wasmAccount } from '../js/verify.js';
import { DEFAULT_ENDPOINT, Node as RpcNode } from '../js/rpc.js';

const ADDR = '0x1234567890abcdef1234567890abcdef12345678';
const TX = `0x${'ab'.repeat(32)}`;

function storage() {
  const m = new Map();
  return { getItem: (k) => m.get(k) ?? null, setItem: (k, v) => m.set(k, v), removeItem: (k) => m.delete(k) };
}

/** A node whose account read anchors at height+1 = FINALIZED (rpc.js's
 * verified read, verbatim answers). */
function node({ finalized = { height: 7, block: 'aa', finalization: 'bb', links: [] } } = {}) {
  return {
    calls: [],
    async call(method, params) {
      this.calls.push({ method, params });
      if (method === 'aether_getAccount') return { address: params[0], balance: '5', nonce: 1, height: 6, state_root: 'cc' };
      if (method === 'aether_status') return { chain_id: 7780, height: 7 };
      if (method === 'aether_getFinalized') return finalized == null ? null : { height: params[0], ...finalized };
      throw new Error(`unexpected ${method}`);
    },
  };
}

/** The wasm module as the explorer would load it (crates/wasm verifyAccount). */
function wasmModule({ verify = () => JSON.stringify({ certified_block: 7, timestamp_ms: 123, balance_wei: '5', nonce: 1 }) } = {}) {
  return { default: async () => {}, verifyAccount: verify };
}

// ---- the verdict shape ----

test('verdict normalizes whatever came back', () => {
  assert.deepEqual(verdict(true, 7), { verified: true, height: 7, reason: '' });
  assert.deepEqual(verdict(false), { verified: false, height: null, reason: '' });
  assert.deepEqual(verdict(1, 'nope', 'why'), { verified: true, height: null, reason: 'why' });
});

// ---- native: window.eastsea.verify ----

test('a whole native surface is detected; a partial one is not', () => {
  const whole = { block() {}, account() {}, receipt() {} };
  assert.equal(nativeVerifier({ eastsea: { verify: whole } }), whole);
  const partial = { block() {}, account() {} };
  assert.equal(nativeVerifier({ eastsea: { verify: partial } }), null);
  assert.equal(nativeVerifier({}), null);
});

test('inside the app the native bridge answers, and wins over wasm', async () => {
  const asks = [];
  const win = {
    eastsea: { verify: {
      block: async (h) => (asks.push(['block', h]), { verified: true, height: h, reason: '' }),
      account: async (a) => (asks.push(['account', a]), { verified: false, reason: 'certificate: expired' }),
      receipt: async () => ({ verified: false, reason: NOT_COMMITTED }),
    } },
  };
  const io = { importFn: async () => { throw new Error('must not be reached'); } };
  const v = await detectVerifier(win, io);
  assert.equal(v.kind, 'native');
  const localNode = { url: DEFAULT_ENDPOINT };

  assert.deepEqual(await v.block(localNode, 6), { verified: true, height: 6, reason: '' });
  // A refusal is a verdict, not a rejection — the badge says why.
  assert.deepEqual(await v.account(localNode, ADDR), { verified: false, height: null, reason: 'certificate: expired' });
  assert.deepEqual(await v.receipt(localNode, TX), { verified: false, height: null, reason: NOT_COMMITTED });
  assert.deepEqual(asks, [['block', 6], ['account', ADDR]]);

  // The bridge rejecting (locked 4100, unauthorized 4200) is a refused verdict too.
  const refusing = await detectVerifier({
    eastsea: { verify: {
      block: () => Promise.reject(Object.assign(new Error('This page cannot use EastSea verification.'), { code: 4200 })),
      account: () => Promise.reject(new Error('no')),
      receipt: () => Promise.reject(new Error('no')),
    } },
  }, io);
  assert.deepEqual(await refusing.block(localNode, 6), { verified: false, height: null, reason: 'This page cannot use EastSea verification.' });
});

test('native checks cannot badge remote HTTP data after another read returns to loopback', async () => {
  let nativeCalls = 0;
  const check = async () => { nativeCalls++; return { verified: true, height: 6 }; };
  const verifier = await detectVerifier({ eastsea: { verify: { block: check, account: check, receipt: check } } });
  const remote = new RpcNode('https://personal.example', { fetch: async () => ({ ok: true,
    json: async () => ({ result: { height: 6, hash: 'forged', balance: '999', verified: true } }) }) });
  const displayed = await remote.call('aether_getBlock', [6]);
  const current = { url: DEFAULT_ENDPOINT, kind: 'node' };
  for (const [kind, key] of [['block', 6], ['account', ADDR], ['receipt', TX]]) {
    assert.equal((await verifier[kind](current, key, displayed)).verified, false,
      'the exact remote answer stays unverified after a concurrent source change');
  }
  assert.equal(nativeCalls, 0);
  assert.equal((await verifier.block({ url: 'http://127.0.0.1:18546' }, 6)).verified, false,
    'another local development node is not the native app node');
});

// ---- wasm: the module the extension uses ----

const wasmIO = (mod, network = { chain_id: 7780 }) => ({
  importFn: async (url) => {
    if (url === new URL('../../extension/wasm/aether_wasm.js', import.meta.url).href) return mod;
    throw new Error(`no module at ${url}`);
  },
  fetch: async (url) => {
    if (url === '../extension/network.json') return { ok: true, json: async () => network };
    return { ok: false };
  },
  storage: storage(),
  now: () => 1_000_000,
  sleep: async () => {},
});

test('a standalone explorer loads the wasm module from the second candidate and verifies an account', async () => {
  const io = wasmIO(wasmModule());
  const v = await detectVerifier({}, io);
  assert.equal(v.kind, 'wasm');

  const out = await v.account(node(), ADDR);
  assert.deepEqual(out, { verified: true, height: 7, reason: '' });
  assert.equal(io.storage.getItem('aether-explorer.verifiedHeight.7780'), '7', 'the floor is persisted');

  // The order of the reads mirrors the extension: account, then its anchor.
  const n = node();
  await wasmAccount(n, ADDR, { mod: wasmModule(), network: { chain_id: 7780 }, storage: storage(), now: () => 1, sleep: async () => {} });
  assert.deepEqual(n.calls.map((c) => c.method), ['aether_getAccount', 'aether_getFinalized', 'aether_status']);
});

test('the wasm floor never goes back and a lying node is refused', async () => {
  // The stored floor is ahead of what the certificate reaches: refused.
  const ahead = storage();
  ahead.setItem('aether-explorer.verifiedHeight.7780', '9');
  const low = await wasmAccount(node(), ADDR, { mod: wasmModule(), network: { chain_id: 7780 }, storage: ahead, now: () => 1, sleep: async () => {} });
  assert.equal(low.verified, false);
  assert.equal(low.reason, 'invalid certified height');

  // The module rejecting (bad certificate, wrong chain) is a refused verdict with its message.
  const lying = wasmModule({ verify: () => { throw new Error('certificate: not signed by the committee'); } });
  const out = await wasmAccount(node(), ADDR, { mod: lying, network: { chain_id: 7780 }, storage: storage(), now: () => 1, sleep: async () => {} });
  assert.deepEqual(out, { verified: false, height: null, reason: 'certificate: not signed by the committee' });
});

test('wasm answers block honestly (accounts only) and receipts as not committed', async () => {
  const v = await detectVerifier({}, wasmIO(wasmModule()));
  assert.deepEqual(await v.block({}, 6), { verified: false, height: null, reason: NOT_VERIFIED });
  assert.deepEqual(await v.receipt({}, TX), { verified: false, height: null, reason: NOT_COMMITTED });
});

// ---- none ----

test('with neither bridge nor module every question says not verified', async () => {
  const io = { importFn: async () => { throw new Error('404'); }, fetch: async () => { throw new Error('404'); } };
  const v = await detectVerifier({}, io);
  assert.equal(v.kind, 'none');
  assert.deepEqual(await v.block({}, 6), { verified: false, height: null, reason: NOT_VERIFIED });
  assert.deepEqual(await v.account({}, ADDR), { verified: false, height: null, reason: NOT_VERIFIED });
  assert.deepEqual(await v.receipt({}, TX), { verified: false, height: null, reason: NOT_COMMITTED });
});

test('a module without its network file is no verifier at all', async () => {
  const io = {
    importFn: async () => wasmModule(),
    fetch: async () => ({ ok: false }),
  };
  const env = await loadWasmVerifier(io);
  assert.equal(env, null);
});
