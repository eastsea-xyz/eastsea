// Audits R2-2 + R2-5 (docs/research/audit-2-2026-10-03.md), end to end at the
// pure-function level: an untrusted RPC — even two endpoints agreeing — must
// not silently define the units a signed transfer uses, and an open
// confirmation must not sign against token details that moved under it.
//
// The malicious node is simulated with fake readers shaped exactly like
// `Rpc.call('eth_call')` and `Rpc.callAgreed` (see test/token-pin.test.mjs);
// no live RPC or Chrome window is involved.

import test from 'node:test';
import assert from 'node:assert/strict';
import { SEL, wordUint } from '../src/lib/tokens.js';
import { foldObserved, acceptChanged, pinnedTokenInfo, denominationOf } from '../src/lib/tokenPin.js';
import { pinStore } from '../src/lib/pinStore.js';
import { buildSendIntent, checkSendIntent } from '../src/lib/sendIntent.js';
import { knownToken } from '../src/lib/knownTokens.js';

const word = (v) => wordUint(v);
const str = (s) => {
  const b = Buffer.from(s, 'utf8').toString('hex');
  return `0x${word(32)}${word(b.length / 2)}${b.padEnd(Math.ceil(b.length / 64) * 64, '0')}`;
};

const usdx = '0x00000000000000000000000000000000000000c1';
const waeth = '0xa2521982a17474cb2f8741c85de653b5282d72b0'; // on the shipped list, 18 decimals
const recipient = '0x00000000000000000000000000000000000000be';
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

/** The amount word of a `transfer(address,uint256)` calldata. */
const amountOf = (calldata) => BigInt(`0x${calldata.slice(-64)}`);

/** What the popup does when the confirmation card is built: the amount text
 * is parsed once, under the decimals the screen shows. */
const intentFor = (d, amountText, acked = false) => buildSendIntent({
  recipient, amountText,
  token: { address: usdxOr(d), decimals: d.decimals, trusted: Boolean(d.trusted), acknowledged: acked, pinGeneration: d.pinGeneration },
});
const usdxOr = (d) => d.symbol === 'WAETH' ? waeth : usdx;

test('R2-2: two identical malicious answers still cannot set the units silently', async () => {
  // Both configured endpoints answer the SAME malicious decimals=9 for a
  // token that is not on the shipped list — the audit's exact sequence.
  const agreed = agreedOf([{ [usdx]: lying }, { [usdx]: lying }]);
  const seen = await pinnedTokenInfo(usdx, { single: readerOf({ [usdx]: lying }), agreed });
  assert.equal(seen.sources, 2); // agreement achieved — and meaningless
  const pins = foldObserved({ tokens: {}, changed: {}, generation: 0 }, usdx, seen.info, 1_000, seen.sources).pins;

  // The metadata may be displayed, but never as a trusted denomination.
  const d = denominationOf(7780, usdx, pins);
  assert.equal(d.decimals, 9);
  assert.equal(d.trusted, false);
  assert.equal(d.unverifiedUnits, true);

  // The confirmation is built around the exact base-unit count…
  const intent = intentFor(d, '1');
  assert.equal(intent.baseUnits, '1000000000'); // 1 × 10^9, shown and acked as units
  // …and without the explicit acknowledgement nothing signs: no silent
  // "1 token" under a node-chosen decimals.
  assert.throws(() => checkSendIntent(intent, { pins, known: null }), /trusted list/);

  // With the acknowledgement, the signature uses exactly the confirmed
  // integer — 1,000,000,000 units — not a re-parse of "1" under any decimals
  // stored later.
  const calldata = checkSendIntent({ ...intent, token: { ...intent.token, acknowledged: true } }, { pins, known: null });
  assert.equal(amountOf(calldata), 1_000_000_000n);
});

test('R2-2: a lying node cannot move the units of a token on the shipped list', async () => {
  const lyingWaeth = { decimals: 9, symbol: 'WAETH', name: 'Wrapped AETH' };
  const agreed = agreedOf([{ [waeth]: lyingWaeth }, { [waeth]: lyingWaeth }]);
  const seen = await pinnedTokenInfo(waeth, { single: readerOf({ [waeth]: lyingWaeth }), agreed });
  const pins = foldObserved({ tokens: {}, changed: {}, generation: 0 }, waeth, seen.info, 1_000, 2).pins;

  // The shipped list decides the denomination; the node's answer is only
  // reported as disagreement noise.
  const d = denominationOf(7780, waeth, pins);
  assert.equal(d.decimals, 18); // not 9, whatever both endpoints said
  assert.equal(d.symbol, 'WAETH');
  assert.equal(d.trusted, true);
  assert.equal(d.nodeDisagrees, true);
  assert.equal(d.unverifiedUnits, undefined);

  // "1" signs as exactly 10^18 units, without any acknowledgement flow.
  const intent = intentFor(d, '1');
  assert.equal(intent.baseUnits, (10n ** 18n).toString());
  assert.equal(intent.token.trusted, true);
  const calldata = checkSendIntent(intent, { pins, known: knownToken(7780, waeth) });
  assert.equal(amountOf(calldata), 10n ** 18n);

  // And an intent carrying the node's lying decimals is refused outright.
  assert.throws(
    () => checkSendIntent({ ...intent, token: { ...intent.token, decimals: 9 } }, { pins, known: knownToken(7780, waeth) }),
    /shipped with the wallet/,
  );
});

test('R2-2: one endpoint is enough to read, never enough to trust', async () => {
  // Only the default local node configured: the pin records its lone answer.
  const seen = await pinnedTokenInfo(usdx, { single: readerOf({ [usdx]: honest }) });
  assert.equal(seen.sources, 1);
  const pins = foldObserved({ tokens: {}, changed: {}, generation: 0 }, usdx, seen.info, 1_000, 1).pins;

  const d = denominationOf(7780, usdx, pins);
  assert.equal(d.decimals, 6); // display still works
  assert.equal(d.trusted, false);
  assert.equal(d.unverifiedUnits, true); // but the metadata is unverified

  const intent = intentFor(d, '2');
  assert.equal(intent.baseUnits, '2000000');
  assert.throws(() => checkSendIntent(intent, { pins, known: null }), /trusted list/);
  assert.equal(amountOf(checkSendIntent({ ...intent, token: { ...intent.token, acknowledged: true } }, { pins, known: null })), 2_000_000n);

  // With a second endpoint configured, a first sighting it does not answer
  // for is not pinned at all (the caller skips the token this scan).
  const oneVoice = agreedOf([{ [usdx]: honest }]);
  await assert.rejects(
    pinnedTokenInfo(usdx, { single: readerOf({ [usdx]: honest }), agreed: oneVoice }),
    (e) => e.tokenUnverified === true,
  );
});

test('R2-5: a confirmation left open while the pins moved is refused', async () => {
  const area = {
    data: new Map(),
    get: async (k) => area.data.get(k),
    set: async (k, v) => { area.data.set(k, v); },
  };
  const store = pinStore(area);
  await store.update(7780, (p) => foldObserved(p, usdx, honest, 1_000, 2).pins); // gen 1

  // UI action A: the confirmation card is built now.
  const dA = denominationOf(7780, usdx, await store.read(7780));
  const intentA = intentFor(dA, '2', true); // acked, "2" × 10^6 units

  // While A sits open, the node diverges and the user reviews and accepts
  // the new details in the Assets view — two generation moves.
  await store.update(7780, (p) => foldObserved(p, usdx, lying, 2_000, 1).pins);
  await store.update(7780, (p) => acceptChanged(p, usdx, lying, 3_000, 2));

  // UI action B: a fresh confirmation against the moved state.
  const dB = denominationOf(7780, usdx, await store.read(7780));
  const intentB = intentFor(dB, '2', true);

  const pinsNow = await store.read(7780);
  // The interleaved stale confirmation is refused with one plain sentence…
  assert.throws(() => checkSendIntent(intentA, { pins: pinsNow, known: null }), /changed since the send was confirmed/);
  // …the current one signs — with ITS units (2 × 10^9 now), not A's.
  assert.equal(amountOf(checkSendIntent(intentB, { pins: pinsNow, known: null })), 2_000_000_000n);
  assert.notEqual(intentA.token.pinGeneration, intentB.token.pinGeneration);
});

test('R2-5: changed pin decimals under the same generation still refuse', async () => {
  // A generation that did not move cannot hide different content: the stored
  // decimals must still match what the confirmation showed.
  const pins = { tokens: { [usdx]: { ...lying, address: usdx, sources: 2, pinnedAt: 1 } }, changed: {}, generation: 12 };
  const intent = intentFor({ decimals: 9, pinGeneration: 12, trusted: false }, '1', true);
  const tampered = JSON.parse(JSON.stringify(pins));
  tampered.tokens[usdx].decimals = 77; // same generation, other content
  assert.throws(() => checkSendIntent(intent, { pins: tampered, known: null }), /changed since the send was confirmed|confirm the send again/);
  assert.equal(amountOf(checkSendIntent(intent, { pins, known: null })), 1_000_000_000n);
});

test('an unconfirmed or flagged token never signs, acknowledged or not', async () => {
  const honestPins = foldObserved({ tokens: {}, changed: {}, generation: 0 }, usdx, honest, 1_000, 2).pins;
  const flagged = foldObserved(honestPins, usdx, lying, 2_000, 1).pins;
  const d = denominationOf(7780, usdx, flagged);
  const intent = intentFor(d, '1', true);
  assert.equal(d.metadataChanged, true);
  assert.throws(() => checkSendIntent(intent, { pins: flagged, known: null }), /paused|review/);

  // No pin at all (the scan could not confirm a first sighting): refused too.
  const none = denominationOf(7780, '0x00000000000000000000000000000000000000c2', { tokens: {}, changed: {}, generation: 0 });
  assert.equal(none.unconfirmed, true);
  const ghost = buildSendIntent({ recipient, amountText: '1', token: { address: '0x00000000000000000000000000000000000000c2', decimals: 6, acknowledged: true, pinGeneration: 0 } });
  assert.throws(() => checkSendIntent(ghost, { pins: { tokens: {}, changed: {}, generation: 0 }, known: null }), /not confirmed yet/);
});

test('intents validate their inputs at both ends', () => {
  const good = { address: usdx, decimals: 6, pinGeneration: 0 };
  assert.throws(() => buildSendIntent({ recipient: '0xnope', amountText: '1', token: good }), /recipient/);
  assert.throws(() => buildSendIntent({ recipient, amountText: '1', token: { ...good, address: '0xNOPE' } }), /token/);
  assert.throws(() => buildSendIntent({ recipient, amountText: '1', token: { ...good, decimals: 78 } }), /decimals/);
  assert.throws(() => buildSendIntent({ recipient, amountText: '1', token: { ...good, decimals: -1 } }), /decimals/);
  assert.throws(() => buildSendIntent({ recipient, amountText: '1', token: { ...good, decimals: 1.5 } }), /decimals/);
  assert.throws(() => buildSendIntent({ recipient, amountText: 'abc', token: good }), /amount/);

  // The executed side re-validates the base-unit integer itself.
  const intent = buildSendIntent({ recipient, amountText: '1', token: good });
  for (const bad of ['-1', '1.5', '0x10', '1e9', '', (2n ** 256n).toString()]) {
    assert.throws(() => checkSendIntent({ ...intent, baseUnits: bad }, { pins: { tokens: {}, changed: {}, generation: 0 }, known: null }), /base units|not an address|usable/, bad);
  }
  assert.throws(() => checkSendIntent({ ...intent, recipient: '0xnope' }, { pins: { tokens: {}, changed: {}, generation: 0 }, known: null }), /not an address/);
});
