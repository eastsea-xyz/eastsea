import test from 'node:test';
import assert from 'node:assert/strict';
import { Rpc } from '../src/lib/rpc.js';
import { Wallet } from '../src/lib/wallet.js';

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
    'http://a': (m) => m === 'eth_chainId' ? { result: '0x1e64' } : ({ error: { code: -32601, message: 'this node does not run the faucet' } }),
    'http://c': (m) => m === 'eth_chainId' ? { result: '0x1e64' } : ({ result: { hash: '0x1' } }),
  });
  const rpc = new Rpc(['http://a', 'http://b', 'http://c'], { fetchImpl: net.fetchImpl });
  assert.deepEqual(await rpc.callAny('aether_faucet', ['0x0']), { hash: '0x1' });
  const other = new Rpc(['http://x'], { fetchImpl: fakeNet({ 'http://x': (m) => m === 'eth_chainId' ? { result: '0x1e64' } : ({ error: { code: -32000, message: 'rate limited' } }) }).fetchImpl });
  await assert.rejects(other.callAny('aether_faucet', ['0x0']), /rate limited/);
});

test('switching chains forgets the old endpoint and refuses its faucet', async () => {
  const net = fakeNet({
    'http://default': () => ({ result: '0x1e64' }),
    'http://dev': (m) => ({ result: m === 'eth_chainId' ? '0x1e61' : { hash: '0x1' } }),
  });
  const rpc = new Rpc(['http://default'], { fetchImpl: net.fetchImpl });
  await rpc.endpoint();
  rpc.setChain(7777, ['http://dev']);
  assert.equal(rpc.current, null);
  assert.equal(await rpc.endpoint(), 'http://dev');
  assert.deepEqual(await rpc.callAny('aether_faucet'), { hash: '0x1' });
});

test('a pending nonce belongs only to its chain', async () => {
  const rpc = { chainId: 7780, call: async () => '0x2' };
  const wallet = new Wallet({ wasm: null, rpc, vault: null });
  wallet.lastNonce = { address: '0xabc', nonce: 8, chainId: 7780 };
  assert.equal(await wallet.nonceFor('0xabc'), 9);
  rpc.chainId = 7777;
  assert.equal(await wallet.nonceFor('0xabc'), 2);
});

test('a method an older node lacks is served by another node', async () => {
  const net = fakeNet({
    'http://old': (m) => (m === 'eth_chainId' ? { result: '0x1e64' } : { error: { code: -32601, message: 'method not found: eth_call' } }),
    'http://new': (m) => ({ result: m === 'eth_chainId' ? '0x1e64' : '0xbeef' }),
  });
  const rpc = new Rpc(['http://old', 'http://new'], { fetchImpl: net.fetchImpl });
  assert.equal(await rpc.call('eth_call', [{}, 'latest']), '0xbeef');
});

test('callAgreed answers only when every node that answers agrees', async () => {
  const net = fakeNet({
    'http://a': (m) => (m === 'eth_chainId' ? { result: '0x1e64' } : { result: '0xaa' }),
    'http://b': (m) => (m === 'eth_chainId' ? { result: '0x1e64' } : { result: '0xaa' }),
    'http://off': () => { throw new Error('refused'); },
    'http://other': (m) => (m === 'eth_chainId' ? { result: '0x1' } : { result: '0xaa' }),
  });
  const rpc = new Rpc(['http://a', 'http://b', 'http://off', 'http://other'], { fetchImpl: net.fetchImpl });
  assert.deepEqual(await rpc.callAgreed('eth_call', [{}]), { result: '0xaa', sources: 2 });
});

test('callAgreed reports a disagreement instead of choosing an answer', async () => {
  const net = fakeNet({
    'http://a': (m) => (m === 'eth_chainId' ? { result: '0x1e64' } : { result: '0xaa' }),
    'http://b': (m) => (m === 'eth_chainId' ? { result: '0x1e64' } : { result: '0xbb' }),
  });
  const rpc = new Rpc(['http://a', 'http://b'], { fetchImpl: net.fetchImpl });
  await assert.rejects(rpc.callAgreed('eth_call', [{}]), (e) => e.disagreed === true && /agree/.test(e.message));
});

test('callAgreed counts the endpoints that answered and fails when none did', async () => {
  const net = fakeNet({
    'http://a': (m) => (m === 'eth_chainId' ? { result: '0x1e64' } : { result: '0xaa' }),
    'http://off': () => { throw new Error('refused'); },
  });
  const rpc = new Rpc(['http://a', 'http://off'], { fetchImpl: net.fetchImpl });
  assert.deepEqual(await rpc.callAgreed('eth_call', [{}]), { result: '0xaa', sources: 1 });
  const none = new Rpc(['http://off'], { fetchImpl: net.fetchImpl });
  await assert.rejects(none.callAgreed('eth_call', [{}]), (e) => e.code === 4900);
});
