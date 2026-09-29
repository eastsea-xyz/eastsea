// Units for ERC-20 metadata and origin discovery (js/erc20.js), against a mock
// reader that answers eth_calls the way a node does.

import test from 'node:test';
import assert from 'node:assert/strict';
import {
  CAPS, looksLikeOfficial, officialAddresses, officialTokens, originBadge,
  parseTokenSources, tokenInfo, tokenOrigin, totalSupply,
} from '../js/erc20.js';
import { SEL } from '../js/abi.js';

const W = (hex) => String(hex).padStart(64, '0');
/** `A(0xaa)` -> '0xaaaa…' (20 repeated bytes): a real-shaped address. */
const A = (n) => `0x${n.toString(16).padStart(2, '0').repeat(20)}`;

/** A reader over `{ address: { selector: hex | (calldata) => hex } }`; anything
 * a contract does not answer throws, as a failed eth_call would. Answers may
 * carry their own 0x; the reader never doubles it. */
function mockRead(contracts) {
  return async (to, data) => {
    const table = contracts[String(to).toLowerCase()];
    const answer = table?.[String(data).slice(2, 10)];
    if (!answer) throw new Error('no contract at ' + to);
    const hex = typeof answer === 'function' ? answer(data) : answer;
    return String(hex).startsWith('0x') ? hex : `0x${hex}`;
  };
}

function listContract(items, { countSel, itemSel }) {
  return {
    [countSel]: W(items.length.toString(16)),
    [itemSel]: (data) => {
      const i = parseInt(String(data).slice(10, 74), 16); // the argument word, right after the selector
      if (!(i >= 0) || i >= items.length) throw new Error('out of range');
      return W(items[i].slice(2));
    },
  };
}

const token = (symbolHex, nameHex, decimals) => ({
  [SEL.decimals]: W((decimals ?? 18).toString(16)),
  [SEL.symbol]: `0x${W('20')}${W((symbolHex.length / 2).toString(16))}${symbolHex.padEnd(64, '0')}`,
  [SEL.name]: `0x${W('20')}${W((nameHex.length / 2).toString(16))}${nameHex.padEnd(64, '0')}`,
  [SEL.totalSupply]: W('bc614e'),
});

const hex = (s) => Buffer.from(s, 'utf8').toString('hex');

test('parseTokenSources finds the chain or says null', () => {
  const file = { chains: { 7780: { network: 'aether-testnet', seed: [A(1)] } } };
  assert.deepEqual(parseTokenSources(file, 7780), { network: 'aether-testnet', seed: [A(1)] });
  assert.equal(parseTokenSources(file, 7781), null);
  assert.equal(parseTokenSources('not json', 7780), null);
  assert.equal(parseTokenSources(null, 7780), null);
  assert.deepEqual(parseTokenSources({ chains: { 7780: {} } }, 7780), { seed: [] }); // seed defaults
});

test('officialAddresses folds in waeth, lowercased', () => {
  assert.deepEqual(officialAddresses({ seed: [A(1).toUpperCase()], waeth: A(2) }), [A(1), A(2)]);
  assert.deepEqual(officialAddresses(null), []);
});

test('tokenInfo reads decimals, symbol and name; rejects non-tokens', async () => {
  const reader = mockRead({ [A(1)]: token(hex('NEB'), hex('Nebula'), 18), [A(2)]: { [SEL.symbol]: token(hex('X'), hex('X'), 18)[SEL.symbol] } });
  assert.deepEqual(await tokenInfo(A(1), reader), { address: A(1), symbol: 'NEB', name: 'Nebula', decimals: 18 });
  assert.equal(await tokenInfo(A(2), reader), null); // no decimals call: not a token
  assert.equal(await tokenInfo(A(9), reader), null); // nothing there at all
  const absurd = mockRead({ [A(3)]: token(hex('X'), hex('X'), 78) });
  assert.equal(await tokenInfo(A(3), absurd), null); // >77 decimals is not a token
});

test('tokenInfo falls back to ??? when symbol or name misbehave', async () => {
  const reader = mockRead({ [A(1)]: { [SEL.decimals]: W('12'), [SEL.symbol]: '0x1234' } }); // malformed string answer
  assert.deepEqual(await tokenInfo(A(1), reader), { address: A(1), symbol: '???', name: '', decimals: 18 });
});

test('totalSupply reads or returns null', async () => {
  const reader = mockRead({ [A(1)]: token(hex('N'), hex('N'), 0) });
  assert.equal(await totalSupply(A(1), reader), 0xbc614en);
  assert.equal(await totalSupply(A(9), reader), null);
});

test('tokenOrigin walks the lists in the wallet\'s priority order', async () => {
  const LAUNCH = A(0xaa), DEX = A(0xbb), PAIR = A(0xcc);
  const inLaunch = A(1), inDex = A(2), inPool = A(3), nowhere = A(4), official = A(5);
  const sources = { launchpad: LAUNCH, tokenFactory: DEX, pairFactory: PAIR, seed: [official] };
  const contracts = {
    [LAUNCH]: listContract([inLaunch], { countSel: SEL.tokenCount, itemSel: SEL.tokens }),
    // the official token also joined the DEX list later; it stays official
    [DEX]: listContract([inLaunch, inDex, official], { countSel: SEL.allTokensLength, itemSel: SEL.allTokens }),
    [PAIR]: {
      [SEL.allPairsLength]: W('1'),
      [SEL.allPairs]: () => W(PAIR.slice(2)),
      [SEL.token0]: () => W(inPool.slice(2)),
      // token1 is unreadable on this pair; the scan carries on
    },
  };
  const reader = mockRead(contracts);
  assert.equal(await tokenOrigin(inLaunch, sources, reader), 'launchpad'); // launchpad outranks dex
  assert.equal(await tokenOrigin(inDex, sources, reader), 'dex');
  assert.equal(await tokenOrigin(inPool, sources, reader), 'pool');
  assert.equal(await tokenOrigin(official, sources, reader), 'seed'); // the official list settles it, not discovery
  assert.equal(await tokenOrigin(nowhere, sources, reader), null);
});

test('tokenOrigin stops each list at its cap and tolerates missing contracts', async () => {
  const LAUNCH = A(0xaa);
  const far = A(7); // listed at index 2, past a cap of 2
  const sources = { launchpad: LAUNCH, tokenFactory: null, pairFactory: null, seed: [] };
  const reader = mockRead({ [LAUNCH]: listContract([A(1), A(2), far], { countSel: SEL.tokenCount, itemSel: SEL.tokens }) });
  assert.equal(await tokenOrigin(far, sources, reader, { launches: 2, factoryTokens: 2, pools: 2 }), null);
  assert.equal(await tokenOrigin(far, sources, reader, { launches: 3, factoryTokens: 2, pools: 2 }), 'launchpad');
  assert.equal(await tokenOrigin(A(9), {}, reader), null); // no lists at all
});

test('originBadge says what the wallet says', () => {
  assert.deepEqual(originBadge('seed'), { text: 'Official list', kind: 'good' });
  assert.deepEqual(originBadge('launchpad'), { text: 'Launchpad · unverified', kind: 'warn' });
  assert.deepEqual(originBadge('dex'), { text: 'DEX-listed · unverified', kind: 'warn' });
  assert.deepEqual(originBadge('pool'), { text: 'In a DEX pool · unverified', kind: 'warn' });
  assert.deepEqual(originBadge(null), { text: 'Not in any list · unverified', kind: 'warn' });
});

test('looksLikeOfficial catches equal, near and look-alike symbols', () => {
  const official = [{ symbol: 'AETH', name: 'Aether' }, { symbol: 'NEB', name: 'Nebula' }];
  assert.equal(looksLikeOfficial({ symbol: 'NEB' }, official), true); // equal
  assert.equal(looksLikeOfficial({ symbol: 'NE8' }, official), true); // one edit
  assert.equal(looksLikeOfficial({ symbol: 'ÆTH' }, official), true); // folds to aeth
  assert.equal(looksLikeOfficial({ symbol: 'nebula' }, official), true); // symbol vs official name
  assert.equal(looksLikeOfficial({ symbol: 'XXXX' }, official), false);
  assert.equal(looksLikeOfficial({ symbol: 'weth' }, official), true); // genuinely one edit from AETH — the wallet warns
  assert.equal(looksLikeOfficial({ symbol: 'dai' }, official), false);
  assert.equal(looksLikeOfficial({}, official), false);
});

test('officialTokens reads the seed list and skips the unreadable', async () => {
  const reader = mockRead({ [A(1)]: token(hex('NEB'), hex('Nebula'), 18) });
  assert.deepEqual(await officialTokens({ seed: [A(1), A(2)], waeth: A(3) }, reader),
    [{ address: A(1), symbol: 'NEB', name: 'Nebula', decimals: 18 }]);
});

test('caps are the wallet\'s caps', () => {
  assert.deepEqual(CAPS, { factoryTokens: 500, pools: 200, launches: 500 });
});
