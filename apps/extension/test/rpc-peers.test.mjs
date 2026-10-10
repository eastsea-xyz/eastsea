import test from 'node:test';
import assert from 'node:assert/strict';
import { Rpc } from '../src/lib/rpc.js';

test('extension chain reads fall back to the certified peer pool when HTTP is unavailable', async () => {
  const calls = [];
  const peers = { call: async (method, params) => { calls.push([method, params]); return method === 'eth_blockNumber' ? '0x42' : { height: 66, verified: true }; } };
  const rpc = new Rpc(['http://127.0.0.1:18545'], { fetchImpl: async () => { throw new Error('node off'); }, peerPool: peers });
  assert.equal(await rpc.call('eth_blockNumber'), '0x42');
  assert.deepEqual(calls, [['eth_blockNumber', []]]);
});

test('extension writes require a configured HTTP node and never use public peers', async () => {
  let called = false;
  const rpc = new Rpc([], { peerPool: { call: async () => { called = true; return 'fake'; } } });
  assert.equal(await rpc.call('eth_blockNumber'), 'fake');
  called = false;
  await assert.rejects(rpc.call('aether_sendTransaction', [{}]), (e) => e.code === 4900);
  assert.equal(called, false);
});

test('an extension network switch invalidates an in-flight peer read', async () => {
  let complete, started;
  const entered = new Promise((resolve) => { started = resolve; });
  const answer = new Promise((resolve) => { complete = resolve; });
  let closed = false;
  const rpc = new Rpc([], { peerPool: { call: async () => { started(); return answer; }, close() { closed = true; } } });
  const read = rpc.call('eth_blockNumber');
  await entered;
  rpc.setChain(7777, []);
  complete('0x8');
  await assert.rejects(read, /network changed/);
  assert.equal(closed, true);
  assert.equal(rpc.peerPool, null);
});
