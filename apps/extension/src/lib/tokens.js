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

/** Tokens seen so far on a chain, and how far each list was read. `pending`
 * holds addresses whose details could not be confirmed yet (the endpoints
 * disagreed, or only one answered): they are retried on the next scan. */
export function emptyCatalog() {
  return { tokens: {}, pending: {}, rejected: [], factoryRead: 0, pairsRead: 0, launchesRead: 0 };
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

/**
 * Symbol, name and decimals from three readers (thunks, so the pin policy in
 * lib/tokenPin.js can source them its own way); null when `decimals` is
 * missing or absurd.
 */
export async function tokenInfoFrom({ decimals, symbol, name }) {
  let d;
  try {
    d = await uint64At(await decimals());
  } catch (e) {
    if (e?.tokenUnverified) throw e; // an unconfirmed read is not a null token
    return null;
  }
  if (d > 77) return null;
  let sym = '???';
  let nm = '';
  try {
    const s = await stringAt(await symbol());
    if (s) sym = s;
  } catch (e) {
    if (e?.tokenUnverified) throw e;
    /* keep ??? */
  }
  try {
    nm = await stringAt(await name());
  } catch (e) {
    if (e?.tokenUnverified) throw e;
    /* keep empty */
  }
  return { address: '', symbol: sym.slice(0, 16), name: nm.slice(0, 48), decimals: d };
}

/** `tokenInfoFrom` with every field read through one reader, as the scan does. */
export async function tokenInfo(address, read) {
  const info = await tokenInfoFrom({
    decimals: () => read(address, call(SEL.decimals)),
    symbol: () => read(address, call(SEL.symbol)),
    name: () => read(address, call(SEL.name)),
  });
  return info && { ...info, address: address.toLowerCase() };
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
 * addresses that turn out not to be tokens are not asked again. Each address
 * remembers the most specific list it was seen in (the launchpad's own list
 * names its tokens), so a launchpad token keeps its badge after it graduates
 * into a DEX pool.
 */
const ORIGIN_RANK = { seed: 1, pool: 2, dex: 3, launchpad: 4 };

export async function discoverTokens(sources, catalog, read, infoOf = tokenInfo) {
  const cat = { ...catalog, tokens: { ...catalog.tokens }, pending: { ...catalog.pending }, rejected: new Set(catalog.rejected) };
  const found = [
    ...Object.entries(cat.pending).map(([a, origin]) => [a, origin]), // retried every scan
    ...(sources.seed || []).map((a) => [a, 'seed']),
    ...(sources.waeth ? [[sources.waeth, 'seed']] : []),
  ];
  cat.factoryRead = await readList(sources.tokenFactory, SEL.allTokensLength, SEL.allTokens, cat.factoryRead, CAPS.factoryTokens, read, (a) => found.push([a, 'dex']));
  cat.pairsRead = await readList(sources.pairFactory, SEL.allPairsLength, SEL.allPairs, cat.pairsRead, CAPS.pools, read, async (pair) => {
    for (const sel of [SEL.token0, SEL.token1]) {
      try {
        found.push([addressAt(await read(pair, call(sel))), 'pool']);
      } catch { /* this pool's side */ }
    }
  });
  cat.launchesRead = await readList(sources.launchpad, SEL.tokenCount, SEL.tokens, cat.launchesRead, CAPS.launches, read, (a) => found.push([a, 'launchpad']));
  for (const [a, origin] of found) {
    const key = a.toLowerCase();
    const known = cat.tokens[key];
    if (known) {
      if ((ORIGIN_RANK[origin] || 0) > (ORIGIN_RANK[known.origin] || 0)) cat.tokens[key] = { ...known, origin };
    } else if (!cat.rejected.has(key)) {
      let info;
      try {
        info = await infoOf(key, read);
      } catch (e) {
        // Unconfirmed details (the endpoints disagreed, or only one answered):
        // neither catalogued nor rejected — the next scan tries again.
        if (e?.tokenUnverified) {
          cat.pending[key] = origin;
          continue;
        }
        throw e;
      }
      delete cat.pending[key];
      if (info) cat.tokens[key] = { ...info, address: key, origin }; else cat.rejected.add(key);
    }
  }
  return { ...cat, pending: cat.pending, rejected: [...cat.rejected] };
}

/**
 * Read new tokens into `catalog`, then every known token's balance for `owner`.
 * Returns the updated catalog and the non-zero holdings (by symbol). Throws only
 * when a balance cannot be read, as in the app.
 */
export async function scanTokens({ owner, sources, catalog, read, infoOf }) {
  const cat = await discoverTokens(sources, catalog, read, infoOf);
  const held = [];
  for (const token of Object.values(cat.tokens)) {
    const balance = (await uintAt(await read(token.address, call(SEL.balanceOf, wordAddress(owner))))).toString();
    if (balance !== '0') held.push({ token, balance });
  }
  held.sort((a, b) => (a.token.symbol.toLowerCase() + a.token.address).localeCompare(b.token.symbol.toLowerCase() + b.token.address));
  return { catalog: cat, held };
}

// ---- display ----

/** Whole base units with thousands separators, for the confirmation line. */
export function grouped(raw) {
  const s = typeof raw === 'bigint' ? raw.toString() : String(raw);
  return s.replace(/\B(?=(\d{3})+(?!\d))/g, ',');
}

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

// ---- sending a token (mirrors TokenSend.swift's TokenAmount and ERC20) ----

/** "1.5" (at most `decimals` fraction digits) -> base units as a BigInt;
 * leading and trailing spaces are ignored, anything else is rejected. */
export function parseTokenAmount(text, decimals) {
  const s = String(text ?? '').trim();
  const m = /^(\d*)(?:\.(\d*))?$/.exec(s);
  if (!s || !m || (m[1] === '' && !m[2])) throw new Error('enter an amount like 1.5');
  const frac = m[2] || '';
  if (frac.length > decimals) throw new Error(`at most ${decimals} decimals`);
  return BigInt(m[1] || '0') * 10n ** BigInt(decimals) + BigInt(frac.padEnd(decimals, '0') || '0');
}

/** All fraction digits, trailing zeros kept (the Max button fills the exact
 * balance, unlike `formatTokenAmount`'s 6). */
export function formatTokenAmountExact(raw, decimals) {
  const s = typeof raw === 'bigint' ? raw.toString() : String(raw);
  if (!(decimals > 0)) return s.replace(/^0+(?=\d)/, '') || '0';
  const padded = '0'.repeat(Math.max(0, decimals + 1 - s.length)) + s;
  const whole = padded.slice(0, -decimals).replace(/^0+/, '') || '0';
  return `${whole}.${padded.slice(-decimals)}`;
}

/** `transfer(address,uint256)` calldata; `amount` is base units. */
export function erc20TransferCalldata(to, amount) {
  if (!/^0x[0-9a-fA-F]{40}$/.test(String(to || ''))) throw new Error('the recipient is not an address');
  const n = typeof amount === 'bigint' ? amount : BigInt(amount);
  if (n < 0n || n > 2n ** 256n - 1n) throw new Error('the amount does not fit a uint256');
  return `0xa9059cbb${wordAddress(to)}${n.toString(16).padStart(64, '0')}`;
}
