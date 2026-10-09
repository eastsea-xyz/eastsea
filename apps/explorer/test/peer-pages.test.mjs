import test from 'node:test';
import assert from 'node:assert/strict';
import { homeView, blockView, txView } from '../js/pages.js';
import { PublicPeerPool } from '../js/peers.js';
import { FailoverNode } from '../js/rpc.js';
import { detectVerifier, NOT_COMMITTED } from '../js/verify.js';

// This smoke checks page output through the real peer reader and RPC failover.
// BLS cryptography itself is exercised by the Rust/WASM and devnet tests.
class El {
  constructor(tag) { this.tagName = tag; this.children = []; }
  setAttribute() {}
  addEventListener() {}
  append(...children) { this.children.push(...children.flat()); }
  replaceChildren(...children) { this.children = children.flat(); }
}
globalThis.Node = El;
globalThis.document = { createElement: (tag) => new El(tag) };
function text(value) { return typeof value === 'string' ? value : value?.children?.map(text).join(' ') || ''; }

const ids = ['11', '22', '33'].map((v) => v.repeat(32));
const network = { chain_id: 7780, identity: 'pinned' };
const header = (height) => ({ height, chain_id: 7780, hash: 'ab'.repeat(32), parent: 'cd'.repeat(32),
  proposer: `0x${'12'.repeat(20)}`, parent_state_root: 'ef'.repeat(32), timestamp_ms: 1000,
  protocol: 4, gas_used: 0, prove_gas: 0, txs: [] });

test('a verified head paints before slow history and concurrent views share the history request', async () => {
  const pool = new PublicPeerPool({ network, peers: ids, now: () => 1000,
    mod: { verifyBlock: (_n, _s, _c, height) => JSON.stringify(header(Number(height))) },
    transport: { call: async (_peer, method, params) => JSON.stringify(method === 'aether_status'
      ? header(3) : method === 'aether_getBlock' ? header(JSON.parse(params)[0])
        : method === 'aether_readPeers' ? ids : { height: JSON.parse(params)[0], block: 'bytes', finalization: 'signed' }),
      closePeer() {}, close() {} } });
  const head = await pool.call('aether_status');
  const block = await pool.call('aether_getBlock', [2]);
  let release, requests = 0;
  const history = new Promise(resolve => { release = resolve; });
  const node = { call: async method => method === 'aether_status' ? head
    : method === 'aether_recentBlocks' ? (requests++, history) : null };
  const first = homeView({ node });
  try {
    const page = await Promise.race([first, new Promise(resolve => setTimeout(() => resolve(null), 30))]);
    assert.ok(page, 'slow history must not hide a verified head');
    assert.match(text(page), /Finalized height/);
    assert.match(text(page), /Loading verified blocks/);
    await homeView({ node });
    assert.equal(requests, 1, 'polling shares the unfinished certified history read');
    release([block]);
    await new Promise(resolve => setImmediate(resolve));
    assert.match(text(page), /abababab/);
    assert.doesNotMatch(text(page), /Loading verified blocks/);
  } finally { release([block]); await first; pool.close(); }
});

test('peer-only home and block pages render verified fields and omit poisoned metrics', async () => {
  const mod = { default: async () => {}, verifyAccount() {},
    verifyBlock: (_n, _s, _c, height) => JSON.stringify(header(Number(height))) };
  const pool = new PublicPeerPool({ network, peers: ids, mod, now: () => 1000,
    transport: {
      call: async (_peer, method, params) => JSON.stringify(method === 'aether_status'
        ? { ...header(3), mempool: 987654321, prover_escrow: '987654321', base_fee: { exec: '987654321' } }
        : method === 'aether_getBlock' ? header(JSON.parse(params)[0])
          : method === 'aether_readPeers' ? ids : { height: JSON.parse(params)[0], block: 'bytes', finalization: 'signed', links: [] }),
      closePeer() {}, close() {},
    } });
  const node = new FailoverNode([{ kind: 'node', url: 'http://127.0.0.1:18545' }], {
    fetch: async () => { throw new Error('node off'); }, peerPool: pool,
  });
  const verifier = await detectVerifier({}, {
    importFn: async () => mod, fetch: async () => ({ ok: true, json: async () => network }),
    storage: null, now: () => 1000,
  });
  const ctx = { node, verifier, chainId: 7780 };
  const home = text(await homeView(ctx));
  assert.ok(home.includes('Finalized height'));
  assert.ok(home.includes('Public chain data verified'));
  assert.ok(home.includes('uncommitted · unavailable'));
  assert.ok(!home.includes('987654321'));
  assert.ok(home.includes('State root — · state proof unavailable'));
  assert.ok(home.includes('Prover escrow — · state proof unavailable'));
  assert.ok(home.includes(`Certified parent state root 0x${'ef'.repeat(32)}`));
  const block = text(await blockView(ctx, 2));
  assert.ok(block.includes('verified by committee certificate'));
  assert.ok(block.includes('verified certified bytes'));
  assert.ok(!block.includes('Not light-client verified'));
  assert.ok(block.includes('State root — · state proof unavailable'));
  assert.ok(block.includes(`Parent state root 0x${'ef'.repeat(32)}`));
  const native = await detectVerifier({ eastsea: { verify: {
    block: async () => { throw new Error('a certified peer object needs no native re-fetch'); },
    account() {}, receipt() {},
  } } });
  const peerBlock = await pool.call('aether_getBlock', [2]);
  assert.equal((await native.block({ url: 'https://personal.example' }, 2, peerBlock)).verified, true,
    'peer provenance remains valid independently of the currently selected source');
  pool.close();
});

test('a legacy receipt label applies only to the displayed block without a commitment', async () => {
  const node = { url: 'http://127.0.0.1:18545', call: async () => ({ height: 2, receipt: {
    success: true, gas_used: 21000, prove_gas: 0, logs: 0, events: [], output: '0x',
  } }) };
  const verifier = { kind: 'wasm', receipt: async () => ({ verified: false, reason: NOT_COMMITTED }) };
  const receipt = text(await txView({ node, verifier }, `0x${'ab'.repeat(32)}`));
  assert.ok(receipt.includes('this block has no receipt commitment'));
  assert.ok(!receipt.includes('no block commits to receipts yet'));
});

test('an HTTP summary cannot earn a verified badge by adding a verified JSON field', async () => {
  const mod = { default: async () => {}, verifyAccount() {}, verifyBlock() {}, verifyReceipt() {} };
  const verifier = await detectVerifier({}, { importFn: async () => mod,
    fetch: async () => ({ ok: true, json: async () => network }), storage: null });
  const node = { url: 'https://personal.example', kind: 'gateway',
    call: async (method) => method === 'aether_getBlock' ? { ...header(2), verified: true, gas_used: 1234567 }
      : null };
  const block = text(await blockView({ node, verifier }, 2));
  assert.ok(!block.includes('verified by committee certificate'));
  assert.ok(block.includes('Not light-client verified'));
  assert.ok(!block.includes('verified certified bytes'));
});
