import test from 'node:test';
import assert from 'node:assert/strict';
import { Rpc } from '../src/lib/rpc.js';

function fakeNet(handlers) {
  const calls = [];
  const fetchImpl = async (url, init) => {
    const { method } = JSON.parse(init.body);
    calls.push([url, method]);
    const h = handlers[url];
    if (!h) throw new Error('refused');
    return { json: async () => h(method) };
  };
  return { calls, fetchImpl };
}

test('skips a node that is off and one on another chain, then remembers the answer', async () => {
  const net = fakeNet({
    'http://b': () => ({ result: '0x1' }),
    'http://c': (m) => (m === 'eth_chainId' ? { result: '0x1e64' } : { result: '0x10' }),
  });
  const rpc = new Rpc(['http://a', 'http://b', 'http://c'], { fetchImpl: net.fetchImpl });
  assert.equal(await rpc.call('eth_blockNumber'), '0x10');
  assert.equal(rpc.current, 'http://c');
  await rpc.call('eth_blockNumber');
  assert.equal(net.calls.filter(([, m]) => m === 'eth_chainId').length, 3);
});

test('backs off failing nodes and reports when none answers', async () => {
  let t = 0;
  const net = fakeNet({});
  const rpc = new Rpc(['http://a'], { fetchImpl: net.fetchImpl, now: () => t });
  await assert.rejects(rpc.call('eth_blockNumber'), (e) => e.code === 4900);
  await assert.rejects(rpc.call('eth_blockNumber'), (e) => e.code === 4900);
  assert.equal(net.calls.length, 1, 'second call inside the 5 s backoff does not probe');
  t = 5_000;
  await assert.rejects(rpc.call('eth_blockNumber'));
  assert.equal(net.calls.length, 2);
  assert.equal(rpc.backoff.get('http://a').delay, 10_000);
});

test('node errors keep their code', async () => {
  const net = fakeNet({ 'http://a': (m) => (m === 'eth_chainId' ? { result: '0x1e64' } : { error: { code: -32000, message: 'nonce too low' } }) });
  const rpc = new Rpc(['http://a'], { fetchImpl: net.fetchImpl });
  await assert.rejects(rpc.call('aether_sendTransaction', [{}]), (e) => e.code === -32000 && /nonce/.test(e.message));
});

test('callAny finds the node that runs a node-local service', async () => {
  const net = fakeNet({
    'http://a': () => ({ error: { code: -32601, message: 'this node does not run the faucet' } }),
    'http://c': () => ({ result: { hash: '0x1' } }),
  });
  const rpc = new Rpc(['http://a', 'http://b', 'http://c'], { fetchImpl: net.fetchImpl });
  assert.deepEqual(await rpc.callAny('aether_faucet', ['0x0']), { hash: '0x1' });
  const other = new Rpc(['http://x'], { fetchImpl: fakeNet({ 'http://x': () => ({ error: { code: -32000, message: 'rate limited' } }) }).fetchImpl });
  await assert.rejects(other.callAny('aether_faucet', ['0x0']), /rate limited/);
});

test('a method an older node lacks is served by another node', async () => {
  const net = fakeNet({
    'http://old': (m) => (m === 'eth_chainId' ? { result: '0x1e64' } : { error: { code: -32601, message: 'method not found: eth_call' } }),
    'http://new': () => ({ result: '0xbeef' }),
  });
  const rpc = new Rpc(['http://old', 'http://new'], { fetchImpl: net.fetchImpl });
  assert.equal(await rpc.call('eth_call', [{}, 'latest']), '0xbeef');
});
