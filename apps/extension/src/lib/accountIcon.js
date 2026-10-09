// Archipelago v3. This is the canonical module; mirror it with
// `node scripts/sync-account-icons.mjs`. See docs/design/46-account-icon.md.
export const ACCOUNT_ICON_VERSION = 3;
export const ACCOUNT_ICON_PALETTES = Object.freeze([
  {
    "name": "tidal",
    "start": "#209792",
    "end": "#1d8781",
    "ink": "#0d2135"
  },
  {
    "name": "coral",
    "start": "#ca2b2b",
    "end": "#b92727",
    "ink": "#eed7a0"
  },
  {
    "name": "cove",
    "start": "#2f62da",
    "end": "#2558d0",
    "ink": "#eed7a0"
  },
  {
    "name": "seagrass",
    "start": "#429c1c",
    "end": "#3b8b18",
    "ink": "#0d2135"
  },
  {
    "name": "anemone",
    "start": "#c7237e",
    "end": "#b62073",
    "ink": "#eed7a0"
  },
  {
    "name": "gold",
    "start": "#aa8518",
    "end": "#987716",
    "ink": "#0d2135"
  },
  {
    "name": "azure",
    "start": "#298ee0",
    "end": "#1f84d6",
    "ink": "#0d2135"
  },
  {
    "name": "reef",
    "start": "#257e52",
    "end": "#206f47",
    "ink": "#eed7a0"
  },
  {
    "name": "rose",
    "start": "#d36979",
    "end": "#cf596b",
    "ink": "#0d2135"
  },
  {
    "name": "kelp",
    "start": "#6e7722",
    "end": "#5f671e",
    "ink": "#eed7a0"
  },
  {
    "name": "orchid",
    "start": "#e444d4",
    "end": "#e232d0",
    "ink": "#0d2135"
  },
  {
    "name": "sea",
    "start": "#257793",
    "end": "#216a83",
    "ink": "#eed7a0"
  },
  {
    "name": "dawn",
    "start": "#df6320",
    "end": "#cd5b1d",
    "ink": "#0d2135"
  },
  {
    "name": "iris",
    "start": "#a029e0",
    "end": "#961fd6",
    "ink": "#eed7a0"
  },
  {
    "name": "copper",
    "start": "#96612c",
    "end": "#865727",
    "ink": "#eed7a0"
  },
  {
    "name": "lilac",
    "start": "#9579d8",
    "end": "#8969d3",
    "ink": "#0d2135"
  }
].map((palette) => Object.freeze(palette)));

// Retain the v2 seed: this polish preserves colors and the 16 px coastline.
const DOMAIN = 'eastsea-account-icon-v2';
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
    palette: seed[0] & 15,
    layout: ((seed[1] << 8) | seed[2]) & 0x3fff,
    shape: (seed[0] >>> 4) & 3,
    rotation: (seed[0] >>> 6) & 3,
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

// The broad coastline is the identity at a glance. These are deliberately
// different topologies, not a grid of small glyphs. M/L/C/Z are integer-only
// commands, shared with Swift's Canvas and the independent Python oracle.
export const ACCOUNT_ICON_SILHOUETTES = Object.freeze([
  ['cove', 'M 52 12 C 36 4 13 10 10 28 C 7 45 25 55 43 48 L 47 38 C 33 44 21 40 22 29 C 23 19 35 15 48 23 Z'],
  ['headland', 'M 12 47 L 12 34 C 22 32 19 18 29 11 C 37 5 51 12 53 24 C 55 35 44 41 35 38 C 29 36 29 48 22 50 Z'],
  ['sandbar', 'M 10 40 C 13 29 21 28 29 26 C 35 24 37 10 48 10 L 55 21 C 44 22 46 35 35 38 C 27 41 21 38 17 51 Z'],
  ['twin peaks', 'M 9 43 L 19 15 C 21 9 25 9 28 17 L 33 29 L 42 12 C 45 7 48 9 50 17 L 56 43 C 42 51 24 51 9 43 Z'],
  ['reef', 'M 8 32 L 25 9 C 28 6 32 8 32 13 L 29 24 L 50 16 C 56 14 58 20 53 25 L 35 48 C 31 54 26 51 28 45 L 32 34 L 13 42 C 7 45 5 39 8 32 Z'],
  ['breaker', 'M 8 43 C 16 39 17 18 31 11 C 44 4 56 14 54 27 C 48 18 36 17 34 28 C 40 26 51 32 56 43 C 40 52 22 51 8 43 Z'],
  ['inlet', 'M 10 48 L 10 26 C 10 6 52 6 52 26 L 52 48 L 40 48 L 40 29 C 40 21 22 21 22 29 L 22 48 Z'],
  ['delta', 'M 27 50 L 25 32 L 9 20 L 14 9 L 31 22 L 48 9 L 56 18 L 39 34 L 40 50 Z'],
  ['spit', 'M 11 48 C 9 26 23 9 51 10 C 49 33 33 49 11 48 Z'],
  ['shelf', 'M 10 15 L 32 10 L 33 25 L 52 20 L 55 40 L 39 50 L 12 45 C 8 34 8 25 10 15 Z'],
  ['hook', 'M 11 10 L 25 10 L 25 32 C 25 45 43 44 43 32 L 43 22 L 55 22 L 55 35 C 55 59 11 58 11 35 Z'],
  ['crescent', 'M 50 8 C 23 4 8 19 10 36 C 12 52 33 58 52 45 C 30 43 29 23 50 8 Z'],
  ['ridge', 'M 8 42 L 14 27 L 25 30 L 31 9 L 43 24 L 51 18 L 57 42 C 39 51 24 50 8 42 Z'],
  ['estuary', 'M 10 13 L 24 11 L 32 28 L 41 10 L 55 15 L 42 33 L 51 47 L 35 51 L 28 39 L 13 48 L 8 34 L 23 29 Z'],
  ['arch', 'M 8 44 C 9 27 18 8 32 8 C 46 8 55 27 56 44 L 42 47 C 42 33 37 24 32 24 C 27 24 22 33 22 47 Z'],
  ['tidal pool', 'M 54 27 C 54 47 38 55 21 48 C 6 41 8 18 23 11 C 39 3 51 13 46 27 C 42 37 30 39 25 29 C 29 32 36 29 35 23 C 34 16 21 21 21 31 C 21 43 43 42 43 29 Z'],
].map(([name, path]) => Object.freeze({ name, path })));

export function accountIconSilhouette(spec) {
  return validSpec(spec) ? spec.shape * 4 + (spec.layout & 3) : null;
}

// Only at >=32 logical pixels: an elongated island and a smaller offset reef.
// Unequal areas and staggered shores prevent paired eyes at every rotation.
function islands(spec, size) {
  const result = [{ d: ACCOUNT_ICON_SILHOUETTES[accountIconSilhouette(spec)].path, transform: `translate(0 ${size < 32 ? 5 : 0}) scale(1 0.8)` }];
  if (size < 32) return result;
  for (let i = 0; i < 2; i++) {
    const bits = (spec.layout >>> (2 + i * 6)) & 63;
    const x = (i === 0 ? 8 : 40) + (bits & 3), y = (i === 0 ? 44 : 5) + ((bits >>> 2) & 3);
    const w = (i === 0 ? 21 : 10) + ((bits >>> 4) & 3);
    const d = i === 0
      ? `M ${x} ${y + 4} C ${x + 2} ${y - 3} ${x + w - 5} ${y + 1} ${x + w - 2} ${y - 2} L ${x + w} ${y + 6} C ${x + w - 3} ${y + 11} ${x + 3} ${y + 13} ${x} ${y + 4} Z`
      : `M ${x} ${y + 2} C ${x + 3} ${y - 1} ${x + w - 4} ${y - 2} ${x + w} ${y + 1} L ${x + w - 2} ${y + 5} C ${x + 3} ${y + 7} ${x + 1} ${y + 5} ${x} ${y + 2} Z`;
    result.push({ d });
  }
  return result;
}

function drawing(spec, size) {
  const palette = ACCOUNT_ICON_PALETTES[spec.palette];
  // Identical IDs are safe: they always resolve to identical fixed gradients.
  const id = `eastsea-island-v3-${spec.palette}`;
  return [
    ['defs', {}, [['linearGradient', { id, x1: 0, y1: 0, x2: 64, y2: 64, gradientUnits: 'userSpaceOnUse', 'color-interpolation': 'sRGB' }, [
      ['stop', { offset: 0, 'stop-color': palette.start }],
      ['stop', { offset: 1, 'stop-color': palette.end }],
    ]]]],
    ['rect', { width: 64, height: 64, rx: 12, fill: `url(#${id})` }],
    ['g', { fill: palette.ink, transform: `rotate(${spec.rotation * 90} 32 32)` }, islands(spec, size).map((attrs) => ['path', attrs])],
  ];
}

/** Canonical UTF-8 SVG, without a trailing newline; invalid specs have no SVG.
 * Interpolated values come from checked numbers and frozen drawing tables. */
export function accountIconSVG(spec, size = 64) {
  if (!validSpec(spec) || !validSize(size)) return null;
  const serialize = ([tag, attrs, children]) => {
    const attributes = Object.entries(attrs).map(([key, value]) => `${key}="${value}"`).join(' ');
    const opening = `<${tag}${attributes ? ` ${attributes}` : ''}`;
    return children ? `${opening}>${children.map(serialize).join('')}</${tag}>` : `${opening}/>`;
  };
  return `<svg xmlns="${SVG_NS}" width="${size}" height="${size}" viewBox="0 0 64 64" aria-hidden="true">${drawing(spec, size).map(serialize).join('')}</svg>`;
}

/** Decorative DOM icon, paired by the caller with the authoritative address.
 * Uses SVG DOM APIs only. Invalid addresses get a neutral, unseeded square. */
export function createAccountIcon(address, size = 32, doc = globalThis.document) {
  if (!validSize(size)) throw new RangeError('Invalid account icon size');
  const spec = deriveAccountIcon(address);
  const node = ([tag, attrs, children]) => {
    const el = doc.createElementNS(SVG_NS, tag);
    for (const [name, value] of Object.entries(attrs)) el.setAttribute(name, String(value));
    if (children) for (const child of children) el.append(node(child));
    return el;
  };
  const svg = node(['svg', { width: size, height: size, viewBox: '0 0 64 64', 'aria-hidden': 'true', focusable: 'false', class: spec ? 'account-icon' : 'account-icon placeholder' }]);
  for (const element of spec ? drawing(spec, size) : [['rect', { width: 64, height: 64, rx: 12, fill: '#808890' }]]) svg.append(node(element));
  return svg;
}
