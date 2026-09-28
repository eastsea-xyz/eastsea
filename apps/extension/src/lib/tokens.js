// ERC-20 tokens this wallet holds, found the way the app finds them
// (apps/wallet/Sources/TokenAssets.swift): the DEX token factory, its pools and
// the launchpad are enumerated with read-only `eth_call`s, then `balanceOf` is
// asked for each token. Unlike the app's AETH, these balances are the node's
// answer, not verified in the browser. Pure functions with the reader passed
// in, so it is tested without a node (test/tokens.test.mjs).

/** Selectors (`forge inspect <Contract> methodIdentifiers`), as in the app. */
export const SEL = {
  allTokensLength: 'dbb80e42', allTokens: '634282af',
  allPairsLength: '574f2ba3', allPairs: '1e3dd18b', token0: '0dfe1681', token1: 'd21220a7',
  tokenCount: '9f181b5e', tokens: '4f64b2be', // launchpad (CurveLaunch)
  symbol: '95d89b41', name: '06fdde03', decimals: '313ce567', balanceOf: '70a08231',
};

/** Caps per list, as in the app. */
export const CAPS = { factoryTokens: 500, pools: 200, launches: 500 };

const decoder = new TextDecoder();

/** The `chains[chainId]` entry of a bundled token-sources.json, or null. */
export function parseTokenSources(file, chainId) {
  try {
    const chains = (typeof file === 'string' ? JSON.parse(file) : file)?.chains;
    const c = chains?.[String(chainId)];
    return c ? { seed: [], ...c } : null;
  } catch {
    return null;
  }
}

/** Tokens seen so far on a chain, and how far each list was read. */
export function emptyCatalog() {
  return { tokens: {}, rejected: [], factoryRead: 0, pairsRead: 0, launchesRead: 0 };
}

// ---- the few Solidity ABI pieces the scan needs ----

export function call(sel, ...words) {
  return `0x${sel}${words.join('')}`;
}

/** An address as one 32-byte argument word. */
export function wordAddress(a) {
  const h = String(a).toLowerCase().replace(/^0x/, '');
  return h.padStart(64, '0');
}

/** A small integer as one 32-byte argument word. */
export function wordUint(n) {
  return n.toString(16).padStart(64, '0');
}

function words(data) {
  const h = String(data).replace(/^0x/, '');
  if (!h || h.length % 64 !== 0 || !/^[0-9a-fA-F]+$/.test(h)) throw new Error('bad answer from the node');
  return h.match(/.{64}/g);
}

/** Word `i` as a BigInt (exact, any size). */
export function uintAt(data, i = 0) {
  const w = words(data);
  if (i >= w.length) throw new Error('bad answer from the node');
  return BigInt(`0x${w[i]}`);
}

/** Word `i` as a number; throws unless the word is a plain uint64. */
export function uint64At(data, i = 0) {
  const w = words(data);
  if (i >= w.length || !/^0{48}/.test(w[i])) throw new Error('bad answer from the node');
  return Number(BigInt(`0x${w[i]}`));
}

/** Word `i` as a lowercase 0x address. */
export function addressAt(data, i = 0) {
  const w = words(data);
  if (i >= w.length) throw new Error('bad answer from the node');
  return `0x${w[i].slice(24)}`;
}

/** A dynamic `string` return value. */
export function stringAt(data) {
  const w = words(data);
  const off = uint64At(data, 0);
  if (off % 32 !== 0 || off / 32 >= w.length) throw new Error('bad answer from the node');
  const at = off / 32;
  const n = uint64At(data, at);
  if (n > 4096) throw new Error('bad answer from the node');
  const hex = w.slice(at + 1).join('').slice(0, n * 2);
  if (hex.length < n * 2) throw new Error('bad answer from the node');
  return decoder.decode(new Uint8Array((hex.match(/.{2}/g) || []).map((b) => parseInt(b, 16)))).replace(/[\u0000-\u001f\u007f]/g, '');
}

// ---- discovery and balances ----

/** Symbol, name and decimals; null when `decimals` is missing or absurd. */
export async function tokenInfo(address, read) {
  let d;
  try {
    d = await uint64At(await read(address, call(SEL.decimals)));
  } catch {
    return null;
  }
  if (d > 77) return null;
  let symbol = '???';
  let name = '';
  try {
    const s = await stringAt(await read(address, call(SEL.symbol)));
    if (s) symbol = s;
  } catch { /* keep ??? */ }
  try {
    name = await stringAt(await read(address, call(SEL.name)));
  } catch { /* keep empty */ }
  return { address: address.toLowerCase(), symbol: symbol.slice(0, 16), name: name.slice(0, 48), decimals: d };
}

/**
 * Entries `from..<min(length, cap)` of an on-chain address list; returns how far
 * it got. A failure stops that list where it is (it resumes next time).
 */
async function readList(contract, countSel, itemSel, from, cap, read, each) {
  if (!contract) return from;
  let n;
  try {
    n = await uint64At(await read(contract, call(countSel)));
  } catch {
    return from;
  }
  let i = from;
  while (i < Math.min(n, cap)) {
    let a;
    try {
      a = await addressAt(await read(contract, call(itemSel, wordUint(i))));
    } catch {
      break;
    }
    await each(a);
    i += 1;
  }
  return i;
}

/**
 * Enumerate the lists from where they were last read. Returns a new catalog;
 * addresses that turn out not to be tokens are not asked again.
 */
export async function discoverTokens(sources, catalog, read) {
  const cat = { ...catalog, tokens: { ...catalog.tokens }, rejected: new Set(catalog.rejected) };
  const found = [...(sources.seed || []), ...(sources.waeth ? [sources.waeth] : [])];
  cat.factoryRead = await readList(sources.tokenFactory, SEL.allTokensLength, SEL.allTokens, cat.factoryRead, CAPS.factoryTokens, read, (a) => found.push(a));
  cat.pairsRead = await readList(sources.pairFactory, SEL.allPairsLength, SEL.allPairs, cat.pairsRead, CAPS.pools, read, async (pair) => {
    for (const sel of [SEL.token0, SEL.token1]) {
      try {
        found.push(addressAt(await read(pair, call(sel))));
      } catch { /* this pool's side */ }
    }
  });
  cat.launchesRead = await readList(sources.launchpad, SEL.tokenCount, SEL.tokens, cat.launchesRead, CAPS.launches, read, (a) => found.push(a));
  for (const a of new Set(found.map((x) => x.toLowerCase()))) {
    if (cat.tokens[a] || cat.rejected.has(a)) continue;
    const info = await tokenInfo(a, read);
    if (info) cat.tokens[a] = info; else cat.rejected.add(a);
  }
  return { ...cat, rejected: [...cat.rejected] };
}

/**
 * Read new tokens into `catalog`, then every known token's balance for `owner`.
 * Returns the updated catalog and the non-zero holdings (by symbol). Throws only
 * when a balance cannot be read, as in the app.
 */
export async function scanTokens({ owner, sources, catalog, read }) {
  const cat = await discoverTokens(sources, catalog, read);
  const held = [];
  for (const token of Object.values(cat.tokens)) {
    const balance = (await uintAt(await read(token.address, call(SEL.balanceOf, wordAddress(owner))))).toString();
    if (balance !== '0') held.push({ token, balance });
  }
  held.sort((a, b) => (a.token.symbol.toLowerCase() + a.token.address).localeCompare(b.token.symbol.toLowerCase() + b.token.address));
  return { catalog: cat, held };
}

// ---- display ----

/** Base units -> display text, at most 6 fraction digits with trailing zeros
 * dropped (TokenUnits.format in the app): exact for whole numbers, "<0.000001"
 * for a non-zero amount too small to show. */
export function formatTokenAmount(raw, decimals) {
  const s = typeof raw === 'bigint' ? raw.toString() : String(raw);
  if (!(decimals > 0) || s === '0') return s;
  const padded = '0'.repeat(Math.max(0, decimals + 1 - s.length)) + s;
  const whole = padded.slice(0, -decimals).replace(/^0+/, '') || '0';
  const frac = padded.slice(-decimals).slice(0, 6).replace(/0+$/, '');
  if (!frac) return whole === '0' ? '<0.000001' : whole;
  return `${whole}.${frac}`;
}
