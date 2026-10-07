import test from 'node:test';
import assert from 'node:assert/strict';
import { aethToWei, weiToAeth, formatAeth, toBigInt } from '../src/lib/units.js';

test('DBLN and wei convert exactly both ways', () => {
  assert.equal(aethToWei('1.5'), 1_500_000_000_000_000_000n);
  assert.equal(aethToWei('.25'), 250_000_000_000_000_000n);
  assert.equal(aethToWei('0.000000000000000001'), 1n);
  assert.equal(weiToAeth(1_500_000_000_000_000_000n), '1.5');
  assert.equal(weiToAeth('0x1'), '0.000000000000000001');
  assert.equal(weiToAeth(0n), '0');
});

test('bad amounts are rejected', () => {
  for (const bad of ['', '.', 'abc', '1.2.3', '-1', '0.0000000000000000001']) assert.throws(() => aethToWei(bad), bad);
  assert.throws(() => toBigInt(-1));
  assert.throws(() => toBigInt('0xzz'));
});

test('display format groups thousands and trims', () => {
  assert.equal(formatAeth(aethToWei('1234567.123456')), '1,234,567.1234');
  assert.equal(formatAeth(aethToWei('2')), '2');
});
