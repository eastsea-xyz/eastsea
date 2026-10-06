// Units for the amount and time helpers (js/format.js).

import test from 'node:test';
import assert from 'node:assert/strict';
import { coinTicker, formatAeth, formatInt, formatRate, formatTokenAmount, localTime, notIncludedText, shortHex, timeAgo, toBigInt, txRate, weiToAeth } from '../js/format.js';

test('toBigInt takes hex, decimal, number and bigint; rejects the rest', () => {
  assert.equal(toBigInt('0x1a'), 26n);
  assert.equal(toBigInt('26'), 26n);
  assert.equal(toBigInt(26), 26n);
  assert.equal(toBigInt(26n), 26n);
  assert.equal(toBigInt('0x0'), 0n);
  assert.throws(() => toBigInt('1.5'));
  assert.throws(() => toBigInt('-1'));
  assert.throws(() => toBigInt('0x'));
  assert.throws(() => toBigInt({}));
});

test('weiToAeth is exact and drops trailing zeros', () => {
  assert.equal(weiToAeth(0), '0');
  assert.equal(weiToAeth(1), '0.000000000000000001');
  assert.equal(weiToAeth(10n ** 18n), '1');
  assert.equal(weiToAeth(15n * 10n ** 17n), '1.5');
  assert.equal(weiToAeth(10n ** 17n), '0.1');
  assert.equal(weiToAeth('0x4599ecb2e063c40'), '0.3134562445666704'); // 0x hex, the shape aether_getAccount returns
});

test('the coin ticker follows the chain (the legacy testnet kept AETH)', () => {
  assert.equal(coinTicker(7780), 'AETH');
  assert.equal(coinTicker(7777), 'DBLN');
  assert.equal(coinTicker(null), 'DBLN'); // before the node answers
});

test('formatAeth groups thousands and trims to the asked digits', () => {
  assert.equal(formatAeth(1234n * 10n ** 18n + 5n * 10n ** 14n), '1,234.0005');
  assert.equal(formatAeth(1234n * 10n ** 18n), '1,234');
  assert.equal(formatAeth(1n, 0), '0');
  assert.equal(formatAeth(123456789n * 10n ** 18n, 0), '123,456,789');
});

test('formatTokenAmount keeps at most 6 fraction digits and never shows a bare 0.x as 0', () => {
  assert.equal(formatTokenAmount(0n, 18), '0');
  assert.equal(formatTokenAmount(1n, 0), '1');
  assert.equal(formatTokenAmount(123456789n, 18), '<0.000001');
  assert.equal(formatTokenAmount(15005n, 4), '1.5005');
  assert.equal(formatTokenAmount(5n * 10n ** 6n, 6), '5');
  assert.equal(formatTokenAmount(5n * 10n ** 6n + 12n, 6), '5.000012');
  assert.equal(formatTokenAmount(1234n * 10n ** 6n + 5n, 6), '1234.000005');
});

test('formatInt groups', () => {
  assert.equal(formatInt(0), '0');
  assert.equal(formatInt(1234567), '1,234,567');
});

test('shortHex shortens only long values', () => {
  assert.equal(shortHex('0x1234'), '0x1234');
  assert.equal(shortHex(`0x${'ab'.repeat(32)}`, 6, 4), '0xabab…abab');
  assert.equal(shortHex(`0x${'12'.repeat(20)}`, 6, 4), '0x1212…1212');
  assert.equal(shortHex(null), ''); // missing values shorten to nothing, not "null"
});

test('timeAgo buckets', () => {
  const now = 1_700_000_000_000;
  assert.equal(timeAgo(now, now), 'just now');
  assert.equal(timeAgo(now - 4_000, now), 'just now');
  assert.equal(timeAgo(now - 30_000, now), '30s ago');
  assert.equal(timeAgo(now - 90_000, now), '2m ago');
  assert.equal(timeAgo(now - 3 * 3600_000, now), '3h ago');
  assert.equal(timeAgo(now - 50 * 3600_000, now), '2d ago');
  assert.equal(timeAgo(now + 60_000, now), 'just now'); // clock skew clamps
});

test('localTime formats as local wall clock', () => {
  const ms = Date.UTC(2026, 8, 29, 5, 6, 7); // 2026-09-29 05:06:07 UTC
  const s = localTime(ms);
  assert.match(s, /^2026-09-29 \d{2}:06:07$/); // minutes and seconds survive any zone
});

test('txRate spans the window and ignores junk', () => {
  const b = (height, ts, n) => ({ height, timestamp_ms: ts, txs: Array(n).fill('0x') });
  const rate = txRate([b(3, 30_000, 4), b(2, 20_000, 3), b(1, 10_000, 3)]);
  assert.deepEqual(rate, { txs: 10, seconds: 20, perSec: 0.5 });
  assert.equal(txRate([b(1, 10_000, 2)]), null);
  assert.equal(txRate([b(1, 10_000, 2), b(2, 10_000, 2)]), null); // one instant
  assert.equal(txRate([]), null);
  assert.equal(txRate(null), null);
  assert.equal(txRate([{ nope: 1 }, b(1, 10_000, 1)]), null); // one usable block
});

test('formatRate rounds to two decimals and passes null through', () => {
  assert.equal(formatRate(0.5), '0.5');
  assert.equal(formatRate(0.3333), '0.33');
  assert.equal(formatRate(123.4), '123.4');
  assert.equal(formatRate(123.6), '123.6');
  assert.equal(formatRate(null), '—');
});

test('notIncludedText says why a transaction is not in a block (bug #5)', () => {
  const stuck = notIncludedText({ kind: 'state_price_above_cap', cap: '2000000000000', price: '43000000000000', blocks: 1200 });
  assert.match(stuck, /state price/);
  assert.match(stuck, /1,200 blocks/);
  assert.equal(notIncludedText({ kind: 'nonce_gap', expected: 4 }), 'an earlier nonce of the sender (4) has not arrived');
  assert.match(notIncludedText({ kind: 'replaced' }), /same nonce/);
  assert.equal(notIncludedText({ kind: 'something_new' }), 'something new');
  assert.equal(notIncludedText(null), null);
  assert.equal(notIncludedText({}), null);
});
