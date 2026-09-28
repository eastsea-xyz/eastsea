// Units for the ABI word parsing and log decoding helpers (js/abi.js).

import test from 'node:test';
import assert from 'node:assert/strict';
import {
  APPROVAL_TOPIC, SEL, TRANSFER_TOPIC, addressAt, call, decodeApproval, decodeTransfer,
  revertReason, stringAt, topicAddress, uint64At, uintAt, wordAddress, wordUint,
} from '../js/abi.js';

const W = (hex) => hex.padStart(64, '0');
const ADDR = '0x1234567890abcdef1234567890abcdef12345678';

test('word builders', () => {
  assert.equal(wordAddress(ADDR), W('1234567890abcdef1234567890abcdef12345678'));
  assert.equal(wordAddress('0xABCdef0000000000000000000000000000000099'), W('abcdef0000000000000000000000000000000099'));
  assert.equal(wordUint(0), W('0'));
  assert.equal(wordUint(255), W('ff'));
  assert.equal(wordUint(1n), W('1'));
  assert.equal(call(SEL.balanceOf, wordAddress(ADDR)), `0x${SEL.balanceOf}${W('1234567890abcdef1234567890abcdef12345678')}`);
});

test('word readers take only whole 32-byte answers', () => {
  const data = `0x${W('2a')}${W(ADDR.slice(2))}${W('ff')}`;
  assert.equal(uintAt(data, 0), 42n);
  assert.equal(addressAt(data, 1), ADDR);
  assert.equal(uintAt(data, 2), 255n);
  assert.throws(() => uintAt('0x1234', 0)); // not whole words
  assert.throws(() => uintAt('0xzz' + '0'.repeat(62), 0)); // not hex
  assert.throws(() => uintAt('0x', 0)); // empty
  assert.throws(() => uintAt(data, 3)); // past the end
});

test('uint64At rejects words with high bytes set', () => {
  assert.equal(uint64At(`0x${W('2a')}`, 0), 42);
  assert.throws(() => uint64At(`0x${'f'.repeat(64)}`, 0));
});

test('stringAt decodes a dynamic string with offset and length', () => {
  // offset 0x20, length 5, "Aether" cut to 5 bytes: "Aethe"
  const bytes = '416574686572'.slice(0, 10); // "Aethe"
  const data = `0x${W('20')}${W('5')}${bytes.padEnd(64, '0')}`;
  assert.equal(stringAt(data), 'Aethe');
  assert.throws(() => stringAt(`0x${W('40')}${W('5')}${bytes}`)); // offset past the data
});

test('event topics match the canonical signatures', () => {
  // keccak256("Transfer(address,address,uint256)")
  assert.equal(TRANSFER_TOPIC, '0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef');
  // keccak256("Approval(address,address,uint256)")
  assert.equal(APPROVAL_TOPIC, '0x8c5be1e5ebec7d5bd14f71427d1e84f3dd0314c0f7b2291e5b200ac8c7c3b925');
});

test('topicAddress takes the low 20 bytes, lowercased', () => {
  assert.equal(topicAddress(`0x${W('1234567890abcdef1234567890abcdef12345678')}`), ADDR);
  assert.equal(topicAddress(ADDR), ADDR); // already an address-shaped value
});

test('decodeTransfer reads a real-shaped log and rejects others', () => {
  const from = '0x' + '11'.repeat(20);
  const to = '0x' + '22'.repeat(20);
  const log = { topics: [TRANSFER_TOPIC, W(from.slice(2)), W(to.slice(2))], data: `0x${W('bc614e')}` };
  assert.deepEqual(decodeTransfer(log), { from, to, value: 0xbc614en }); // 12,345,678
  assert.equal(decodeTransfer({ ...log, topics: [APPROVAL_TOPIC, ...log.topics.slice(1)] }), null);
  assert.equal(decodeTransfer({ ...log, topics: [TRANSFER_TOPIC, W(from.slice(2))] }), null); // too few topics
  assert.equal(decodeTransfer({ ...log, data: '0x1234' }), null); // malformed data does not throw
  assert.equal(decodeTransfer(null), null);
});

test('decodeApproval reads owner, spender and value', () => {
  const owner = '0x' + 'aa'.repeat(20);
  const spender = '0x' + 'bb'.repeat(20);
  const log = { topics: [APPROVAL_TOPIC, W(owner.slice(2)), W(spender.slice(2))], data: `0x${W('64')}` };
  assert.deepEqual(decodeApproval(log), { owner, spender, value: 100n });
  assert.equal(decodeApproval({ ...log, topics: [TRANSFER_TOPIC, ...log.topics.slice(1)] }), null);
});

test('revertReason decodes Error(string) and passes other messages through', () => {
  // Error("nope"): offset 32, length 4, "nope"
  const body = `08c379a0${W('20')}${W('4')}${'6e6f7065'.padEnd(64, '0')}`;
  assert.equal(revertReason(`execution reverted: 0x${body}`), '"nope"');
  assert.equal(revertReason('0x' + body), '"nope"');
  assert.equal(revertReason('some other error'), 'some other error');
  assert.equal(revertReason(''), '');
  // a truncated Error(string) stays as the raw message
  const short = 'execution reverted: 0x08c379a0' + '00'.repeat(10);
  assert.equal(revertReason(short), short);
});
