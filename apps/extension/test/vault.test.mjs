import test from 'node:test';
import assert from 'node:assert/strict';
import { Vault, hex } from '../src/lib/vault.js';
import { ADD_COINS_FIRST, Wallet, checkPaidStateBalance } from '../src/lib/wallet.js';
import { loadWasm, memoryArea } from './helpers.mjs';

async function setup(now = () => Date.now()) {
  const wasm = await loadWasm();
  const local = memoryArea();
  const session = memoryArea();
  const vault = new Vault({ local, session, addressOf: wasm.accountAddress, now });
  return { wasm, local, session, vault };
}

test('create, lock, unlock; the stored vault holds no plaintext key', async () => {
  const { vault, local } = await setup();
  const address = await vault.create('correct horse');
  assert.match(address, /^0x[0-9a-fA-F]{40}$/);
  const stored = JSON.stringify(local.map.get('vault'));
  assert.ok(!stored.includes('"d"'), 'no JWK private field at rest');
  await vault.lock();
  assert.equal(await vault.unlocked(), false);
  await assert.rejects(vault.sign(new Uint8Array([1])), /locked/);
  await assert.rejects(vault.unlock('wrong password'), /Wrong password/);
  assert.equal(await vault.unlock('correct horse'), address);
  assert.equal(await vault.unlocked(), true);
});

test('short passwords are refused', async () => {
  const { vault } = await setup();
  await assert.rejects(vault.create('short'), /8 characters/);
});

test('the lock timer expires the session', async () => {
  let t = 1_000_000;
  const { vault } = await setup(() => t);
  await vault.create('correct horse');
  t += 31 * 60_000;
  assert.equal(await vault.unlocked(), false);
});

test('an imported key gives the same address, and the backup round-trips', async () => {
  const { vault, wasm } = await setup();
  const secret = '07'.repeat(32);
  const a = await vault.importSecret(secret, 'correct horse', wasm.publicKeyFromSecret);
  assert.equal(await vault.revealSecret('correct horse'), secret);
  const again = await setup();
  assert.equal(await again.vault.importSecret(`0x${secret}`, 'another pass', wasm.publicKeyFromSecret), a);
  await assert.rejects(vault.importSecret('00'.repeat(32), 'correct horse', wasm.publicKeyFromSecret));
});

test('a WebCrypto signature over the prepared envelope passes the chain check', async () => {
  const { vault, wasm } = await setup();
  await vault.create('correct horse');
  const info = await vault.info();
  const status = { chain_id: 7780, base_fee: { exec: '0', prove: '0' } };
  const tx = { to: '0x00000000000000000000000000000000000000aa', value_wei: '5', data: '0x', gas: 0 };
  const sent = [];
  const rpc = {
    chainId: 7780,
    call: async (m, p) => {
      if (m === 'aether_status') return status;
      if (m === 'eth_getTransactionCount') return '0x4';
      if (m === 'aether_sendTransaction') { sent.push(p[0]); return { hash: '0xabc' }; }
      throw new Error(m);
    },
  };
  const w = new Wallet({ wasm, rpc, vault });
  // WebCrypto can return high-s; attach normalizes and verifies, so repeat a few times.
  for (let i = 0; i < 6; i += 1) assert.equal(await w.send(tx), '0xabc');
  assert.equal(sent.length, 6);
  assert.deepEqual(sent.map((e) => e.header.nonce), [4, 5, 6, 7, 8, 9], 'quick sends do not reuse a nonce');
  assert.equal(sent[0].header.sender.toLowerCase(), info.address.toLowerCase());
  const sig = hex.dec(sent[0].signature);
  assert.equal(sig.length, 64 + 33, 'r‖s plus the compressed key');
});

test('overlapping sends run one at a time and get distinct nonces', async () => {
  const { vault, wasm } = await setup();
  await vault.create('correct horse');
  const status = { chain_id: 7780, base_fee: { exec: '0', prove: '0' } };
  const sent = [];
  let statusCalls = 0;
  const rpc = {
    chainId: 7780,
    call: async (m, p) => {
      if (m === 'aether_status') { statusCalls += 1; return status; }
      if (m === 'eth_getTransactionCount') { await new Promise((r) => setTimeout(r, 20)); return '0x0'; }
      if (m === 'aether_sendTransaction') { sent.push(p[0]); return { hash: `0x${sent.length}` }; }
      // A funded account (G2: only a zero balance sends with no tip).
      if (m === 'eth_getBalance') return '0xde0b6b3a7640000';
      throw new Error(m);
    },
  };
  const w = new Wallet({ wasm, rpc, vault });
  const tx = { to: '0x00000000000000000000000000000000000000aa', value_wei: '1', data: '0x', gas: 0 };
  const shown = { chain_id: 7780, base_fee: { exec: '7', prove: '0' } };
  await Promise.all([w.send(tx, { status: shown }), w.send(tx), w.send(tx)]);
  assert.deepEqual(sent.map((e) => e.header.nonce), [0, 1, 2]);
  assert.equal(statusCalls, 2, 'the send with a shown snapshot does not fetch another');
  assert.equal(String(sent[0].header.max_fee.exec), String(7n * 2n + 1_000_000_000n), 'signed with the fee the user saw');
});

// A6-2: on a paid-state chain a zero balance signs a state budget of 0, which
// the chain rejects, so the wallet refuses before asking the vault to sign.
async function paidStateSend(chainId, balanceHex) {
  const { vault, wasm } = await setup();
  await vault.create('correct horse');
  const status = { chain_id: chainId, base_fee: { exec: '0', state: '1000000000000', prove: '0' } };
  const sent = [];
  let signed = 0;
  const signing = { info: () => vault.info(), sign: (m) => { signed += 1; return vault.sign(m); } };
  const rpc = {
    chainId,
    call: async (m, p) => {
      if (m === 'aether_status') return status;
      if (m === 'eth_getTransactionCount') return '0x0';
      if (m === 'eth_getBalance') { if (balanceHex === null) throw new Error('node down'); return balanceHex; }
      if (m === 'aether_sendTransaction') { sent.push(p[0]); return { hash: '0xabc' }; }
      throw new Error(m);
    },
  };
  const w = new Wallet({ wasm, rpc, vault: signing });
  const tx = { to: '0x00000000000000000000000000000000000000aa', value_wei: '0', data: '0x', gas: 0 };
  return { send: () => w.send(tx), sent, signs: () => signed };
}

test('paid-state chain: a zero balance is refused before signing', async () => {
  const s = await paidStateSend(7801, '0x0');
  await assert.rejects(s.send(), { message: ADD_COINS_FIRST });
  assert.equal(s.signs(), 0, 'nothing reached the vault');
  assert.equal(s.sent.length, 0);
  const unread = await paidStateSend(7801, null);
  await assert.rejects(unread.send(), /Could not read the balance/);
  assert.equal(unread.signs(), 0);
});

test('paid-state chain: a funded sender signs and sends', async () => {
  const s = await paidStateSend(7801, '0xde0b6b3a7640000');
  assert.equal(await s.send(), '0xabc');
  assert.equal(s.signs(), 1);
  assert.equal(s.sent.length, 1);
});

test('legacy 7780: a zero (or unreadable) balance still sends as before', async () => {
  for (const balance of ['0x0', null]) {
    const s = await paidStateSend(7780, balance);
    assert.equal(await s.send(), '0xabc');
    assert.equal(s.sent.length, 1);
  }
});

test('checkPaidStateBalance: only the legacy chain lets a zero balance through', () => {
  assert.doesNotThrow(() => checkPaidStateBalance(7780, 0n));
  assert.doesNotThrow(() => checkPaidStateBalance(7780, null));
  assert.doesNotThrow(() => checkPaidStateBalance(7801, 1n));
  assert.throws(() => checkPaidStateBalance(7801, 0n), { message: ADD_COINS_FIRST });
  assert.throws(() => checkPaidStateBalance('7801', null), /Could not read/);
});
