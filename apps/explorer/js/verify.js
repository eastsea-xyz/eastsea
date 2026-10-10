// The verifier an explorer page asks, chosen once at boot (detectVerifier):
// inside the EastSea app's Explore tab the native bridge (window.eastsea.verify,
// apps/wallet/Resources/provider.js — the wallet's Rust verifier, ≈0.68 ms a
// committee certificate where browser wasm takes ≈11.7,
// docs/research/wasm-speed-2026-10-05.md); else the wasm module the extension
// uses (apps/extension/wasm/aether_wasm.js, verifyAccount) when this page can
// load it; else nothing and the page says "not verified". One answer shape
// everywhere — {verified, height, reason} — so the pages have one code path.

import { readVerdict, readSource } from './peers.js';
import { DEFAULT_ENDPOINT } from './rpc.js';

export const NOT_VERIFIED = 'not verified';
export const NOT_COMMITTED = 'not committed';

/** The {verified, height, reason} every verifier answers with. */
export function verdict(verified, height = null, reason = '') {
  return { verified: !!verified, height: Number.isSafeInteger(height) ? height : null, reason: String(reason || '') };
}

/** Legacy blocks without a receipt commitment cannot certify their receipts.
 * Modern receipt proofs are verified by the public reader before display. */
export const notCommitted = () => verdict(false, null, NOT_COMMITTED);

/** `window.eastsea.verify` when the page runs inside the app and the surface
 * is whole — a page must never be half a verifier. */
export function nativeVerifier(win = globalThis) {
  const v = win?.eastsea?.verify;
  return v && ['block', 'account', 'receipt'].every((k) => typeof v[k] === 'function') ? v : null;
}

// Where the wasm module and its network file sit, tried in order: beside the
// explorer (a deployment that carries its own wasm/) and next door in an
// apps/ checkout (apps/extension). In the app bundle neither exists — the
// native path has already answered, so these are never fetched.
const MODULE_URLS = [new URL('../wasm/aether_wasm.js', import.meta.url).href,
  new URL('../../extension/wasm/aether_wasm.js', import.meta.url).href];
const NETWORK_URLS = ['network.json', '../extension/network.json'];

/** The retries mirror the wallet's FFI and the extension's rpc.js: a node
 * answers null for a height until its block is finalized, ~250 ms a poll. */
const POLLS = 40;
const POLL_MS = 250;

async function certifiedAt(node, height, sleep) {
  for (let attempt = 0; attempt < POLLS; attempt++) {
    const answer = await node.call('aether_getFinalized', [height]).catch(() => null);
    if (answer != null) return answer;
    if (attempt < POLLS - 1) await sleep(POLL_MS);
  }
  throw new Error(`block ${height} not finalized yet`);
}

/** The verified-height floor, in the storage the explorer already uses for
 * its endpoint and theme. Storage may be denied outright (some private
 * modes): the floor then stays 0 and verification still runs. */
function readFloor(storage, key) {
  try {
    const v = parseInt(storage?.getItem(key) ?? '0', 10);
    return Number.isSafeInteger(v) && v >= 0 ? v : 0;
  } catch {
    return 0;
  }
}

function writeFloor(storage, key, height) {
  try {
    // Finalized blocks never go back: the floor only rises (extension rpc.js).
    if (height > readFloor(storage, key)) storage.setItem(key, String(height));
  } catch { /* storage denied: the floor stays 0, verification still runs */ }
}

/** An account check through the wasm module — the extension's verified read
 * (src/lib/rpc.js callVerifiedAccount) against this page's one node: fetch
 * the account, anchor its height with a finalized certificate, verify, and
 * only ever raise the stored height floor. Never rejects; a check that ran
 * and failed is a verdict, not an error. */
export async function wasmAccount(node, address, env, displayed = null) {
  try {
    const account = displayed || await node.call('aether_getAccount', [address]);
    const height = account?.height;
    if (!Number.isSafeInteger(height) || height < 0) return verdict(false, null, 'account height is missing');
    const [finalized, status] = await Promise.all([
      certifiedAt(node, height + 1, env.sleep),
      node.call('aether_status'),
    ]);
    const floorKey = `aether-explorer.verifiedHeight.${status.chain_id}`;
    const floor = readFloor(env.storage, floorKey);
    const verified = JSON.parse(String(env.mod.verifyAccount(
      JSON.stringify(env.network), JSON.stringify(status), JSON.stringify(account),
      JSON.stringify(finalized), address, BigInt(floor), BigInt(env.now()),
    )));
    const certified = Number(verified.certified_block);
    if (!Number.isSafeInteger(certified) || certified < floor) return verdict(false, null, 'invalid certified height');
    writeFloor(env.storage, floorKey, certified);
    return verdict(true, certified);
  } catch (e) {
    return verdict(false, null, (e && e.message) || String(e));
  }
}

/** Try to load the wasm verifier. Returns its environment ({mod, network,
 * storage, now, sleep}) or null — a page that cannot reach the module or its
 * network file simply has no wasm verifier. */
export async function loadWasmVerifier(io = {}) {
  const importFn = io.importFn || ((u) => import(u));
  const fetchFn = io.fetch || ((...a) => globalThis.fetch(...a));
  let mod = null;
  for (const url of MODULE_URLS) {
    try {
      mod = await importFn(url);
      break;
    } catch { /* try the next candidate */ }
  }
  if (!mod || typeof mod.verifyAccount !== 'function' || typeof mod.default !== 'function') return null;
  let network = null;
  for (const url of NETWORK_URLS) {
    try {
      const r = await fetchFn(url);
      if (r.ok) {
        network = await r.json();
        break;
      }
    } catch { /* try the next candidate */ }
  }
  if (!network) return null;
  try {
    await mod.default();   // the .wasm sits next to the .js (wasm-pack --target web)
  } catch {
    return null;
  }
  return {
    mod,
    network,
    storage: io.storage ?? (() => { try { return localStorage; } catch { return null; } })(),
    now: io.now || (() => Date.now()),
    sleep: io.sleep || ((ms) => new Promise((resolve) => setTimeout(resolve, ms))),
  };
}

/** A native answer is the bridge's verdict already; a rejection (locked,
 * unauthorized, the bridge gone) is a refused verdict with its message. */
function nativeAsk(promise) {
  return promise.then(
    (v) => (v && typeof v === 'object' && v.verified === true
      ? verdict(true, v.height)
      : verdict(false, null, (v && v.reason) || NOT_VERIFIED)),
    (e) => verdict(false, null, (e && e.message) || NOT_VERIFIED),
  );
}

function nativeDisplayed(native, kind, node, key, displayed) {
  const checked = readVerdict(displayed);
  if (checked) return Promise.resolve(checked);
  // The bridge verifies its own node independently. It cannot certify a
  // different HTTP node's displayed fields at the same height/address. Use
  // the answer's recorded source before the mutable current-source badge.
  const source = readSource(displayed) || node?.source;
  if ((source?.url || node?.url) !== DEFAULT_ENDPOINT) return Promise.resolve(verdict(false, null, 'read from another node'));
  return nativeAsk(Promise.resolve().then(() => native[kind](key)));
}

/** Pick the verifier once, at boot: native when the page is inside the app,
 * else wasm when the module loads, else none. Every kind answers the same
 * three questions and never rejects. */
export async function detectVerifier(win = globalThis, io = {}) {
  const native = nativeVerifier(win);
  if (native) {
    return {
      kind: 'native',
      block: (node, height, displayed) => nativeDisplayed(native, 'block', node, height, displayed),
      account: (node, address, displayed) => nativeDisplayed(native, 'account', node, address, displayed),
      receipt: (node, hash, displayed) => nativeDisplayed(native, 'receipt', node, hash, displayed),
    };
  }
  const env = await loadWasmVerifier(io);
  if (env) {
    return {
      kind: 'wasm',
      env,
      // PublicPeerPool verifies and constructs the exact displayed object.
      // Independently re-fetching a genuine proof must not bless a forged
      // HTTP summary or a different receipt rendered by a concurrent request.
      block: async (_node, _height, displayed) => readVerdict(displayed) || verdict(false, null, NOT_VERIFIED),
      account: (node, address, displayed) => readVerdict(displayed) || wasmAccount(node, address, env, displayed),
      receipt: async (_node, _hash, displayed) => readVerdict(displayed)
        || (typeof env.mod.verifyReceipt === 'function' ? verdict(false, null, NOT_VERIFIED) : notCommitted()),
    };
  }
  return {
    kind: 'none',
    block: async () => verdict(false, null, NOT_VERIFIED),
    account: async () => verdict(false, null, NOT_VERIFIED),
    receipt: notCommitted,
  };
}
