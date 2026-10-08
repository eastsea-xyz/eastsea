import test from 'node:test';
import assert from 'node:assert/strict';
import { KNOWN_TOKENS } from '../src/lib/knownTokens.js';
import { tokenArtSource, tokenFallbackAppearance } from '../ui/token-art.js';

test('official artwork follows the pinned chain and address', () => {
  for (const [address, token] of Object.entries(KNOWN_TOKENS[7780])) {
    assert.ok(tokenArtSource(7780, address).endsWith(`/assets/${token.symbol}-256.png`));
    assert.equal(tokenArtSource(7780, address.toUpperCase()), tokenArtSource(7780, address));
    assert.equal(tokenArtSource(7777, address), null);
  }
});

test('unknown addresses and metadata cannot borrow official artwork', () => {
  const lookalike = { address: '0x1111111111111111111111111111111111111111', symbol: 'NEB', name: 'Test Nebula', trusted: true };
  assert.equal(tokenArtSource(7780, lookalike.address), null);
  assert.equal(tokenArtSource(7780, lookalike), null);
  assert.equal(tokenArtSource(7780, ''), null);
});

test('generated color matches wallet hash vectors and casefolds the address', () => {
  const first = { address: '0x1111111111111111111111111111111111111111', symbol: 'MOON' };
  assert.equal(tokenFallbackAppearance(first).hue, 5);
  assert.equal(tokenFallbackAppearance({ ...first, address: first.address.toUpperCase() }).fill, tokenFallbackAppearance(first).fill);
  const mixed = { address: '0x777e11112222333344445555666677778888a40b', symbol: 'MOON' };
  assert.equal(tokenFallbackAppearance(mixed).fill, 'hsl(80 45% 62%)');
  assert.deepEqual(tokenFallbackAppearance({ ...mixed, address: mixed.address.toUpperCase() }), tokenFallbackAppearance(mixed));
});

test('metadata renaming keeps color while two same-ticker addresses differ', () => {
  const first = { address: '0x1111111111111111111111111111111111111111', symbol: 'MOON' };
  const second = { address: '0x2222222222222222222222222222222222222222', symbol: 'MOON' };
  const renamed = { ...first, symbol: '  NEB', name: 'Test Nebula', trusted: true };
  assert.notEqual(tokenFallbackAppearance(first).fill, tokenFallbackAppearance(second).fill);
  assert.equal(tokenFallbackAppearance(first).fill, tokenFallbackAppearance(renamed).fill);
  assert.equal(tokenFallbackAppearance(renamed).letter, 'N');
  assert.equal(tokenFallbackAppearance({ ...first, symbol: ' ' }).letter, '?');
  assert.equal(tokenArtSource(7780, renamed.address), null);
});
