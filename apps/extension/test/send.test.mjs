// The token-send helpers and the send-flow checks, mirroring the app's
// apps/wallet/Tests/token-send (same edge cases both sides).

import test from 'node:test';
import assert from 'node:assert/strict';
import { Brand } from '../src/lib/brand.js';
import { erc20TransferCalldata, formatTokenAmount, formatTokenAmountExact, parseTokenAmount } from '../src/lib/tokens.js';
import { addressRisk, isValidAddress, looksLikeOfficial, normalized, revertReason, splitHoldings, tokenLabel, tokenShort } from '../src/lib/safety.js';

test('parseTokenAmount: exact, any decimals, spaces trimmed, junk rejected', () => {
  assert.equal(parseTokenAmount('1.5', 18), 1500000000000000000n);
  assert.equal(parseTokenAmount('  1.5 \n', 18), 1500000000000000000n);
  assert.equal(parseTokenAmount('1.000000000000000001', 18), 1000000000000000001n);
  assert.equal(parseTokenAmount('0.000000000000000001', 18), 1n);
  assert.equal(parseTokenAmount('1234567', 0), 1234567n);
  assert.throws(() => parseTokenAmount('1234567.0', 0), /at most 0 decimals/);
  assert.throws(() => parseTokenAmount('1.000000000000000001', 17), /at most 17 decimals/);
  assert.equal(parseTokenAmount('0', 18), 0n);
  assert.equal(parseTokenAmount('007', 2), 700n);
  assert.equal(parseTokenAmount('.5', 1), 5n);
  assert.equal(parseTokenAmount('1.', 2), 100n);
  assert.throws(() => parseTokenAmount('', 18));
  assert.throws(() => parseTokenAmount('.', 18));
  assert.throws(() => parseTokenAmount('abc', 18));
  assert.throws(() => parseTokenAmount('1.2.3', 18));
  assert.throws(() => parseTokenAmount('-1', 18));
  assert.throws(() => parseTokenAmount('1,5', 18));
  assert.throws(() => parseTokenAmount('１', 18)); // full-width digit
});

test('formatTokenAmountExact: all digits, for the Max button', () => {
  assert.equal(formatTokenAmountExact('1000000000000000001', 18), '1.000000000000000001');
  assert.equal(formatTokenAmountExact('1234567', 0), '1234567');
  assert.equal(formatTokenAmountExact('1500000', 6), '1.500000');
  assert.equal(formatTokenAmountExact('0', 18), '0.000000000000000000');
  assert.equal(formatTokenAmount(1500000n, 6), '1.5'); // the 6-digit display one still caps
});

test('erc20TransferCalldata: selector + two padded words', () => {
  const rcpt = '0x00000000000000000000000000000000000000aA';
  assert.equal(erc20TransferCalldata(rcpt, 25000000000000000000n),
    `0xa9059cbb${'0'.repeat(24)}00000000000000000000000000000000000000aa${'0'.repeat(47)}15af1d78b58c40000`);
  assert.equal(erc20TransferCalldata(rcpt, 0n).length, 10 + 128);
  assert.equal(erc20TransferCalldata(rcpt, 10n ** 77n).length, 10 + 128); // 10^77 still fits
  assert.throws(() => erc20TransferCalldata(rcpt, 2n * 10n ** 77n), /uint256/);
  assert.throws(() => erc20TransferCalldata('0x1234', 1n), /not an address/);
});

test('addressRisk: first-4/last-4 poisoning and first sends', () => {
  const hist = ['0x1234567890abcdef1234567890abcdef12345678', '0xaabbccddeeff00112233445566778899aabbccdd'];
  let r = addressRisk('0x1234abcdef777777777777777777777777775678', hist);
  assert.equal(r.poisoningMatch, '0x1234567890abcdef1234567890abcdef12345678');
  assert.equal(r.firstSend, true);
  r = addressRisk('0x1234567890ABCDEF1234567890abcdef12345678', hist);
  assert.equal(r.poisoningMatch, null);
  assert.equal(r.firstSend, false); // the same address, different case
  r = addressRisk('0x1234567890abcdef1234567890abcdef12345679', hist);
  assert.equal(r.poisoningMatch, null); // only the suffix differs: new, not poisoned
  assert.equal(r.firstSend, true);
  r = addressRisk('0xabc4567890abcdef1234567890abcdef12345678', hist);
  assert.equal(r.poisoningMatch, null);
  r = addressRisk('0x1234', hist);
  assert.deepEqual(r, { poisoningMatch: null, firstSend: true });
  assert.equal(isValidAddress('0x1234567890AbCdEf1234567890aBcDeF12345678'), true);
  assert.equal(isValidAddress('1234567890abcdef1234567890abcdef12345678'), false);
  assert.equal(isValidAddress('0x1234567890abcdef1234567890abcdef1234567g'), false);
});

test('look-alike symbols: folded, edit distance 1, names containing an official symbol', () => {
  const official = [{ symbol: Brand.coinTicker, name: Brand.coinName }, { symbol: 'USDT', name: 'Tether Dollar' }];
  assert.equal(looksLikeOfficial({ symbol: 'DBLN', name: 'x' }, official), true);
  assert.equal(looksLikeOfficial({ symbol: 'DBL', name: '' }, official), true);
  assert.equal(looksLikeOfficial({ symbol: 'DBLNR', name: '' }, official), true);
  assert.equal(looksLikeOfficial({ symbol: 'WDBLN', name: '' }, official), true);
  assert.equal(looksLikeOfficial({ symbol: 'ĐBLN', name: '' }, official), true);
  assert.equal(looksLikeOfficial({ symbol: 'NEB', name: 'DBLNs' }, official), true);
  assert.equal(looksLikeOfficial({ symbol: 'NEB', name: 'Nebula' }, official), false);
  assert.equal(looksLikeOfficial({ symbol: 'NEBD', name: 'Nebula' }, official), false);
  assert.equal(looksLikeOfficial({ symbol: 'usdt', name: '' }, official), true);
  assert.equal(normalized('Æth​er '), 'aether'); // zero-width and spaces stripped
});

test('revertReason: Error(string) decoded, anything else kept', () => {
  const reason = revertReason('execution reverted: 0x08c379a0'
    + '0'.repeat(62) + '20' + '0'.repeat(63) + 'e'
    + '546f6f206c6974746c6520676173' + '0'.repeat(36));
  assert.equal(reason, '"Too little gas"');
  assert.equal(revertReason('execution reverted: 0x'), 'execution reverted: 0x');
  assert.equal(revertReason('node did not answer'), 'node did not answer');
});

test('labels: never a symbol alone', () => {
  assert.equal(tokenShort('0x8a9b000000000000000000000000000000000f41c'), '0x8a9b…f41c');
  assert.equal(tokenLabel({ symbol: 'NEB', address: '0x8a9b000000000000000000000000000000000f41c' }), 'NEB · 0x8a9b…f41c');
});

test('splitHoldings: own actions and official tokens are main-listed', () => {
  const holdings = [
    { token: { address: '0xofficial', symbol: 'A' } },
    { token: { address: '0xrandom', symbol: 'B' } },
    { token: { address: '0xtouched', symbol: 'C' } },
    { token: { address: '0xhidden', symbol: 'D' } },
    { token: { address: '0xshown', symbol: 'E' } },
  ];
  const { main, unverified } = splitHoldings(holdings, {
    touched: ['0xtouched'], official: ['0xofficial'], hidden: ['0xhidden'], shown: ['0xshown'],
  });
  assert.deepEqual(main.map((x) => x.token.symbol), ['A', 'C', 'E']);
  assert.deepEqual(unverified.map((x) => x.token.symbol), ['B', 'D']);
  assert.deepEqual(splitHoldings(holdings, { official: ['0XOFFICIAL'] }).main.map((x) => x.token.symbol), ['A']); // case-insensitive
});
