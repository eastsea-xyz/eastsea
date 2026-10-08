// The home page's live observations and pending transactions refresh only
// while the visitor watches. The injected schedule keeps tests offline.
export const PAGE_POLL_MS = 10_000;

export function pollCurrentPage(refresh, { getState, schedule = globalThis.setInterval }) {
  return schedule(() => {
    const { hidden, hash, pending } = getState();
    if (!hidden && (!hash || hash === '#' || hash === '#/' || pending)) refresh();
  }, PAGE_POLL_MS);
}
