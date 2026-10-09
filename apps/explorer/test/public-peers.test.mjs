import test from 'node:test';
import assert from 'node:assert/strict';
import { PublicPeerPool, compiledPeers, readVerdict } from '../js/peers.js';

const ids = ['11', '22', '33', '44'].map((s) => s.repeat(32));
const network = { chain_id: 7780, identity: 'pinned', validators: ids.slice(0, 3).map((node) => ({ node })) };
const header = (height = 8) => ({ chain_id: 7780, height, hash: 'ab'.repeat(32), parent: 'cd'.repeat(32),
  timestamp_ms: 1_000, proposer: `0x${'12'.repeat(20)}`, parent_state_root: 'ef'.repeat(32),
  txs: [], gas_used: 0, prove_gas: 0, protocol: 4 });

function setup({ bad = new Set(), slow = new Set(), heights = new Map(), peers = ids.slice(0, 3).map((node) => ({ node })), timeoutMs = 50, monotonic, sleep } = {}) {
  const calls = [], closed = [], stored = new Map();
  const transport = {
    async call(peerJson, method, paramsJson) {
      const peer = JSON.parse(peerJson), params = JSON.parse(paramsJson);
      calls.push([peer.node, method, params]);
      if (slow.has(peer.node)) return new Promise(() => {});
      if (method === 'aether_status') return JSON.stringify({ ...header(heights.get(peer.node) ?? 8), chain_id: 7780, hash: bad.has(peer.node) ? '00'.repeat(32) : header().hash, mempool: 999999, prover_escrow: '999999' });
      if (method === 'aether_getFinalized') return JSON.stringify({ height: params[0], block: 'encoded', finalization: 'signed', links: [] });
      if (method === 'aether_getBlock') return JSON.stringify(header(params[0]));
      if (method === 'aether_readPeers') return JSON.stringify(ids);
      throw new Error(`unsupported ${method}`);
    },
    closePeer(node) { closed.push(node); }, close() {},
  };
  const mod = {
    verifyBlock(net, status, cert, height, floor, now, fresh) {
      assert.equal(JSON.parse(net).identity, 'pinned');
      assert.equal(typeof height, 'bigint');
      assert.equal(typeof floor, 'bigint');
      assert.equal(typeof now, 'bigint');
      const s = JSON.parse(status);
      if (fresh && s.hash !== header().hash) throw new Error('header differs from certificate');
      if (height < floor) throw new Error('finalized blocks never go back');
      return JSON.stringify(header(Number(height)));
    },
  };
  const pool = new PublicPeerPool({ network, peers, transport, mod, now: () => 1_000, timeoutMs, monotonic, sleep,
    floorStore: { get: async (k) => stored.get(k), set: async (k, v) => stored.set(k, v) } });
  return { pool, calls, closed, stored, transport, mod };
}

test('compiled seeds include distinct known network nodes and release peers', () => {
  assert.deepEqual(compiledPeers(network, { chain_id: 7780, peers: [ids[3], ids[0]] }).map((p) => p.node), ids);
  assert.throws(() => compiledPeers(network, { chain_id: 7777, peers: [ids[3]] }), /chain/);
});

test('a cold peer-only read keeps three certified peers and exposes only certified status', async () => {
  const { pool, stored } = setup();
  const status = await pool.call('aether_status');
  assert.equal(pool.livePeers.length, 3);
  assert.equal(status.height, 8);
  assert.equal(status.verified, true);
  assert.equal(readVerdict(status)?.verified, true);
  assert.equal(readVerdict({ ...status }), null, 'JSON claims cannot copy verification provenance');
  assert.equal(status.mempool, null);
  assert.equal(status.prover_escrow, null);
  assert.equal(stored.get('verifiedHeight.7780'), 8);
  assert.ok(pool.metrics.firstVerifiedHeadMs >= 0);
  assert.equal(await pool.call('eth_blockNumber'), '0x8');
  pool.close();
});

test('slightly behind certified peers stay admitted and serve history without lowering live height', async () => {
  const heights = new Map([[ids[0], 9], [ids[1], 8], [ids[2], 7]]);
  const { pool, mod, closed, stored } = setup({ heights, timeoutMs: 1000 });
  const original = mod.verifyBlock;
  const freshChecks = [];
  mod.verifyBlock = (...args) => {
    if (args[6]) freshChecks.push({ height: args[3], floor: args[4] });
    return original(...args);
  };
  try {
    assert.equal((await pool.call('aether_status')).height, 9);
    assert.equal(pool.livePeers.length, 3);
    assert.deepEqual(freshChecks.slice(0, 3), [
      { height: 9n, floor: 0n }, { height: 8n, floor: 0n }, { height: 7n, floor: 0n },
    ], 'each peer still gets a fresh, pinned certificate check');
    assert.equal((await pool.call('aether_status')).height, 9);
    assert.equal(await pool.call('eth_blockNumber'), '0x9');
    const lagging = pool.livePeers.find((peer) => peer.node === ids[2]);
    const block = await pool.block(lagging, 6);
    assert.equal(block.height, 6);
    assert.equal(readVerdict(block)?.height, 6);
    assert.equal(stored.get('verifiedHeight.7780'), 9);
    assert.equal(closed.length, 0);
    heights.set(ids[2], 6);
    await assert.rejects(pool.head(lagging, true), /never go back/,
      'a peer may lag other peers but cannot replay its own older head');
  } finally { pool.close(); }
});

test('a persisted head floor retains fresh lagging peers for history while refusing lower live status', async () => {
  const { pool, closed, stored } = setup({ timeoutMs: 1000 });
  stored.set('verifiedHeight.7780', 9);
  try {
    await assert.rejects(pool.call('aether_status'), /never go back|behind verified/i);
    assert.equal(pool.livePeers.length, 3);
    assert.equal(closed.length, 0);
    await assert.rejects(pool.call('eth_blockNumber'), /never go back|behind verified/i);
    const block = await pool.call('aether_getBlock', [7]);
    assert.equal(readVerdict(block)?.height, 7);
    assert.equal(await pool.floor(), 9);
    assert.equal(closed.length, 0);
  } finally { pool.close(); }
});

test('concurrent peer head refreshes can finish below a newly advanced global floor', async () => {
  const heights = new Map();
  const { pool, transport, closed, stored } = setup({ heights, timeoutMs: 1000 });
  await pool.call('aether_status');
  pool.now = () => 10_000;
  heights.set(ids[0], 9);
  heights.set(ids[1], 10);
  let release;
  const certificate = new Promise((resolve) => { release = resolve; });
  const original = transport.call.bind(transport);
  transport.call = async (peer, method, params) => {
    if (method === 'aether_getFinalized' && JSON.parse(peer).node === ids[0]) await certificate;
    return original(peer, method, params);
  };
  const older = pool.head({ node: ids[0] }, true);
  older.catch(() => {});
  try {
    await new Promise((resolve) => setImmediate(resolve));
    assert.equal((await pool.head({ node: ids[1] }, true)).height, 10);
    release();
    assert.equal((await older).height, 9);
    assert.equal(pool.livePeers.length, 3);
    assert.equal(stored.get('verifiedHeight.7780'), 10);
    pool.cursor = 0;
    assert.equal((await pool.call('aether_status')).height, 10);
    assert.equal(closed.length, 0);
  } finally { release(); pool.close(); await older.catch(() => {}); }
});

test('concurrent refreshes of one peer share the status and certificate requests', async () => {
  const { pool, transport, calls } = setup({ timeoutMs: 1000 });
  await pool.call('aether_status');
  pool.now = () => 10_000;
  calls.length = 0;
  let release;
  const response = new Promise((resolve) => { release = resolve; });
  const original = transport.call.bind(transport);
  transport.call = async (peer, method, params) => {
    const value = await original(peer, method, params);
    if (method === 'aether_status') await response;
    return value;
  };
  const refreshed = Promise.all([pool.head({ node: ids[0] }, true), pool.head({ node: ids[0] }, true)]);
  refreshed.catch(() => {});
  try {
    await new Promise((resolve) => setImmediate(resolve));
    release();
    assert.deepEqual((await refreshed).map((head) => head.height), [8, 8]);
    assert.equal(calls.filter(([, method]) => method === 'aether_status').length, 1);
    assert.equal(calls.filter(([, method]) => method === 'aether_getFinalized').length, 1);
  } finally { release(); pool.close(); await refreshed.catch(() => {}); }
});

test('late sibling errors preserve a temporary quarantine and close the peer only once', async () => {
  const { pool, closed } = setup();
  await pool.call('aether_status');
  try {
    const peer = { node: ids[0] };
    pool.drop(peer, 'public peer timed out');
    const first = { ...pool.dropped.get(peer.node) };
    pool.drop(peer, 'read error: connection lost');
    pool.drop(peer, 'public read peer was dropped');
    assert.deepEqual(pool.dropped.get(peer.node), first);
    assert.equal(closed.filter((node) => node === peer.node).length, 1);
    assert.equal(pool.metrics.rejectedPeers, 1);
    pool.drop(peer, 'header differs from certificate');
    assert.equal(pool.dropped.get(peer.node).reason, 'header differs from certificate');
    assert.equal(pool.dropped.get(peer.node).until, 1000 + 24 * 60 * 60_000);
    assert.equal(closed.filter((node) => node === peer.node).length, 1, 'proof failure can escalate without closing again');
  } finally { pool.close(); }
});

test('a forged header drops its peer and a fourth peer refills the set', async () => {
  const { pool, closed } = setup({ bad: new Set([ids[0]]) });
  assert.equal((await pool.call('aether_status')).height, 8);
  assert.ok(closed.includes(ids[0]));
  assert.equal(pool.livePeers.length, 3);
  assert.ok(pool.livePeers.every((p) => p.node !== ids[0]));
  pool.close();
});

test('a slow peer times out, closes, and is replaced without stalling verified reads', async () => {
  const { pool, closed } = setup({ slow: new Set([ids[0]]), timeoutMs: 10 });
  assert.equal((await pool.call('aether_status')).height, 8);
  assert.ok(closed.includes(ids[0]));
  assert.equal(pool.livePeers.length, 3);
  pool.close();
});

test('historical block fields come from certified bytes; a forged RPC summary is rejected', async () => {
  const { pool, transport, closed } = setup();
  await pool.call('aether_status');
  const original = transport.call.bind(transport);
  transport.call = async (peer, method, params) => {
    if (method === 'aether_getBlock' && JSON.parse(peer).node === ids[0]) return JSON.stringify({ ...header(7), gas_used: 123 });
    return original(peer, method, params);
  };
  let block;
  for (let i = 0; i < 3; i++) block = await pool.call('aether_getBlock', [7]);
  assert.equal(block.gas_used, 0);
  assert.equal(block.verified, true);
  assert.ok(closed.includes(ids[0]));
  assert.equal(block.state_root, null, 'the child-state root has no proof in a block header');
  pool.close();
});

test('uncommitted methods and writes never travel over the public read transport', async () => {
  const { pool, calls } = setup();
  await assert.rejects(pool.call('aether_sendTransaction', [{}]), /read|proof|unsupported/i);
  await assert.rejects(pool.call('eth_call', [{ to: '0xabc' }, 'latest']), /proof|unsupported/i);
  assert.equal(calls.length, 0);
  pool.close();
});

test('selection spreads peers across configured operators and relays before duplicating hints', async () => {
  const peers = [
    { node: ids[0], operator: 'a', relay: 'https://relay-a.example' },
    { node: ids[1], operator: 'a', relay: 'https://relay-a.example' },
    { node: ids[2], operator: 'b', relay: 'https://relay-b.example' },
    { node: ids[3], operator: 'c', relay: 'https://relay-c.example' },
  ];
  const { pool } = setup({ peers });
  await pool.call('aether_status');
  assert.deepEqual(pool.livePeers.map((p) => p.node), [ids[0], ids[2], ids[3]]);
  pool.close();
});

test('a denied persistent store still prevents live-head rollback within the session', async () => {
  const { pool, transport } = setup();
  pool.floorStore = null;
  await pool.call('aether_status');
  pool.now = () => 10_000;
  const original = transport.call.bind(transport);
  transport.call = async (peer, method, params) => method === 'aether_status'
    ? JSON.stringify(header(6)) : original(peer, method, params);
  await assert.rejects(pool.call('aether_status'), /never go back/);
  assert.equal(await pool.floor(), 8);
  pool.close();
});

test('account balances use a next-block proof and never copy unproved code size', async () => {
  const { pool, transport, mod, closed } = setup();
  const address = `0x${'12'.repeat(20)}`;
  const original = transport.call.bind(transport);
  transport.call = async (peer, method, params) => method === 'aether_getAccount'
    ? JSON.stringify({ address, height: 8, state_root: header().parent_state_root, balance: '5', nonce: 3,
      code_size: 999999, proof: { valid: JSON.parse(peer).node !== ids[0] } })
    : original(peer, method, params);
  mod.verifyAccount = (net, status, account, finalized, requested, floor, now) => {
    const a = JSON.parse(account);
    assert.equal(requested, address);
    assert.equal(JSON.parse(finalized).height, 9);
    assert.equal(typeof floor, 'bigint');
    assert.equal(typeof now, 'bigint');
    if (!a.proof.valid) throw new Error('account proof failed');
    return JSON.stringify({ address, balance_wei: '5', nonce: 3, state_height: 8, certified_block: 9, timestamp_ms: 1000 });
  };
  const account = await pool.call('aether_getAccount', [address]);
  assert.equal(account.balance, '5');
  assert.equal(account.code_size, null);
  assert.equal(readVerdict(account)?.height, 9);
  assert.ok(closed.includes(ids[0]));
  pool.close();
});

test('a current account proof overtaken by a concurrent head remains unavailable without evicting its peer', async () => {
  const heights = new Map();
  const { pool, transport, mod, stored, closed } = setup({ heights, timeoutMs: 1000 });
  const address = `0x${'12'.repeat(20)}`;
  await pool.call('aether_status');
  const original = transport.call.bind(transport);
  transport.call = async (peer, method, params) => method === 'aether_getAccount'
    ? JSON.stringify({ address, height: 8, state_root: header().parent_state_root, proof: {} })
    : original(peer, method, params);
  let release;
  const verified = new Promise((resolve) => { release = resolve; });
  mod.verifyAccount = async (_network, _status, _answer, certificate, _address, floor) => {
    assert.equal(JSON.parse(certificate).height, 9);
    assert.equal(floor, 8n);
    await verified;
    return JSON.stringify({ address, balance_wei: '5', nonce: 3, state_height: 8, certified_block: 9, timestamp_ms: 1000 });
  };
  const account = pool.account({ node: ids[0] }, address);
  account.catch(() => {});
  try {
    await new Promise((resolve) => setImmediate(resolve));
    heights.set(ids[1], 10);
    assert.equal((await pool.head({ node: ids[1] }, true)).height, 10);
    release();
    await assert.rejects(account, (error) => error.unavailable && /behind verified|never go back/i.test(error.message));
    assert.equal(stored.get('verifiedHeight.7780'), 10);
    assert.equal(pool.accounts.has(address), false);
    assert.equal(pool.livePeers.length, 3);
    assert.equal(closed.length, 0);
  } finally { release(); pool.close(); await account.catch(() => {}); }
});

test('receipt proof responses bind the requested hash and reject forged receipt data', async () => {
  const { pool, transport, mod, closed } = setup();
  const hash = `0x${'ab'.repeat(32)}`;
  const original = transport.call.bind(transport);
  transport.call = async (peer, method, params) => method === 'aether_getReceiptProof'
    ? JSON.stringify({ height: 7, index: 0, receipt: { tx_hash: hash, success: true, gas_used: 21_000,
      events: [], output: '0x' }, certified_block: { height: 7 }, proof: { valid: JSON.parse(peer).node !== ids[0] } })
    : original(peer, method, params);
  mod.verifyReceipt = (net, status, proof, cert, floor) => {
    const p = JSON.parse(proof);
    assert.deepEqual(JSON.parse(cert), p.certified_block);
    assert.equal(floor, 0n, 'historical receipt inclusion must not lower the live head floor');
    if (!p.proof.valid) throw new Error('receipt proof failed');
    return JSON.stringify({ receipt: p.receipt, height: p.height, index: p.index, certified_block: 7 });
  };
  const receipt = await pool.call('aether_getReceipt', [hash]);
  assert.equal(receipt.receipt.tx_hash, hash);
  assert.equal(readVerdict(receipt)?.height, 7);
  assert.ok(closed.includes(ids[0]));
  pool.close();
});

test('missing historical data and invalid caller parameters do not evict certified peers', async () => {
  const { pool, transport, mod, closed } = setup();
  await pool.call('aether_status');
  const original = transport.call.bind(transport);
  transport.call = async (peer, method, params) => method === 'aether_getReceiptProof' ? 'null' : original(peer, method, params);
  mod.verifyReceipt = () => { throw new Error('must not verify missing data'); };
  await assert.rejects(pool.call('aether_getReceipt', [`0x${'ab'.repeat(32)}`]), /no certified receipt/);
  await assert.rejects(pool.call('aether_getAccount', ['invalid']), /address/);
  await assert.rejects(pool.call('aether_getBlock', [-1]), /height/);
  assert.equal(pool.livePeers.length, 3);
  assert.equal(closed.length, 0);
  assert.equal((await pool.call('aether_status')).height, 8);
  pool.close();
});

test('account verification waits for a fixed next-block certificate without dropping an honest peer', async () => {
  const { pool, transport, mod, closed } = setup();
  const address = `0x${'12'.repeat(20)}`;
  const original = transport.call.bind(transport);
  let accounts = 0;
  const targets = [];
  transport.call = async (peer, method, params) => {
    if (method === 'aether_getAccount') {
      accounts++;
      return JSON.stringify({ address, height: 8, state_root: header().parent_state_root, balance: '5', nonce: 3, proof: {} });
    }
    if (method === 'aether_getFinalized' && JSON.parse(params)[0] === 9) {
      targets.push(JSON.parse(params)[0]);
      return targets.length === 1 ? 'null' : JSON.stringify({ height: 9, block: 'encoded', finalization: 'signed', links: [] });
    }
    return original(peer, method, params);
  };
  mod.verifyAccount = (_net, _status, account, finalized, requested, floor, now) => {
    assert.equal(JSON.parse(account).height, 8);
    assert.equal(JSON.parse(finalized).height, 9);
    assert.equal(requested, address);
    assert.equal(floor, 8n);
    assert.equal(now, 1000n);
    return JSON.stringify({ address, balance_wei: '5', nonce: 3, state_height: 8, certified_block: 9, timestamp_ms: 1000 });
  };
  const account = await pool.call('aether_getAccount', [address]);
  assert.equal(account.balance, '5');
  assert.equal(accounts, 1, 'the original proof remains pinned while the chain tip advances');
  assert.deepEqual(targets, [9, 9]);
  assert.equal(closed.length, 0);
  assert.equal(pool.livePeers.length, 3);
  pool.close();
});

test('account certificate polling has a three-second budget and never re-fetches the account', async () => {
  const { pool, transport, mod, closed } = setup();
  const address = `0x${'12'.repeat(20)}`;
  await pool.call('aether_status');
  const peer = pool.livePeers[0];
  const original = transport.call.bind(transport);
  let accounts = 0;
  const targets = [], waits = [];
  let elapsed = 0;
  pool.monotonic = () => elapsed;
  pool.sleep = async (ms) => { waits.push(ms); elapsed += ms; };
  transport.call = async (peer, method, params) => {
    if (method === 'aether_getAccount') {
      accounts++;
      return JSON.stringify({ address, height: 8, state_root: header().parent_state_root, balance: '5', nonce: 3, proof: {} });
    }
    if (method === 'aether_getFinalized' && JSON.parse(params)[0] === 9) { targets.push(9); return 'null'; }
    return original(peer, method, params);
  };
  mod.verifyAccount = () => { throw new Error('a missing certificate must not be verified'); };
  await assert.rejects(pool.account(peer, address), /not finalized yet/);
  assert.equal(accounts, 1);
  assert.ok(targets.length >= 12 && targets.length <= 13, 'the account path has an immediate poll and at most twelve 250ms retries');
  assert.ok(targets.every((height) => height === 9));
  assert.equal(waits.length, 12);
  assert.ok(waits.reduce((sum, ms) => sum + ms, 0) <= 3000);
  assert.equal(closed.length, 0);
  pool.close();
});

test('a malformed account certificate is dropped immediately rather than polled again', async () => {
  const { pool, transport, mod, closed } = setup();
  const address = `0x${'12'.repeat(20)}`;
  const original = transport.call.bind(transport);
  let liarPolls = 0;
  transport.call = async (peer, method, params) => {
    if (method === 'aether_getAccount') return JSON.stringify({ address, height: 8, state_root: header().parent_state_root, balance: '5', nonce: 3, proof: {} });
    if (method === 'aether_getFinalized' && JSON.parse(params)[0] === 9) {
      const bad = JSON.parse(peer).node === ids[0];
      if (bad) liarPolls++;
      return bad ? 'false' : JSON.stringify({ height: 9, block: 'encoded', finalization: 'signed', links: [] });
    }
    return original(peer, method, params);
  };
  mod.verifyAccount = (_net, _status, _account, finalized) => {
    if (JSON.parse(finalized).finalization !== 'signed') throw new Error('account certificate failed');
    return JSON.stringify({ address, balance_wei: '5', nonce: 3, state_height: 8, certified_block: 9, timestamp_ms: 1000 });
  };
  assert.equal((await pool.call('aether_getAccount', [address])).balance, '5');
  assert.equal(liarPolls, 1);
  assert.ok(closed.includes(ids[0]));
  pool.close();
});

test('concurrent public reads respect a peer burst of sixteen and eight requests per second', async () => {
  let tick = 0;
  const waits = [], sent = [];
  const { pool, transport } = setup({ timeoutMs: 5000, monotonic: () => tick,
    sleep: (ms) => new Promise((resolve) => { waits.push({ at: tick + ms, resolve }); }) });
  let tokens = 16, last = 0;
  transport.call = async () => {
    const refill = Math.floor((tick - last) * 8 / 1000);
    if (refill) { tokens = Math.min(16, tokens + refill); last = tick; }
    if (tokens === 0) throw new Error('server busy: peer burst exhausted');
    tokens--;
    sent.push(tick);
    return '{}';
  };
  const finished = Promise.all(Array.from({ length: 20 }, () => pool.raw({ node: ids[0] }, 'aether_status')));
  finished.catch(() => {}); // the old unpaced implementation fails this assertion
  for (let turn = 0; turn < 8; turn++) {
    await new Promise((resolve) => setImmediate(resolve));
    if (!waits.length) break;
    tick = Math.min(...waits.map((w) => w.at));
    for (const wait of waits.splice(0)) {
      if (wait.at <= tick) wait.resolve();
      else waits.push(wait);
    }
  }
  await finished;
  assert.deepEqual(sent.slice(0, 16), Array(16).fill(0));
  assert.deepEqual(sent.slice(16), [125, 250, 375, 500]);
  assert.equal(tick, 500);
  pool.close();
});

test('pacing consumes the existing call deadline and refuses a request before sending it late', async () => {
  let tick = 0;
  const sent = [];
  const { pool, transport } = setup({ timeoutMs: 5000, monotonic: () => tick,
    sleep: async (ms) => { tick += ms; } });
  transport.call = async (peer) => { sent.push(JSON.parse(peer).node); return '{}'; };
  for (let n = 0; n < 16; n++) await pool.raw({ node: ids[0] }, 'aether_status');
  await assert.rejects(pool.raw({ node: ids[0] }, 'aether_status', [], 50), /deadline/);
  assert.equal(sent.length, 16, 'an expired local queue budget never reaches the peer');
  await pool.raw({ node: ids[1] }, 'aether_status', [], 50);
  assert.equal(tick, 0, 'one depleted peer does not pace a different peer');
  await pool.raw({ node: ids[0] }, 'aether_status', [], 500);
  assert.equal(tick, 125);
  assert.equal(sent.length, 18);
  pool.close();
});

test('a paced request gives transport only the remaining deadline', async () => {
  let tick = 0;
  const { pool, transport } = setup({ timeoutMs: 5000, monotonic: () => tick,
    sleep: async (ms) => { tick += ms; } });
  transport.call = async () => '{}';
  for (let n = 0; n < 16; n++) await pool.raw({ node: ids[0] }, 'aether_status');
  let wireStarted = false;
  transport.call = () => { wireStarted = true; return new Promise(() => {}); };
  const originalTimer = globalThis.setTimeout;
  let wireBudget;
  try {
    globalThis.setTimeout = (callback, ms) => {
      if (wireStarted) { wireBudget = ms; queueMicrotask(callback); }
      return null;
    };
    await assert.rejects(pool.raw({ node: ids[0] }, 'aether_status', [], 200), /timed out/);
  } finally { globalThis.setTimeout = originalTimer; }
  assert.equal(wireBudget, 75, '125ms of pacing consumes the original 200ms transport deadline');
  pool.close();
});

test('a cold thirty-block page respects three honest peers rate gates without dropping them', async () => {
  let tick = 0;
  const waits = [], gates = new Map();
  const { pool, transport, closed } = setup({ timeoutMs: 5000, monotonic: () => tick,
    sleep: (ms) => new Promise((resolve) => { waits.push({ at: tick + ms, resolve }); }) });
  const original = transport.call.bind(transport);
  let busy = 0;
  transport.call = async (peerJson, method, params) => {
    const node = JSON.parse(peerJson).node;
    let gate = gates.get(node);
    if (!gate) { gate = { tokens: 16, last: tick }; gates.set(node, gate); }
    const refill = Math.floor((tick - gate.last) * 8 / 1000);
    if (refill) { gate.tokens = Math.min(16, gate.tokens + refill); gate.last = tick; }
    if (gate.tokens === 0) { busy++; throw new Error('server busy: peer burst exhausted'); }
    gate.tokens--;
    return method === 'aether_status' ? JSON.stringify(header(40)) : original(peerJson, method, params);
  };
  let done = false;
  const result = pool.call('aether_recentBlocks', [30]).finally(() => { done = true; });
  result.catch(() => {});
  for (let turn = 0; turn < 128 && !done; turn++) {
    await new Promise((resolve) => setImmediate(resolve));
    if (!waits.length) continue;
    tick = Math.min(...waits.map((w) => w.at));
    for (const wait of waits.splice(0)) {
      if (wait.at <= tick) wait.resolve();
      else waits.push(wait);
    }
  }
  const blocks = await result;
  assert.equal(blocks.length, 30);
  assert.equal(blocks[0].height, 40);
  assert.equal(blocks.at(-1).height, 11);
  assert.equal(pool.livePeers.length, 3);
  assert.equal(busy, 0);
  assert.equal(closed.length, 0);
  assert.ok(tick > 0, 'the full page must wait for actual budget refill');
  pool.close();
});

test('a genuine server-busy response still drops the peer despite local pacing', async () => {
  const { pool, transport, closed } = setup();
  await pool.call('aether_status');
  const busyPeer = pool.livePeers[pool.cursor % pool.livePeers.length].node;
  const original = transport.call.bind(transport);
  transport.call = async (peer, method, params) => {
    if (method === 'aether_getBlock' && JSON.parse(peer).node === busyPeer) throw new Error('server busy');
    return original(peer, method, params);
  };
  assert.equal((await pool.call('aether_getBlock', [7])).height, 7);
  assert.ok(closed.includes(busyPeer));
  assert.equal(pool.dropped.get(busyPeer).reason, 'server busy');
  pool.close();
});

test('a concurrent receipt page keeps honest peers below their four-stream limit', async () => {
  const { pool, transport, mod, closed } = setup({ timeoutMs: 1000 });
  await pool.call('aether_status');
  const original = transport.call.bind(transport);
  const active = new Map(), pending = [];
  let maximum = 0, done = false;
  transport.call = async (peerJson, method, params) => {
    if (method !== 'aether_getReceiptProof') return original(peerJson, method, params);
    const node = JSON.parse(peerJson).node;
    const count = (active.get(node) || 0) + 1;
    maximum = Math.max(maximum, count);
    if (count > 4) throw new Error('public read rate or concurrency limit reached');
    active.set(node, count);
    try {
      await new Promise((resolve) => pending.push(resolve));
      return JSON.stringify({ height: 7, receipt: { tx_hash: JSON.parse(params)[0] }, certified_block: {} });
    } finally { active.set(node, active.get(node) - 1); }
  };
  mod.verifyReceipt = (_network, _status, answer) => JSON.stringify(JSON.parse(answer));
  const finished = Promise.all(Array.from({ length: 18 }, (_, i) => pool.call('aether_getReceipt',
    [`0x${(i + 1).toString(16).padStart(64, '0')}`]))).finally(() => { done = true; });
  finished.catch(() => {});
  try {
    for (let turn = 0; turn < 32 && !done; turn++) {
      await new Promise((resolve) => setImmediate(resolve));
      for (const resolve of pending.splice(0)) resolve();
    }
    assert.equal((await finished).length, 18);
    assert.equal(maximum, 4);
    assert.equal(closed.length, 0);
    assert.equal(pool.livePeers.length, 3);
  } finally {
    for (const resolve of pending.splice(0)) resolve();
    pool.close();
    await finished.catch(() => {});
  }
});

test('transport-wide concurrency never exceeds the wasm limit of thirty-two calls', async () => {
  const { pool, transport } = setup({ timeoutMs: 1000 });
  const pending = [];
  let active = 0, maximum = 0, done = false;
  transport.call = async () => {
    active++;
    maximum = Math.max(maximum, active);
    try {
      if (active > 32) throw new Error('too many public read calls in flight');
      await new Promise((resolve) => pending.push(resolve));
      return '{}';
    } finally { active--; }
  };
  const finished = Promise.all(Array.from({ length: 36 }, (_, i) => pool.raw({
    node: (Math.floor(i / 4) + 1).toString(16).padStart(2, '0').repeat(32),
  }, 'aether_status'))).finally(() => { done = true; });
  finished.catch(() => {});
  try {
    for (let turn = 0; turn < 16 && !done; turn++) {
      await new Promise((resolve) => setImmediate(resolve));
      for (const resolve of pending.splice(0)) resolve();
    }
    assert.equal((await finished).length, 36);
    assert.equal(maximum, 32);
  } finally {
    for (const resolve of pending.splice(0)) resolve();
    pool.close();
    await finished.catch(() => {});
  }
});

test('a timed-out wire call keeps its slot until transport actually settles', async () => {
  const { pool, transport } = setup();
  const pending = [];
  let sent = 0;
  transport.call = () => { sent++; return new Promise((resolve) => pending.push(resolve)); };
  try {
    const first = await Promise.allSettled(Array.from({ length: 4 }, () => pool.raw({ node: ids[0] }, 'aether_status', [], 20)));
    assert.ok(first.every((result) => result.status === 'rejected' && /timed out/.test(result.reason.message)));
    await assert.rejects(pool.raw({ node: ids[0] }, 'aether_status', [], 20),
      (error) => error.unavailable && /deadline/.test(error.message));
    assert.equal(sent, 4, 'a JavaScript timeout cannot free an occupied iroh stream');
    for (const resolve of pending.splice(0)) resolve('{}');
    await new Promise((resolve) => setImmediate(resolve));
    transport.call = async () => { sent++; return '{}'; };
    await pool.raw({ node: ids[0] }, 'aether_status');
    assert.equal(sent, 5, 'settled wire calls return their permits');
  } finally {
    for (const resolve of pending.splice(0)) resolve('{}');
    pool.close();
  }
});

test('closing the pool wakes a queued concurrency waiter without sending it', async () => {
  const { pool, transport } = setup();
  const pending = [];
  let sent = 0;
  transport.call = () => { sent++; return new Promise((resolve) => pending.push(resolve)); };
  const first = Array.from({ length: 4 }, () => pool.raw({ node: ids[0] }, 'aether_status', [], 100));
  const finished = Promise.allSettled(first);
  await new Promise((resolve) => setImmediate(resolve));
  const queued = pool.raw({ node: ids[0] }, 'aether_status', [], 100);
  queued.catch(() => {});
  await new Promise((resolve) => setImmediate(resolve));
  try {
    pool.close();
    await assert.rejects(queued, /closed/);
    assert.equal(sent, 4);
  } finally {
    for (const resolve of pending.splice(0)) resolve('{}');
    pool.close();
    await finished;
  }
});
