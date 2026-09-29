import test from 'node:test';
import assert from 'node:assert/strict';
import { normalizeTx, describeCall, originAllowed, MAX_GAS } from '../src/lib/methods.js';

const TO = '0x00000000000000000000000000000000000000aa';

test('EIP-1193 transaction requests normalize to the builder format', () => {
  assert.deepEqual(normalizeTx({ to: TO, value: '0xde0b6b3a7640000', data: '0xA9059CBB', gas: '0x5208' }), { to: TO, value_wei: '1000000000000000000', data: '0xa9059cbb', gas: 21000 });
  assert.deepEqual(normalizeTx({ to: TO }), { to: TO, value_wei: '0', data: '0x', gas: 0 });
  assert.deepEqual(normalizeTx({ data: '0x6000' }), { to: '', value_wei: '0', data: '0x6000', gas: 0 });
});

test('malformed requests are refused', () => {
  assert.throws(() => normalizeTx(null));
  assert.throws(() => normalizeTx({ to: '0x1234' }), /address/);
  assert.throws(() => normalizeTx({ to: TO, data: '0x123' }), /hex/);
  assert.throws(() => normalizeTx({ to: TO, value: -1 }), /value/);
  assert.throws(() => normalizeTx({ to: TO, gas: MAX_GAS + 1 }), /cap/);
  assert.throws(() => normalizeTx({}), /creation/);
});

test('known selectors are named for the approval screen', () => {
  assert.equal(describeCall({ to: TO, data: '0x' }), 'Send DBLN');
  assert.equal(describeCall({ to: TO, data: '0x38ed1739aa' }), 'Swap tokens');
  assert.equal(describeCall({ to: TO, data: '0xcce7ec13' }), 'Buy on the launch curve');
  assert.equal(describeCall({ to: TO, data: '0x5cf66fe1' }), 'Buy with DBLN (graduated pool)');
  assert.match(describeCall({ to: '', data: '0x6000' }), /Deploy/);
  assert.match(describeCall({ to: TO, data: '0x12345678' }), /0x12345678/);
});

test('only https and this computer may connect', () => {
  assert.ok(originAllowed('https://dex.example'));
  assert.ok(originAllowed('http://localhost:8080'));
  assert.ok(originAllowed('http://127.0.0.1:8081'));
  assert.ok(!originAllowed('http://evil.example'));
  assert.ok(!originAllowed('file:///tmp/x.html'));
  assert.ok(!originAllowed('null'));
});
