// Audit R2-2 (docs/research/audit-2-2026-10-03.md): the only trusted
// denomination in the extension is the allowlist shipped in the build
// (lib/knownTokens.js) — two configured endpoints agreeing on a decimals
// answer is continuity, not a quorum of trust. These checks pin the table's
// shape (so a typo cannot quietly demote a real token to "unverified" or
// promote a stranger to "trusted") and keep it in sync with the deployment
// addresses the wallet already ships (token-sources.json).

import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { Brand, coinTicker, coinName } from '../src/lib/brand.js';
import { NATIVE_COINS, KNOWN_TOKENS, knownToken } from '../src/lib/knownTokens.js';

const ADDRESS = /^0x[0-9a-f]{40}$/;

test('every entry is a lowercase address with well-formed metadata', () => {
  for (const [chain, table] of Object.entries(KNOWN_TOKENS)) {
    assert.match(String(chain), /^\d+$/, `chain key ${chain} must be numeric`);
    for (const [addr, t] of Object.entries(table)) {
      assert.match(addr, ADDRESS, `${chain}:${addr} must be a lowercase 0x address`);
      assert.equal(typeof t.symbol, 'string');
      assert.ok(t.symbol.length > 0 && t.symbol.length <= 16, `${chain}:${addr} symbol length`);
      assert.equal(typeof t.name, 'string');
      assert.ok(t.name.length > 0 && t.name.length <= 64, `${chain}:${addr} name length`);
      assert.ok(Number.isSafeInteger(t.decimals) && t.decimals >= 0 && t.decimals <= 77, `${chain}:${addr} decimals`);
    }
  }
});

test('lookups ignore case and stay per chain', () => {
  const waeth = KNOWN_TOKENS[7780]['0xa2521982a17474cb2f8741c85de653b5282d72b0'];
  assert.ok(waeth);
  // The same address in any case resolves to the same entry (as a copy);
  // junk input and other chains resolve to null — without ever throwing.
  assert.deepEqual(knownToken(7780, '0XA2521982A17474CB2F8741C85DE653B5282D72B0'), waeth);
  assert.deepEqual(knownToken('7780', '0xA2521982a17474CB2F8741c85DE653b5282d72b0'), waeth);
  assert.deepEqual(knownToken(7780, '0xa2521982a17474cb2f8741c85de653b5282d72b0'.toUpperCase()), waeth);
  assert.equal(knownToken(7777, '0xa2521982a17474cb2f8741c85de653b5282d72b0'), null); // other chain
  assert.equal(knownToken(7780, '0x00000000000000000000000000000000000000ff'), null); // unknown
  assert.equal(knownToken(7780, ''), null);
  assert.equal(knownToken(7780, null), null);
  assert.equal(knownToken(7780, undefined), null);
  assert.equal(knownToken(null, '0xa2521982a17474cb2f8741c85de653b5282d72b0'), null);
});

test('the table is frozen, and lookups hand out copies', () => {
  assert.ok(Object.isFrozen(KNOWN_TOKENS));
  assert.ok(Object.isFrozen(KNOWN_TOKENS[7780]));
  for (const t of Object.values(KNOWN_TOKENS[7780])) assert.ok(Object.isFrozen(t));
  assert.ok(Object.isFrozen(NATIVE_COINS));
  // A copy: mutating a lookup result cannot corrupt the shipped table.
  const one = knownToken(7780, '0xa2521982a17474cb2f8741c85de653b5282d72b0');
  one.decimals = 9;
  assert.equal(knownToken(7780, '0xa2521982a17474cb2f8741c85de653b5282d72b0').decimals, 18);
});

test('the native entry follows the chain, not the node', () => {
  assert.equal(NATIVE_COINS[7780].symbol, 'DBLN');
  assert.equal(NATIVE_COINS[7780].name, 'Test Doubloon');
  assert.ok(Number.isSafeInteger(NATIVE_COINS[7780].decimals));
  // The per-chain brand agrees: DBLN on every chain, "Test Doubloon" on the
  // legacy testnet.
  assert.equal(NATIVE_COINS[7780].symbol, coinTicker(7780));
  assert.equal(NATIVE_COINS[7780].name, coinName(7780));
  assert.equal(coinTicker(7777), 'DBLN');
});

test('every address the wallet ships as a source token is on the list', async () => {
  // token-sources.json is bundled with the extension and lists the deployed
  // WAETH and seed tokens for the testnet. A listing the UI treats as
  // "official" must never fall back to unverified units.
  const sources = JSON.parse(readFileSync(new URL('../token-sources.json', import.meta.url), 'utf8'));
  for (const [chain, s] of Object.entries(sources.chains || {})) {
    const deployed = [s.waeth, ...(s.seed || [])].filter(Boolean);
    for (const address of deployed) {
      const known = knownToken(Number(chain), address);
      assert.ok(known, `${chain} ${address} (${s.network}) is deployed but not in knownTokens.js`);
      assert.ok(Number.isSafeInteger(known.decimals));
    }
  }
});

test('WAETH keeps its contract symbol but reads as wrapped test DBLN', async () => {
  const { displayTokenName, knownToken: kt } = await import('../src/lib/knownTokens.js');
  const waeth = '0xa2521982a17474cb2f8741c85de653b5282d72b0';
  assert.equal(kt(7780, waeth).symbol, 'WAETH');            // the contract's own
  assert.equal(displayTokenName(7780, waeth, 'Wrapped AETH'), 'Wrapped test DBLN');
  assert.equal(displayTokenName(7780, waeth.toUpperCase().replace('0X', '0x'), 'Wrapped AETH'), 'Wrapped test DBLN');
  assert.equal(displayTokenName(7780, '0x6bc5ded76ccbdc8df35e7cd28b68fed245a74416', 'Test Nebula'), 'Test Nebula');
});
