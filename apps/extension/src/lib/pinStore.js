// Pin storage with a generation counter (audit R2-5): asset refreshes and
// metadata-review accepts both write the whole pin object, and two extension
// views acting at once used to overwrite each other — a just-accepted change
// could be silently replaced by a scan that read older state, and an open
// confirmation could not tell that the pins behind it had moved. Every write
// here is a serialized read-modify-write: the mutator always runs against the
// pins in storage at write time (never a copy read before a network round
// trip), and the generation bumps exactly when the stored content changed, so
// a send intent (lib/sendIntent.js) can reject anything confirmed against a
// stale generation while no-op scans do not invalidate open confirmations.
//
// chrome.storage has no atomic compare-and-swap, so the compare lives one
// level up: all writers in this service worker funnel through this one
// queue, and the send path re-reads storage and compares generations at
// execute time. The `area` is chrome.storage.local-shaped (get/set), so this
// is tested without Chrome (test/pin-store.test.mjs).

import { pinKey, emptyPins } from './tokenPin.js';

/** The pins with a guaranteed integer generation. */
export const withGeneration = (pins) => ({
  ...emptyPins(),
  ...pins,
  generation: Number.isSafeInteger(pins?.generation) ? pins.generation : 0,
});

/** Pins compare by content only (tokens + changed), ignoring the generation. */
export const samePinContent = (a, b) => {
  const body = (p) => JSON.stringify({ tokens: p?.tokens || {}, changed: p?.changed || {} });
  return body(a) === body(b);
};

/** A serialized read-modify-write pin store over one storage area. */
export function pinStore(area) {
  let queue = Promise.resolve();

  /** The pins stored for `chainId` right now. */
  async function read(chainId) {
    return withGeneration(await area.get(pinKey(chainId)));
  }

  /**
   * Apply `mutate(current)` to the pins in storage and store the result.
   * `mutate` must be pure (no awaits): it is re-run against whatever storage
   * holds when the write's turn comes, so a review accepted while a scan was
   * in flight is kept, not clobbered. The generation bumps only when the
   * content actually changed. Returns the stored pins.
   */
  function update(chainId, mutate) {
    const run = async () => {
      const current = await read(chainId);
      const next = withGeneration(await mutate(current));
      if (samePinContent(current, next)) return current;
      next.generation = current.generation + 1;
      await area.set(pinKey(chainId), next);
      return next;
    };
    const out = queue.then(run, run); // a failed write does not jam the queue
    queue = out.then(() => {}, () => {});
    return out;
  }

  return { read, update };
}
