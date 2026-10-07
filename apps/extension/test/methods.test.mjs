import test from 'node:test';
import assert from 'node:assert/strict';
import { normalizeTx, withTransferGas, CODE_RECIPIENT_TRANSFER_GAS, describeCall, originAllowed, MAX_GAS } from '../src/lib/methods.js';

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
  // The ticker the caller passes names the coin.
  assert.equal(describeCall({ to: TO, data: '0x' }, { ticker: 'DBLN' }), 'Send DBLN');
  assert.equal(describeCall({ to: TO, data: '0xd0e30db0' }, { ticker: 'DBLN' }), 'Wrap DBLN');
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

test('a plain transfer to code gets more than the 21,000 default (live run 2026-10-06)', () => {
  const plain = normalizeTx({ to: TO, value: '0x1' });
  assert.equal(plain.gas, 0, 'unset: the builder would sign 21,000');
  assert.equal(withTransferGas(plain, '0x').gas, 0, 'an ordinary account keeps the default');
  assert.equal(withTransferGas(plain, '0x6080604052').gas, CODE_RECIPIENT_TRANSFER_GAS, 'a contract receive()');
  const delegated = '0xef0100' + '77'.repeat(20);
  assert.equal(withTransferGas(plain, delegated).gas, CODE_RECIPIENT_TRANSFER_GAS, 'a 7702-delegated account');
  assert.equal(withTransferGas({ ...plain, gas: 30_000 }, '0x60').gas, 30_000, "a page's explicit gas wins");
  const call = normalizeTx({ to: TO, data: '0xa9059cbb' });
  assert.equal(withTransferGas(call, '0x60').gas, 0, 'calls keep the builder default');
  assert.equal(withTransferGas(plain, undefined).gas, 0, 'no answer: unchanged');
});
