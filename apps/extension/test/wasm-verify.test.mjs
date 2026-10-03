import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { loadWasm } from './helpers.mjs';
import { Rpc } from '../src/lib/rpc.js';

const fixture = JSON.parse(await readFile(new URL('../../../crates/light/tests/fixtures/devnet4.json', import.meta.url)));
const network = {
  chain_id: 7777,
  identity: '8dc3275f2e956c661b3e16b2e0faaa0344b01bd0a73c49e27c7be51821ffd730e05155a67c60a87a64820325036aba9e00c892063ff519bbe6f0c3c05e638c3241114cfb74906f9f32568239f080e9eb33b4276146bcac77ddbe96138e2f30c5',
};
const status = { chain_id: 7777 };
const account = { address: fixture.address, height: fixture.height, state_root: fixture.state_root,
  balance: fixture.balance, nonce: 0, proof: fixture.proof };
const finalized = { height: fixture.height, block: fixture.anchor_block,
  finalization: fixture.anchor_finalization, links: [] };

test('packaged wasm verifies a captured certificate and account proof, then rejects tampering', async () => {
  const wasm = await loadWasm();
  const verify = (n = network, s = status, a = account, f = finalized, floor = 0n, now = 0n) =>
    JSON.parse(wasm.verifyAccount(JSON.stringify(n), JSON.stringify(s), JSON.stringify(a), JSON.stringify(f), fixture.address, floor, now));
  const checked = verify();
  assert.equal(checked.balance_wei, '4242');
  assert.equal(checked.certified_block, 6);
  assert.throws(() => verify(network, status, { ...account, proof: fixture.other_proof }), /proof/);
  assert.throws(() => verify(network, status, { ...account, balance: '0xffff' }), /fields differ/);
  assert.throws(() => verify(network, status, account, finalized, 7n), /never go back/);
  assert.throws(() => verify(network, { chain_id: 7780 }), /chain/);
  assert.throws(() => verify(network, status, account, { ...finalized, finalization: '00' }), /certificate/);
  assert.throws(() => verify(network, status, account, finalized, 0n, BigInt(checked.timestamp_ms) + 600_001n), /stale/);
});

test('RPC selection uses the packaged verifier before exposing a balance', async () => {
  const wasm = await loadWasm();
  const checked = JSON.parse(wasm.verifyAccount(JSON.stringify(network), JSON.stringify(status),
    JSON.stringify(account), JSON.stringify(finalized), fixture.address, 0n, 0n));
  const floor = new Map();
  const rpc = new Rpc(['https://bad', 'https://good'], {
    chainId: 7777, network, now: () => checked.timestamp_ms,
    floorStore: { get: async (key) => floor.get(key), set: async (key, value) => floor.set(key, value) },
    verifyAccount: wasm.verifyAccount,
    fetchImpl: async (url, init) => {
      const { method } = JSON.parse(init.body);
      const result = method === 'aether_getAccount' ? (url === 'https://bad' ? { ...account, balance: '0xffff' } : account)
        : method === 'aether_getFinalized' ? finalized : status;
      return { json: async () => ({ result }) };
    },
  });
  assert.equal(await rpc.call('eth_getBalance', [fixture.address]), '0x1092');
  assert.equal(rpc.backoff.get('https://bad').kind, 'lying');
  assert.equal(rpc.current, 'https://good');
  assert.equal(floor.get('verifiedHeight.7777'), 6);
});
