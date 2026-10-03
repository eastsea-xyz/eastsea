// Audit A3 (docs/research/audit-1-2026-10-03.md): an untrusted RPC must not be
// able to change a token's units before the user signs. These checks cover the
// pin policy the background applies: a token's decimals, symbol and name are
// pinned when first seen — from two endpoints that must agree — and later reads
// that disagree are recorded, never used, and block sending until the user
// re-confirms the change.

import test from 'node:test';
import assert from 'node:assert/strict';
import { SEL, call, parseTokenAmount, wordUint } from '../src/lib/tokens.js';
import {
  pinKey, emptyPins, sameTokenMetadata, foldObserved, acceptChanged, sendBlocker, pinnedTokenInfo, catalogWithPins, denominationOf,
} from '../src/lib/tokenPin.js';
import { KNOWN_TOKENS } from '../src/lib/knownTokens.js';

const word = (v) => wordUint(v);
const str = (s) => {
  const b = Buffer.from(s, 'utf8').toString('hex');
  return `0x${word(32)}${word(b.length / 2)}${b.padEnd(Math.ceil(b.length / 64) * 64, '0')}`;
};

const usdx = '0x00000000000000000000000000000000000000c1';
const key = usdx; // already lowercase
const honest = { decimals: 6, symbol: 'USDX', name: 'Test Dollar' };
const lying = { decimals: 9, symbol: 'USDX', name: 'Test Dollar' };

/** One endpoint's raw answers, like `Rpc.call('eth_call', …)`. */
const readerOf = (meta) => async (to, data) => {
  const sel = data.slice(2, 10);
  const m = meta[to];
  if (!m) throw new Error('bad answer from the node');
  if (sel === SEL.decimals) return word(m.decimals);
  if (sel === SEL.symbol) return str(m.symbol);
  if (sel === SEL.name) return str(m.name);
  throw new Error('bad answer from the node');
};

/** Every endpoint asked, the answer only when they all agree — the shape of
 * `Rpc.callAgreed`. */
const agreedOf = (metas) => async (to, data) => {
  const answers = [];
  for (const meta of metas) answers.push(await readerOf(meta)(to, data));
  if (answers.some((a) => a !== answers[0])) {
    const e = new Error('The nodes did not agree on one answer.');
    e.disagreed = true;
    throw e;
  }
  return { result: answers[0], sources: answers.length };
};

test('the storage key is per chain', () => {
  assert.equal(pinKey(7780), 'tokenPin.7780');
  assert.deepEqual(emptyPins(), { tokens: {}, changed: {} });
});

test('first sighting pins what two agreeing endpoints answered', async () => {
  const agreed = agreedOf([{ [usdx]: honest }, { [usdx]: honest }]);
  const { info, sources } = await pinnedTokenInfo(usdx, { single: readerOf({ [usdx]: honest }), agreed });
  assert.equal(sources, 2);
  const { pins, outcome } = foldObserved(emptyPins(), usdx, info, 1_000, sources);
  assert.equal(outcome, 'pinned');
  assert.deepEqual(
    { decimals: pins.tokens[key].decimals, symbol: pins.tokens[key].symbol, name: pins.tokens[key].name, sources: pins.tokens[key].sources, pinnedAt: pins.tokens[key].pinnedAt },
    { ...honest, sources: 2, pinnedAt: 1_000 },
  );
});

test('the A3 sequence no longer signs 1,000 tokens: units come from the pin', async () => {
  // Two honest endpoints agree USDX has 6 decimals; that answer is pinned.
  const agreed = agreedOf([{ [usdx]: honest }, { [usdx]: honest }]);
  const first = await pinnedTokenInfo(usdx, { single: readerOf({ [usdx]: honest }), agreed });
  let pins = foldObserved(emptyPins(), usdx, first.info, 1_000, first.sources).pins;

  // Later the selected endpoint alone answers decimals()=9 for the genuine
  // 6-decimals token — exactly the audit's sequence.
  const again = await pinnedTokenInfo(usdx, { single: readerOf({ [usdx]: lying }) });
  const f2 = foldObserved(pins, usdx, again.info, 2_000, 1);
  assert.equal(f2.outcome, 'changed');

  // The disagreement is recorded but NEVER used: the pin keeps 6 decimals…
  assert.equal(f2.pins.tokens[key].decimals, 6);
  assert.equal(f2.pins.changed[key].decimals, 9);
  // …sending is blocked with one plain sentence…
  const blocker = sendBlocker(f2.pins, usdx);
  assert.equal(typeof blocker, 'string');
  assert.ok(/paused|blocked/.test(blocker));
  assert.equal(sendBlocker(f2.pins, '0x00000000000000000000000000000000000000ff'), null);
  // …and "1 token" still parses as 1,000,000 units, not 1,000,000,000.
  assert.equal(parseTokenAmount('1', f2.pins.tokens[key].decimals), 1_000_000n);
  pins = f2.pins;

  // A read that agrees with the pin again clears the flag on its own.
  const honestAgain = await pinnedTokenInfo(usdx, { single: readerOf({ [usdx]: honest }) });
  const f3 = foldObserved(pins, usdx, honestAgain.info, 3_000, 1);
  assert.equal(f3.outcome, 'agree');
  assert.equal(f3.pins.changed[key], undefined);
  assert.equal(sendBlocker(f3.pins, usdx), null);

  // After the user re-confirms the new values on the review screen, they
  // become the pin and the block clears.
  const f4 = foldObserved(pins, usdx, again.info, 4_000, 1); // re-flag first
  const accepted = acceptChanged(f4.pins, usdx, again.info, 5_000, 2);
  assert.equal(accepted.tokens[key].decimals, 9);
  assert.equal(accepted.tokens[key].sources, 2);
  assert.equal(accepted.changed[key], undefined);
  assert.equal(sendBlocker(accepted, usdx), null);
});

test('a first sighting the nodes dispute is not pinned', async () => {
  const disputed = agreedOf([{ [usdx]: honest }, { [usdx]: lying }]);
  await assert.rejects(
    pinnedTokenInfo(usdx, { single: readerOf({ [usdx]: lying }), agreed: disputed }),
    (e) => e.tokenUnverified === true,
  );
  // One endpoint answering alone is not a first sighting either: with a second
  // endpoint configured, its agreement is required.
  const oneVoice = agreedOf([{ [usdx]: honest }]);
  await assert.rejects(
    pinnedTokenInfo(usdx, { single: readerOf({ [usdx]: honest }), agreed: oneVoice }),
    (e) => e.tokenUnverified === true,
  );
});

test('with a single configured node, the pin records that it had no second opinion', async () => {
  const { info, sources } = await pinnedTokenInfo(usdx, { single: readerOf({ [usdx]: honest }) });
  assert.equal(sources, 1);
  const { pins } = foldObserved(emptyPins(), usdx, info, 1_000, sources);
  assert.equal(pins.tokens[key].sources, 1);
});

test('metadata comparison covers symbol and name too', () => {
  const a = { decimals: 6, symbol: 'USDX', name: 'Test Dollar' };
  assert.equal(sameTokenMetadata(a, { ...a }), true);
  assert.equal(sameTokenMetadata(a, { ...a, decimals: 9 }), false);
  assert.equal(sameTokenMetadata(a, { ...a, symbol: 'USDZ' }), false);
  assert.equal(sameTokenMetadata(a, { ...a, name: 'Other Dollar' }), false);
  assert.equal(sameTokenMetadata(null, a), false);

  // A symbol-only change is still a change: it is recorded and blocks sending.
  const renamed = { decimals: 6, symbol: 'USDZ', name: 'Test Dollar' };
  const f = foldObserved(foldObserved(emptyPins(), usdx, a, 1, 1).pins, usdx, renamed, 2, 1);
  assert.equal(f.outcome, 'changed');
  assert.ok(sendBlocker(f.pins, usdx));
});

test('a flagged catalog entry still shows the pinned details', () => {
  const pinned = foldObserved(emptyPins(), usdx, honest, 1, 2).pins;
  const flagged = foldObserved(pinned, usdx, lying, 2, 1).pins;
  const catalog = { tokens: { [usdx]: { ...lying, address: usdx, origin: 'seed' }, other: { decimals: 3, symbol: 'XYZ', name: 'Other', address: 'other' } }, rejected: [] };
  const synced = catalogWithPins(catalog, flagged);
  // The lying entry is rewritten to the pin; unpinned entries are untouched.
  assert.deepEqual(
    { decimals: synced.tokens[usdx].decimals, symbol: synced.tokens[usdx].symbol, name: synced.tokens[usdx].name },
    { ...honest },
  );
  assert.equal(synced.tokens[usdx].origin, 'seed');
  assert.equal(synced.tokens.other.decimals, 3);
});

// ---- audit R2-2: denominations come from the shipped list, not the pins ----

test('denominationOf: the shipped list decides for its tokens, pins are unverified for the rest', () => {
  const waeth = Object.keys(KNOWN_TOKENS[7780])[0];
  const waethPin = { decimals: 9, symbol: 'WAETH', name: 'Wrapped AETH', address: waeth }; // a lying pin
  const pins = { tokens: { [usdx]: { ...honest, address: usdx }, [waeth]: waethPin }, changed: {}, generation: 4 };

  // On the shipped list: shipped decimals, flagged as node disagreement, but
  // never "unverified units" and never paused.
  const known = denominationOf(7780, waeth, pins);
  assert.equal(known.decimals, 18);
  assert.equal(known.trusted, true);
  assert.equal(known.nodeDisagrees, true); // the pin differs from the list
  assert.equal(known.unverifiedUnits, undefined);
  assert.equal(known.pinGeneration, 4);

  // Not on the list: the pin's details may be shown, as unverified units.
  const unverified = denominationOf(7780, usdx, pins);
  assert.deepEqual(
    { decimals: unverified.decimals, symbol: unverified.symbol, name: unverified.name },
    honest,
  );
  assert.equal(unverified.trusted, false);
  assert.equal(unverified.unverifiedUnits, true);
  assert.equal(unverified.metadataChanged, false);
  assert.equal(unverified.pinGeneration, 4);

  // A flagged unlisted token reports the change…
  const flagged = foldObserved(pins, usdx, lying, 5, 1).pins;
  assert.equal(denominationOf(7780, usdx, flagged).metadataChanged, true);

  // …no pin at all is simply unconfirmed; junk input resolves quietly.
  const none = denominationOf(7780, '0x00000000000000000000000000000000000000c2', pins);
  assert.equal(none.unconfirmed, true);
  assert.equal(denominationOf(7780, '', pins).unconfirmed, true);
  assert.equal(denominationOf(7780, usdx, null).unconfirmed, true);
});

test('folding keeps the generation and does not churn on a persisting change', () => {
  const withGen = { tokens: {}, changed: {}, generation: 7 };
  const pinned = foldObserved(withGen, usdx, honest, 1_000, 2).pins;
  assert.equal(pinned.generation, 7); // folds never move the generation
  const flagged = foldObserved(pinned, usdx, lying, 2_000, 1).pins;
  assert.equal(flagged.generation, 7);

  // The same divergent answer observed again is not a new change: the stored
  // record (and so its content) stays exactly as it was.
  const again = foldObserved(flagged, usdx, lying, 9_000, 1);
  assert.equal(again.outcome, 'changed');
  assert.equal(again.pins.changed[key].seenAt, 2_000); // not refreshed to 9_000

  // A *different* divergence still updates the record.
  const moved = foldObserved(flagged, usdx, { decimals: 12, symbol: 'USDX', name: 'Test Dollar' }, 3_000, 1);
  assert.equal(moved.pins.changed[key].decimals, 12);

  // Accepting renumbers nothing either — only the store bumps generations.
  assert.equal(acceptChanged(flagged, usdx, lying, 4_000, 2).generation, 7);
});

test('catalogWithPins shows shipped details for listed tokens even against a lying pin', () => {
  const [waeth] = Object.keys(KNOWN_TOKENS[7780]);
  const catalog = { tokens: {
    [usdx]: { decimals: 3, symbol: 'OLD', name: 'Stale', address: usdx },
    [waeth]: { decimals: 9, symbol: 'WAETH', name: 'Lying Pin', address: waeth },
  }, rejected: [] };
  const pins = foldObserved(foldObserved({ tokens: {}, changed: {}, generation: 0 }, usdx, honest, 1, 1).pins, waeth, { decimals: 9, symbol: 'WAETH', name: 'Lying Pin' }, 2, 1).pins;
  const synced = catalogWithPins(catalog, pins, 7780);
  // The unlisted token shows its pinned details…
  assert.deepEqual(
    { decimals: synced.tokens[usdx].decimals, symbol: synced.tokens[usdx].symbol, name: synced.tokens[usdx].name },
    honest,
  );
  // …the listed one shows the shipped list's, pin or no pin.
  assert.equal(synced.tokens[waeth].decimals, 18);
  assert.equal(synced.tokens[waeth].name, KNOWN_TOKENS[7780][waeth].name);
});
