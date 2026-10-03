import test from 'node:test';
import assert from 'node:assert/strict';
import { Rpc } from '../src/lib/rpc.js';

const address = '0x00000000000000000000000000000000000000aa';
const network = { chain_id: 7780, identity: 'pinned' };

function setup(servers) {
  const calls = [];
  const stored = new Map();
  const rpc = new Rpc(Object.keys(servers), {
    network,
    now: () => 1_000,
    floorStore: { get: async (key) => stored.get(key), set: async (key, value) => stored.set(key, value) },
    fetchImpl: async (url, init) => {
      const { method } = JSON.parse(init.body);
      calls.push([url, method]);
      if (!servers[url]) throw new Error('offline');
      return { json: async () => ({ result: servers[url][method] }) };
    },
    verifyAccount: async (networkJson, statusJson, accountJson, finalizedJson, requested, floor) => {
      const [net, status, account, finalized] = [networkJson, statusJson, accountJson, finalizedJson].map(JSON.parse);
      assert.equal(net.identity, 'pinned');
      if (account.height + 1 < floor) throw new Error('finalized blocks never go back');
      if (status.chain_id !== net.chain_id || finalized.bad || requested !== address) {
        throw new Error('certificate or proof failed');
      }
      return JSON.stringify({ balance_wei: '4242', nonce: 3, certified_block: account.height + 1 });
    },
  });
  return { rpc, calls, stored };
}

function node(height, bad = false) {
  return {
    aether_getAccount: { height },
    aether_getFinalized: { bad },
    aether_status: { chain_id: 7780 },
  };
}

test('a bad follower is demoted; another supplies a verified balance', async () => {
  const { rpc, calls, stored } = setup({ 'https://liar': node(5, true), 'https://honest': node(6) });
  assert.equal(await rpc.call('eth_getBalance', [address, 'latest']), '0x1092');
  assert.equal(rpc.current, 'https://honest');
  assert.equal(rpc.backoff.get('https://liar').delay, 600_000);
  assert.equal(stored.get('verifiedHeight.7780'), 7);
  await rpc.call('eth_getTransactionCount', [address]);
  assert.equal(calls.filter(([url]) => url === 'https://liar').length, 3, 'demoted follower stays parked');
});

test('a replay below the persisted height is rejected after a worker restart', async () => {
  const { rpc, stored } = setup({ 'https://old': node(5) });
  stored.set('verifiedHeight.7780', 7);
  await assert.rejects(rpc.call('aether_getAccount', [address]), /verified account/);
  assert.equal(stored.get('verifiedHeight.7780'), 7);
  assert.equal(rpc.backoff.get('https://old').kind, 'stale');
  assert.equal(rpc.backoff.get('https://old').delay, 5_000);
});

test('unverified account fields never pass through on verification failure', async () => {
  const { rpc } = setup({ 'https://liar': node(5, true) });
  await assert.rejects(rpc.call('eth_getBalance', [address]), /verified account/);
  await assert.rejects(rpc.call('eth_getTransactionCount', [address]), /verified account/);
});

test('a newly finalized account waits for its next certified block', async () => {
  let attempts = 0;
  const server = node(5);
  Object.defineProperty(server, 'aether_getFinalized', {
    get: () => (++attempts === 1 ? null : { bad: false }),
  });
  const { rpc } = setup({ 'https://follower': server });
  assert.equal(await rpc.call('eth_getBalance', [address]), '0x1092');
  assert.equal(attempts, 2);
  assert.equal(rpc.backoff.has('https://follower'), false);
});
