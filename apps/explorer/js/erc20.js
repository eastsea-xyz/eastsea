// ERC-20 metadata and where a token came from, found the way the wallet finds
// it (apps/extension/src/lib/tokens.js): the DEX token factory, its pools and
// the launchpad are enumerated with read-only `eth_call`s. The "unverified"
// labels mirror the app's zero-trust inbox policy (docs/design/09-wallet.md):
// a token the launchpad listed is a launchpad token even after it graduates
// into a pool, and nothing here is a statement about the token's legitimacy.
// Pure functions with the reader passed in (test/erc20.test.mjs).

import { SEL, call, wordAddress, wordUint, addressAt, stringAt, uint64At, uintAt } from './abi.js';

/** Caps per list, as in the app. They bound the page's scan, not the chain. */
export const CAPS = { factoryTokens: 500, pools: 200, launches: 500 };

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

/** The official list proper: the bundled seed tokens and WAETH, lowercase. */
export function officialAddresses(sources) {
  return [...(sources?.seed || []), ...(sources?.waeth ? [sources.waeth] : [])].map((a) => String(a).toLowerCase());
}

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
  return { address: String(address).toLowerCase(), symbol: symbol.slice(0, 16), name: name.slice(0, 48), decimals: d };
}

/** Total supply in base units, or null when the contract does not answer. */
export async function totalSupply(address, read) {
  try {
    return await uintAt(await read(address, call(SEL.totalSupply)));
  } catch {
    return null;
  }
}

/** Entries `0..<min(length, cap)` of an on-chain address list; stops at the
 * first miss. Returns how many it read (the caller decides what that means). */
async function readList(contract, countSel, itemSel, cap, read, each) {
  if (!contract) return 0;
  let n;
  try {
    n = await uint64At(await read(contract, call(countSel)));
  } catch {
    return 0;
  }
  let i = 0;
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
 * Where a token comes from, for the badge. The official list settles it first
 * (an official token stays official however many DEX lists it later joined —
 * the wallet's main list is by address, not by discovery). Then the wallet's
 * origin priority: launchpad > dex > pool — a launchpad token keeps its badge
 * after it graduates into a pool, so the scan goes in that order and stops at
 * the first hit. Every miss costs the whole list, hence the caps.
 */
export async function tokenOrigin(address, sources, read, caps = CAPS) {
  const a = String(address).toLowerCase();
  if (officialAddresses(sources).includes(a)) return 'seed';
  const list = { launchpad: [sources?.launchpad, SEL.tokenCount, SEL.tokens, caps.launches],
    dex: [sources?.tokenFactory, SEL.allTokensLength, SEL.allTokens, caps.factoryTokens] };
  // An unread list (contract missing on this chain, or a read error before
  // item 0) reads as empty; the scan simply moves on to the next one.
  for (const [origin, [contract, countSel, itemSel, cap]] of Object.entries(list)) {
    let found = false;
    await readList(contract, countSel, itemSel, cap, read, (x) => { if (String(x).toLowerCase() === a) found = true; });
    if (found) return origin;
  }
  let inPool = false;
  await readList(sources?.pairFactory, SEL.allPairsLength, SEL.allPairs, caps.pools, read, async (pair) => {
    if (inPool) return;
    for (const sel of [SEL.token0, SEL.token1]) {
      try {
        if ((await addressAt(await read(pair, call(sel)))).toLowerCase() === a) inPool = true;
      } catch { /* this pool's side */ }
    }
  });
  if (inPool) return 'pool';
  return null;
}

/** The badge a token's origin gets — same wording as the wallet's, so a token
 * reads the same here as it does in the app. */
export function originBadge(origin) {
  switch (origin) {
    case 'seed': return { text: 'Official list', kind: 'good' };
    case 'launchpad': return { text: 'Launchpad · unverified', kind: 'warn' };
    case 'dex': return { text: 'DEX-listed · unverified', kind: 'warn' };
    case 'pool': return { text: 'In a DEX pool · unverified', kind: 'warn' };
    default: return { text: 'Not in any list · unverified', kind: 'warn' };
  }
}

// ---- impersonation (apps/extension/src/lib/safety.js) ----

/** Ligatures NFKD leaves alone; the rest folds by decomposition. */
const LIGATURES = { 'æ': 'ae', 'ø': 'o', 'đ': 'd', 'ł': 'l', 'ß': 'ss' };

/** Lowercase ASCII alphanumerics only, so look-alike checks see through
 * "ÆTH", "aeth " and zero-width tricks. */
export function normalized(s) {
  const folded = String(s ?? '')
    .toLowerCase()
    .replace(/[æøđłß]/g, (c) => LIGATURES[c] || c)
    .normalize('NFKD')
    .replace(/[\u0300-\u036f]/g, '');
  return (folded.match(/[a-z0-9]/g) || []).join('');
}

export function editDistance(a, b) {
  const x = [...a], y = [...b];
  let prev = [...Array(y.length + 1).keys()];
  for (let i = 1; i <= x.length; i += 1) {
    const cur = [i];
    for (let j = 1; j <= y.length; j += 1) {
      cur[j] = Math.min(prev[j] + 1, cur[j - 1] + 1, prev[j - 1] + (x[i - 1] === y[j - 1] ? 0 : 1));
    }
    prev = cur;
  }
  return prev[y.length];
}

function resembles(a, b) {
  if (a === b) return true;
  if (b === 'aeth' && a.includes('aeth')) return true;
  return Math.min(a.length, b.length) >= 3 && editDistance(a, b) <= 1;
}

/** Does this token's symbol or name equal or resemble an official one? */
export function looksLikeOfficial(token, official) {
  const s = normalized(token?.symbol);
  const n = normalized(token?.name);
  for (const o of official || []) {
    const os = normalized(o.symbol);
    const on = normalized(o.name);
    if (!os) continue;
    if (resembles(s, os) || (on && (resembles(s, on) || resembles(n, os)))) return true;
  }
  return false;
}

/** The official tokens' metadata, for the impersonation check (skips any the
 * node will not read; the check runs on what it got). */
export async function officialTokens(sources, read) {
  const out = [];
  for (const a of officialAddresses(sources)) {
    const info = await tokenInfo(a, read);
    if (info) out.push(info);
  }
  return out;
}

/** `balanceOf` in base units; null when the call fails. */
export async function balanceOf(token, owner, read) {
  try {
    return await uintAt(await read(token, call(SEL.balanceOf, wordAddress(owner))));
  } catch {
    return null;
  }
}
