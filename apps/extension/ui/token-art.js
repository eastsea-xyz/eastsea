import { knownToken } from '../src/lib/knownTokens.js';

const ART = Object.freeze({
  WAETH: 'WAETH-256.png',
  NEB: 'NEB-256.png',
  ORB: 'ORB-256.png',
  CMT: 'CMT-256.png',
});

/** Artwork follows the shipped chain/address identity, never RPC metadata. */
export function tokenArtSource(chainId, address) {
  const entry = knownToken(chainId, address);
  const file = entry && ART[entry.symbol];
  return file ? new URL(`assets/${file}`, import.meta.url).href : null;
}

/** Match TokenIconSpec.swift: lowercase-address FNV-1a, then spread hues.
 * Metadata can change the letter, but can never choose the color or art. */
export function tokenFallbackAppearance(token) {
  let seed = 0x811c9dc5;
  for (const byte of new TextEncoder().encode(String(token?.address || '').toLowerCase())) {
    seed = Math.imul(seed ^ byte, 0x01000193) >>> 0;
  }
  const hue = (Math.imul(seed, 2654435761) >>> 0) % 360;
  const first = [...String(token?.symbol || '').trim()][0] || '?';
  return { hue, fill: `hsl(${hue} 45% 62%)`, letter: first.toUpperCase() };
}
