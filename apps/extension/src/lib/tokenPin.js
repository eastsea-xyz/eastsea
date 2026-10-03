// Trust-on-first-use pinning for ERC-20 metadata (docs/research/audit-1-2026-10-03.md,
// finding A3): the units a signed transfer uses must never be something one
// untrusted RPC can change. The first time a token is seen, its decimals, symbol
// and name are read from two different configured endpoints that must agree;
// the agreed values are pinned in extension storage keyed by (chainId, token
// address). A later read that disagrees with the pin is never used — the token
// is marked "metadata changed" and sending it is blocked until the user
// re-confirms on a screen that shows the old and new values. Pure functions
// with the readers passed in, so this is tested without a node
// (test/token-pin.test.mjs).

import { SEL, call, tokenInfoFrom } from './tokens.js';

/** The per-chain pin store: `tokenPin.<chainId>` in chrome.storage.local. */
export const pinKey = (chainId) => `tokenPin.${chainId}`;

export const emptyPins = () => ({ tokens: {}, changed: {} });

/** Do two metadata reads describe the same token details? */
export function sameTokenMetadata(a, b) {
  return Boolean(a && b) && a.decimals === b.decimals && a.symbol === b.symbol && a.name === b.name;
}

/** A first sighting that could not be confirmed by two endpoints: the caller
 * skips the token this scan (it is neither catalogued nor rejected) rather
 * than trust one endpoint's answer. */
const unverified = (why) => Object.assign(new Error(why), { tokenUnverified: true });

/**
 * Token metadata read through the pin policy:
 * - `single(to, data)`: one endpoint's raw answer (the everyday continuity read).
 * - `agreed(to, data)`: `{ result, sources }` — every configured endpoint
 *   asked, the answer only when they all agree (`Rpc.callAgreed`). Given for
 *   first sightings and metadata reviews, when a second endpoint exists; the
 *   answer is accepted only when `sources` is at least 2.
 * Returns `{ info, sources }` (`info` null when the answers do not decode as
 * a token, like `tokenInfo`), or throws a `tokenUnverified` error.
 */
export async function pinnedTokenInfo(address, { single, agreed = null }) {
  let sources = 1;
  const one = async (data) => {
    if (!agreed) return await single(address, data);
    let out;
    try {
      out = await agreed(address, data);
    } catch (e) {
      if (e?.disagreed) throw unverified("The nodes did not agree on this token's details.");
      throw e;
    }
    if (out.sources < 2) throw unverified('Only one node answered about this token, so its details are not confirmed yet.');
    sources = Math.max(sources, out.sources);
    return out.result;
  };
  const info = await tokenInfoFrom({
    decimals: () => one(call(SEL.decimals)),
    symbol: () => one(call(SEL.symbol)),
    name: () => one(call(SEL.name)),
  });
  return { info: info && { ...info, address: address.toLowerCase() }, sources };
}

/**
 * Fold one parsed metadata read into the pin state (pure):
 * - 'pinned': first sighting — `observed` becomes the pin (the caller read it
 *   through the two-source rule above).
 * - 'agree': matches the pin; a pending change flag also clears, because the
 *   anomaly ended on its own.
 * - 'changed': disagrees — recorded under `changed`, never used, and sending
 *   is blocked (`sendBlocker`) until the user re-confirms.
 */
export function foldObserved(pins, address, observed, now = Date.now(), sources = 1) {
  const key = String(address || '').toLowerCase();
  const cur = pins.tokens[key];
  if (!cur) {
    return {
      pins: { tokens: { ...pins.tokens, [key]: { ...observed, address: key, sources, pinnedAt: now } }, changed: pins.changed },
      outcome: 'pinned',
    };
  }
  if (sameTokenMetadata(cur, observed)) {
    if (!pins.changed[key]) return { pins, outcome: 'agree' };
    const changed = { ...pins.changed };
    delete changed[key];
    return { pins: { tokens: pins.tokens, changed }, outcome: 'agree' };
  }
  return {
    pins: { tokens: pins.tokens, changed: { ...pins.changed, [key]: { ...observed, address: key, seenAt: now } } },
    outcome: 'changed',
  };
}

/** After the user compared the old and new values and chose the new ones: they
 * become the pin and the block clears. */
export function acceptChanged(pins, address, observed, now = Date.now(), sources = pins.tokens[String(address || '').toLowerCase()]?.sources ?? 1) {
  const key = String(address || '').toLowerCase();
  const tokens = { ...pins.tokens, [key]: { ...observed, address: key, sources, pinnedAt: now } };
  const changed = { ...pins.changed };
  delete changed[key];
  return { tokens, changed };
}

/** May this token be sent right now? One plain sentence when not. */
export function sendBlocker(pins, address) {
  const key = String(address || '').toLowerCase();
  return pins.changed[key]
    ? 'This token now reports different details than the ones saved on this device, so sending is paused until you review the change in Assets.'
    : null;
}

/** A catalog whose token entries carry the pinned details, so every view that
 * reads the catalog (activity labels, the safety sets) shows what was pinned,
 * not what one endpoint last said. */
export function catalogWithPins(catalog, pins) {
  const tokens = {};
  for (const [key, t] of Object.entries(catalog.tokens || {})) {
    const pin = pins.tokens[key];
    tokens[key] = pin ? { ...t, decimals: pin.decimals, symbol: pin.symbol, name: pin.name } : t;
  }
  return { ...catalog, tokens };
}
