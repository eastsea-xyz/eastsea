// Audit R2-5 (docs/research/audit-2-2026-10-03.md): metadata-pin writes from
// the asset scan and from a metadata review used to race — whichever finished
// last silently replaced the other, and an open send confirmation could not
// tell the pins behind it had moved. All writes now funnel through one
// serialized read-modify-write whose mutator always runs against the pins in
// storage at write time, with a generation that moves only on real content
// changes. These checks race the two writers exactly the way two extension
// views can.

import test from 'node:test';
import assert from 'node:assert/strict';
import { pinKey, emptyPins, foldObserved, acceptChanged } from '../src/lib/tokenPin.js';
import { pinStore, withGeneration, samePinContent } from '../src/lib/pinStore.js';

const usdx = '0x00000000000000000000000000000000000000c1';
const honest = { decimals: 6, symbol: 'USDX', name: 'Test Dollar' };
const lying = { decimals: 9, symbol: 'USDX', name: 'Test Dollar' };

/** A minimal chrome.storage.local-shaped area (get/set), in memory. */
const memoryArea = () => {
  const data = new Map();
  return {
    get: async (k) => data.get(k),
    set: async (k, v) => { data.set(k, v); },
    dump: () => data,
  };
};

test('a missing store reads as generation 0 with no pins', async () => {
  const store = pinStore(memoryArea());
  assert.deepEqual(await store.read(7780), { tokens: {}, changed: {}, generation: 0 });
});

test('the generation moves only when the content changes', async () => {
  const store = pinStore(memoryArea());
  const p1 = await store.update(7780, (p) => foldObserved(p, usdx, honest, 1_000, 2).pins);
  assert.equal(p1.generation, 1); // a new pin is a change
  const p2 = await store.update(7780, (p) => foldObserved(p, usdx, honest, 2_000, 2).pins);
  assert.equal(p2.generation, 1); // the same observation again changes nothing
  assert.equal((await store.update(7780, (p) => p)).generation, 1); // nor does a no-op write
  const p3 = await store.update(7780, (p) => foldObserved(p, usdx, lying, 3_000, 1).pins);
  assert.equal(p3.generation, 2); // a divergence does
  assert.equal(p3.changed[usdx].decimals, 9);
  // Content comparison ignores the generation by design.
  assert.equal(samePinContent({ tokens: {}, changed: {}, generation: 5 }, emptyPins()), true);
  assert.equal(withGeneration({ tokens: {}, changed: {} }).generation, 0);
  assert.equal(withGeneration({ tokens: {}, changed: {}, generation: 41 }).generation, 41);
  assert.equal(withGeneration(null).generation, 0);
});

test('a scan in flight cannot clobber a review accepted meanwhile (R2-5)', async () => {
  const area = memoryArea();
  const store = pinStore(area);
  // The pin starts honest, then the node turns dishonest.
  await store.update(7780, (p) => foldObserved(p, usdx, honest, 1_000, 2).pins);
  const flagged = await store.update(7780, (p) => foldObserved(p, usdx, lying, 2_000, 1).pins);
  assert.equal(flagged.generation, 2);

  // A scan that read the state BEFORE the review starts folding its stale
  // observation of the honest pin back in…
  const staleScan = store.update(7780, (p) => foldObserved(p, usdx, honest, 3_000, 1).pins);
  // …while the user, in another view, compares and accepts the new details.
  const review = store.update(7780, (p) => acceptChanged(p, usdx, lying, 4_000, 2));
  await Promise.all([staleScan, review]);

  const pins = await store.read(7780);
  // The accept survived…
  assert.equal(pins.tokens[usdx].decimals, 9);
  assert.equal(pins.changed[usdx], undefined);
  // …and neither write clobbered the other: the scan's fold cleared the flag
  // (its honest read agreed with the then-current pin), the accept then moved
  // the pin — two content changes, two generation steps, both applied.
  assert.equal(pins.generation, 4);
});

test('updates apply to whatever storage holds at write time', async () => {
  const area = memoryArea();
  const store = pinStore(area);
  // Storage written outside the queue (a previous worker's last write, say).
  await area.set(pinKey(7780), { tokens: { [usdx]: { ...honest, address: usdx, sources: 2, pinnedAt: 1 } }, changed: {}, generation: 7 });
  const pins = await store.update(7780, (p) => foldObserved(p, usdx, lying, 2_000, 1).pins);
  // The fold saw the stored 6-decimals pin, flagged instead of replacing it,
  // and the generation moved from the stored value.
  assert.equal(pins.tokens[usdx].decimals, 6);
  assert.equal(pins.changed[usdx].decimals, 9);
  assert.equal(pins.generation, 8);
});

test('a failed write does not jam the queue', async () => {
  const store = pinStore(memoryArea());
  await assert.rejects(store.update(7780, () => { throw new Error('storage full'); }));
  const pins = await store.update(7780, (p) => foldObserved(p, usdx, honest, 1, 1).pins);
  assert.equal(pins.tokens[usdx].decimals, 6);
  assert.equal(pins.generation, 1);
});

test('chains do not share a queue result (per-key isolation)', async () => {
  const store = pinStore(memoryArea());
  const neb = '0x6bc5ded76ccbdc8df35e7cd28b68fed245a74416';
  await Promise.all([
    store.update(7780, (p) => foldObserved(p, usdx, honest, 1, 1).pins),
    store.update(7777, (p) => foldObserved(p, neb, lying, 1, 1).pins),
  ]);
  const [a, b] = await Promise.all([store.read(7780), store.read(7777)]);
  assert.equal(a.tokens[usdx].decimals, 6);
  assert.equal(a.tokens[neb], undefined);
  assert.equal(b.tokens[neb].decimals, 9);
  assert.equal(b.tokens[usdx], undefined);
  assert.equal(a.generation, 1);
  assert.equal(b.generation, 1);
});
