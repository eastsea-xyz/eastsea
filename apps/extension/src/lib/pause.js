// "Network paused": the chain has made no new block for a while. The same rule
// as the app (WalletModel.trackChainProgress): the height has not moved here
// for 60 s, or the newest block is that old while the height is not moving
// here either — so a wrong clock on this computer alone never looks like a
// pause. Pure, so it is tested without a node (test/pause.test.mjs).

/** No new block for this long means the network is paused. */
export const PAUSE_AFTER_MS = 60_000;
/** The newest-block age only counts once the height has been still this long. */
export const STILL_GRACE_MS = 20_000;

/**
 * Fold one observation into the pause state. `prev` is the last state (null on
 * the first look); `{ now, height, blockAt }` are this look, with `height` the
 * chain's height and `blockAt` its newest block's time (both null when the node
 * does not answer, which keeps the previous state).
 * Returns `{ lastHeight, heightChangedAt, pausedSince }`; `pausedSince` is null
 * while the network runs.
 */
export function nextPauseState(prev, { now, height, blockAt }) {
  const lastHeight = height != null ? height : prev?.lastHeight ?? null;
  const moved = height != null && prev?.lastHeight != null && height !== prev.lastHeight;
  const heightChangedAt = moved ? now : prev?.heightChangedAt ?? now;
  const still = now - heightChangedAt;
  const oldBlock = blockAt != null && now - blockAt > PAUSE_AFTER_MS;
  const paused = still > PAUSE_AFTER_MS || (oldBlock && still > STILL_GRACE_MS);
  const since = paused ? Math.min(...[blockAt, heightChangedAt, now].filter((t) => t != null)) : null;
  return { lastHeight, heightChangedAt, pausedSince: since };
}

/** "Network paused · last block 3 min ago", as in the app's NetworkPausedText. */
export function pausedLine(pausedSince, now) {
  const minutes = Math.max(1, Math.floor((now - pausedSince) / 60_000));
  const ago = minutes < 120 ? `${minutes} min ago` : `${Math.floor(minutes / 60)} h ago`;
  return `Network paused · last block ${ago}`;
}

/** What the paused pill explains on hover, as in the app (no promises). */
export const PAUSE_HELP = 'The network has not made a new block for a while. The balance shown is the last one this wallet read; a pause by itself does not move funds. It updates by itself when blocks resume.';
