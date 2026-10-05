import test from 'node:test';
import assert from 'node:assert/strict';
import { describeHistory, linkedAddress, mergeHistory } from '../src/lib/history.js';

const own = '0x' + '12'.repeat(20);
const other = '0x' + '34'.repeat(20);
const router = '0x' + '56'.repeat(20);
const token = '0x' + '78'.repeat(20);
const base = { address: own, height: 7, tx_index: 0, tx_hash: '0xabc', timestamp_ms: 7000,
  direction: 'in', kind: 'native_transfer', from: other, to: own, value_wei: '5000000000000000000',
  method: null, success: true, tokens: [] };

test('native receive and router swap are readable, with token identity', () => {
  assert.match(describeHistory(base).title, /Received 5 DBLN from/);
  assert.match(describeHistory(base, { ticker: 'AETH' }).title, /Received 5 AETH from/);
  const swap = { ...base, address: own, from: own, to: router, kind: 'contract_call', method: '0xac344b4d',
    value_wei: '10000000000000000000', tokens: [{ token, from: router, to: own, amount: '250000000000000000000' }],
    pair_swaps: [{ pair: router, amount0_in: '10', amount1_in: '0', amount0_out: '0', amount1_out: '250' }] };
  const title = describeHistory(swap, { sources: { router }, catalog: { [token]: { symbol: 'NEB', decimals: 18 } } }).title;
  assert.match(title, /Swapped 10 DBLN → 250 NEB · 0x7878…7878/);
});

test('node finality replaces a local pending row once per hash', () => {
  const local = [{ hash: '0xABC', title: 'Send', state: 'pending', at: 1 }];
  const chain = [describeHistory(base)];
  assert.equal(mergeHistory(local, chain).length, 1);
  assert.equal(mergeHistory(local, chain)[0].source, 'node');
  assert.equal(mergeHistory(local, chain)[0].state, 'done');
});

test('linked wallets retain one finalized row per hash and address', () => {
  const sender = describeHistory({ ...base, address: other, direction: 'out', kind: 'native_transfer', from: other });
  const recipient = describeHistory(base);
  const pending = { hash: '0xABC', owner: own, title: 'Pending', state: 'pending', at: 1 };
  const merged = mergeHistory([pending], [sender, recipient, { ...recipient, owner: own.toUpperCase() }]);
  assert.equal(merged.length, 2);
  assert.deepEqual(new Set(merged.map((item) => item.owner.toLowerCase())), new Set([own, other]));
  assert.equal(merged.find((item) => item.owner === own).state, 'done');
});

test('linked wallets reject duplicate and malformed addresses', () => {
  assert.equal(linkedAddress(other.toUpperCase().replace('0X', '0x'), own), other);
  assert.throws(() => linkedAddress(own, own));
  assert.throws(() => linkedAddress('0x123', own));
  assert.throws(() => linkedAddress(other, own, [other]));
});

test('zero allowance is a revoke, and unknown calls retain token deltas', () => {
  const revoke = { ...base, kind: 'contract_call', from: own, to: token, method: '0x095ea7b3', approval_amount: '0', approval_spender: other };
  assert.match(describeHistory(revoke).title, /^Revoked token/);
  const unknown = { ...revoke, method: '0xdeadbeef', approval_amount: null,
    tokens: [{ token, from: own, to: other, amount: '1000000000000000000' }] };
  assert.match(describeHistory(unknown).title, /method 0xdeadbeef.*−1000000000000000000 base units Token/);
});
