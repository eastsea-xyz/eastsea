// Archipelago v1. This is the canonical module; mirror it with
// `node scripts/sync-account-icons.mjs`. See docs/design/46-account-icon.md.
export const ACCOUNT_ICON_VERSION = 1;
export const ACCOUNT_ICON_PALETTES = Object.freeze([
  '#4e8dad', '#368f8b', '#73864a', '#a77a45',
  '#b36c5c', '#96749e', '#758694', '#958130',
]);
export const ACCOUNT_ICON_INK = '#101820';

const DOMAIN = 'eastsea-account-icon-v1';
const SVG_NS = 'http://www.w3.org/2000/svg';
const SHA256_INITIAL = [
  0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
  0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
];
const SHA256_K = [
  0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
  0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
  0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
  0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
  0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
  0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
  0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
  0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

const rotateRight = (v, n) => (v >>> n) | (v << (32 - n));

/** The prescribed 43-byte input always occupies one SHA-256 block. A local,
 * synchronous implementation also works on eastsea-page:, where browsers
 * may not expose SubtleCrypto. It is only used for public icon derivation. */
function seedPrefix(addressBytes) {
  const block = new Uint8Array(64);
  for (let i = 0; i < DOMAIN.length; i++) block[i] = DOMAIN.charCodeAt(i);
  block.set(addressBytes, DOMAIN.length);
  const length = DOMAIN.length + addressBytes.length;
  block[length] = 0x80;
  block[62] = (length * 8) >>> 8;
  block[63] = (length * 8) & 255;

  const w = new Uint32Array(64);
  for (let i = 0; i < 16; i++) {
    const offset = i * 4;
    w[i] = (block[offset] << 24) | (block[offset + 1] << 16) | (block[offset + 2] << 8) | block[offset + 3];
  }
  for (let i = 16; i < 64; i++) {
    const x = w[i - 15], y = w[i - 2];
    const s0 = rotateRight(x, 7) ^ rotateRight(x, 18) ^ (x >>> 3);
    const s1 = rotateRight(y, 17) ^ rotateRight(y, 19) ^ (y >>> 10);
    w[i] = (w[i - 16] + s0 + w[i - 7] + s1) >>> 0;
  }
  let [a, b, c, d, e, f, g, h] = SHA256_INITIAL;
  for (let i = 0; i < 64; i++) {
    const s1 = rotateRight(e, 6) ^ rotateRight(e, 11) ^ rotateRight(e, 25);
    const choose = (e & f) ^ (~e & g);
    const t1 = (h + s1 + choose + SHA256_K[i] + w[i]) >>> 0;
    const s0 = rotateRight(a, 2) ^ rotateRight(a, 13) ^ rotateRight(a, 22);
    const majority = (a & b) ^ (a & c) ^ (b & c);
    const t2 = (s0 + majority) >>> 0;
    h = g; g = f; f = e; e = (d + t1) >>> 0;
    d = c; c = b; b = a; a = (t1 + t2) >>> 0;
  }
  const firstWord = (SHA256_INITIAL[0] + a) >>> 0;
  return [firstWord >>> 24, (firstWord >>> 16) & 255, (firstWord >>> 8) & 255];
}

/** Exactly 20 decoded bytes, optionally prefixed with 0x/0X. No trimming,
 * coercion, chain, key, metadata, storage, network or randomness. */
export function deriveAccountIcon(address, version = ACCOUNT_ICON_VERSION) {
  if (version !== ACCOUNT_ICON_VERSION || typeof address !== 'string' || (address.length !== 40 && address.length !== 42) || !/^(?:0[xX])?[0-9a-fA-F]{40}$/.test(address)) return null;
  const hex = address.length === 42 ? address.slice(2) : address;
  const bytes = new Uint8Array(20);
  for (let i = 0; i < bytes.length; i++) bytes[i] = parseInt(hex.slice(i * 2, i * 2 + 2), 16);
  const seed = seedPrefix(bytes);
  return Object.freeze({
    version: ACCOUNT_ICON_VERSION,
    palette: seed[0] & 7,
    layout: ((seed[1] << 8) | seed[2]) & 0x3fff,
    shape: (seed[0] >>> 3) & 3,
    rotation: (seed[0] >>> 5) & 3,
  });
}

function validSpec(spec) {
  return spec != null && spec.version === ACCOUNT_ICON_VERSION
    && Number.isInteger(spec.palette) && spec.palette >= 0 && spec.palette < ACCOUNT_ICON_PALETTES.length
    && Number.isInteger(spec.layout) && spec.layout >= 0 && spec.layout <= 0x3fff
    && Number.isInteger(spec.shape) && spec.shape >= 0 && spec.shape <= 3
    && Number.isInteger(spec.rotation) && spec.rotation >= 0 && spec.rotation <= 3;
}

const validSize = (size) => typeof size === 'number' && Number.isFinite(size) && size > 0;

/** Both renderers share the integer geometry and ascending cell order. */
function glyphs(spec) {
  const result = [];
  for (let i = 0; i < 15; i++) {
    if (i !== 0 && !(spec.layout & (1 << (i - 1)))) continue;
    const x = 9 + 12 * (i % 4), y = 9 + 12 * Math.floor(i / 4);
    if (spec.shape === 0) result.push(['rect', { x, y, width: 10, height: 10 }]);
    else if (spec.shape === 1) result.push(['circle', { cx: x + 5, cy: y + 5, r: 5 }]);
    else if (spec.shape === 2) result.push(['path', { d: `M${x + 5} ${y}L${x + 10} ${y + 10}L${x} ${y + 10}Z` }]);
    else result.push(['path', { d: `M${x} ${y}L${x + 10} ${y}A10 10 0 0 1 ${x} ${y + 10}Z` }]);
  }
  return result;
}

/** Canonical UTF-8 SVG, without a trailing newline; invalid specs have no SVG.
 * All interpolated values come from checked numbers and the fixed palette. */
export function accountIconSVG(spec, size = 64) {
  if (!validSpec(spec) || !validSize(size)) return null;
  const body = glyphs(spec).map(([tag, attrs]) => `<${tag} ${Object.entries(attrs).map(([key, value]) => `${key}="${value}"`).join(' ')}/>`).join('');
  return `<svg xmlns="${SVG_NS}" width="${size}" height="${size}" viewBox="0 0 64 64" aria-hidden="true"><rect width="64" height="64" rx="12" fill="${ACCOUNT_ICON_PALETTES[spec.palette]}"/><g fill="${ACCOUNT_ICON_INK}" transform="rotate(${spec.rotation * 90} 32 32)">${body}</g></svg>`;
}

/** Decorative DOM icon, paired by the caller with the authoritative address.
 * Uses SVG DOM APIs only. Invalid addresses get a neutral, unseeded square. */
export function createAccountIcon(address, size = 32, doc = globalThis.document) {
  if (!validSize(size)) throw new RangeError('Invalid account icon size');
  const spec = deriveAccountIcon(address);
  const node = (tag, attrs) => {
    const el = doc.createElementNS(SVG_NS, tag);
    for (const [name, value] of Object.entries(attrs)) el.setAttribute(name, String(value));
    return el;
  };
  const svg = node('svg', { width: size, height: size, viewBox: '0 0 64 64', 'aria-hidden': 'true', focusable: 'false', class: spec ? 'account-icon' : 'account-icon placeholder' });
  svg.append(node('rect', { width: 64, height: 64, rx: 12, fill: spec ? ACCOUNT_ICON_PALETTES[spec.palette] : '#808890' }));
  if (spec) {
    const group = node('g', { fill: ACCOUNT_ICON_INK, transform: `rotate(${spec.rotation * 90} 32 32)` });
    for (const [tag, attrs] of glyphs(spec)) group.append(node(tag, attrs));
    svg.append(group);
  }
  return svg;
}
