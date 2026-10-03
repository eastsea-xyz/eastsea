// Trust-on-first-use pinning for ERC-20 metadata (docs/research/audit-1-2026-10-03.md
// finding A3, tightened by audit-2 R2-2): the units a signed transfer uses must
// never be something an untrusted RPC can decide. Pins record what the nodes
// said for continuity — a later read that disagrees with the pin is never used
// silently — but since audit 2 they are NOT trusted denominations: trusted
// units come only from the shipped list (lib/knownTokens.js), and every other
// token's units are unverified, needing an explicit base-unit confirmation to
// send (lib/sendIntent.js). The first time a token is seen, its decimals,
// symbol and name are read from two different configured endpoints that must
// agree (when a second endpoint exists); the agreed values are pinned in
// extension storage keyed by (chainId, token address). A stored pin state also
// carries a `generation`, moved by lib/pinStore.js whenever its content
// changes, which the send path checks so an open confirmation cannot sign
// against pins that moved under it (audit R2-5). Pure functions with the
// readers passed in, so this is tested without a node (test/token-pin.test.mjs).

import { SEL, call, tokenInfoFrom } from './tokens.js';
import { knownToken } from './knownTokens.js';

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
      pins: { ...pins, tokens: { ...pins.tokens, [key]: { ...observed, address: key, sources, pinnedAt: now } } },
      outcome: 'pinned',
    };
  }
  if (sameTokenMetadata(cur, observed)) {
    if (!pins.changed[key]) return { pins, outcome: 'agree' };
    const changed = { ...pins.changed };
    delete changed[key];
    return { pins: { ...pins, tokens: pins.tokens, changed }, outcome: 'agree' };
  }
  // The same divergent answer again keeps the recorded change as it is: a
  // persisting disagreement must not churn the stored content (and so the pin
  // generation) on every scan.
  if (pins.changed[key] && sameTokenMetadata(pins.changed[key], observed)) return { pins, outcome: 'changed' };
  return {
    pins: { ...pins, tokens: pins.tokens, changed: { ...pins.changed, [key]: { ...observed, address: key, seenAt: now } } },
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
  return { ...pins, generation: (pins.generation || 0), tokens, changed };
}

/** May this token be sent right now? One plain sentence when not. (Tokens on
 * the shipped list are exempt: their units never come from the pin, so node
 * noise does not pause them — the caller checks the list first.) */
export function sendBlocker(pins, address) {
  const key = String(address || '').toLowerCase();
  return pins.changed[key]
    ? 'This token now reports different details than the ones saved on this device, so sending is paused until you review the change in Assets.'
    : null;
}

/** The details a display or a send must use for `address` on `chainId`
 * (audit R2-2): the shipped list decides; the pin only ever supplies
 * unverified details. Returns `{ decimals, symbol, name, trusted }` plus the
 * state the views show — `unverifiedUnits` (not on the shipped list),
 * `nodeDisagrees` (on the list, but the stored pin or a flagged change
 * differs from it), `metadataChanged`, `unconfirmed` (no pin at all), and the
 * `pinGeneration` the send intent must carry. */
export function denominationOf(chainId, address, pins) {
  const key = String(address || '').toLowerCase();
  const generation = Number(pins?.generation) || 0;
  const known = knownToken(chainId, key);
  if (known) {
    const pin = pins?.tokens?.[key];
    return {
      decimals: known.decimals, symbol: known.symbol, name: known.name,
      trusted: true,
      nodeDisagrees: Boolean(pins?.changed?.[key]) || (Boolean(pin) && !sameTokenMetadata(pin, known)),
      pinGeneration: generation,
    };
  }
  const pin = pins?.tokens?.[key];
  if (!pin) {
    return { decimals: null, symbol: null, name: null, trusted: false, unconfirmed: true, pinGeneration: generation };
  }
  return {
    decimals: pin.decimals, symbol: pin.symbol, name: pin.name,
    trusted: false, unverifiedUnits: true,
    metadataChanged: Boolean(pins?.changed?.[key]),
    pinGeneration: generation,
  };
}

/** A catalog whose token entries carry the pinned details, so every view that
 * reads the catalog (activity labels, the safety sets) shows what was pinned,
 * not what one endpoint last said. Entries on the shipped list (`chainId`)
 * show the shipped details instead — a lying pin never rewrites them. */
export function catalogWithPins(catalog, pins, chainId = null) {
  const tokens = {};
  for (const [key, t] of Object.entries(catalog.tokens || {})) {
    const known = chainId != null ? knownToken(chainId, key) : null;
    if (known) {
      tokens[key] = { ...t, decimals: known.decimals, symbol: known.symbol, name: known.name };
      continue;
    }
    const pin = pins.tokens[key];
    tokens[key] = pin ? { ...t, decimals: pin.decimals, symbol: pin.symbol, name: pin.name } : t;
  }
  return { ...catalog, tokens };
}
