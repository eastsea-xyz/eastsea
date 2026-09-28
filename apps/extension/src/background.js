// The extension's service worker: answers pages (through content.js), opens
// an approval window for anything that needs the user, and serves the popup.
// The origin of a page request always comes from Chrome (the port's sender),
// never from the page.

import init, { accountAddress, prepareTx, attachSignature, publicKeyFromSecret } from '../wasm/aether_wasm.js';
import { Vault, DEFAULT_LOCK_MINUTES } from './lib/vault.js';
import { Rpc, RpcError, DEFAULT_RPCS } from './lib/rpc.js';
import { Wallet } from './lib/wallet.js';
import { CHAIN_HEX, READ_METHODS, SEND_METHODS, normalizeTx, describeCall, originAllowed } from './lib/methods.js';
import { weiToAeth } from './lib/units.js';
import { parseTokenSources, scanTokens } from './lib/tokens.js';
import { TERMS_VERSION } from './lib/terms.js';

const ready = init({ module_or_path: chrome.runtime.getURL('wasm/aether_wasm_bg.wasm') });
const area = (a) => ({
  get: async (k) => (await a.get(k))[k],
  set: (k, v) => a.set({ [k]: v }),
  remove: (k) => a.remove(k),
});
const local = area(chrome.storage.local);
const session = area(chrome.storage.session);
const vault = new Vault({ local, session, addressOf: (pub) => accountAddress(pub) });
const rpc = new Rpc(DEFAULT_RPCS);
const wallet = new Wallet({ wasm: { prepareTx, attachSignature }, rpc, vault });

const UI_PREFIX = chrome.runtime.getURL('ui/');
const ports = new Set(); // content-script ports, for events
const approvals = new Map(); // id -> {origin, kind, tx, resolve, reject, windowId}

const err = (code, message) => Object.assign(new Error(message), { code });
// A restarted worker has no pending approvals: clear what the last one listed.
session.set('approvals', []);
/** At most this many requests from one site wait at once (no approval-window flood). */
const MAX_PENDING_PER_ORIGIN = 3;

async function applySettings() {
  const custom = (await local.get('rpcs')) || [];
  rpc.setUrls([...custom, ...DEFAULT_RPCS]);
}
applySettings();
chrome.storage.onChanged.addListener((c, a) => { if (a === 'local' && c.rpcs) applySettings(); });

// ---- connected sites ----

async function sites() {
  return (await local.get('sites')) || {};
}
async function connectedAddress(origin) {
  const [s, info] = [await sites(), await vault.info()];
  return info && s[origin] && s[origin].address === info.address ? info.address : null;
}
async function setSite(origin, entry) {
  const s = { ...(await sites()) };
  if (entry) s[origin] = entry; else delete s[origin];
  await local.set('sites', s);
  const address = entry ? entry.address : null;
  broadcast(origin, 'accountsChanged', address ? [address] : []);
}
function broadcast(origin, event, data) {
  for (const p of ports) if (!origin || p.origin === origin) p.post({ event, data });
}

// ---- activity ----

async function record(item) {
  const list = ((await local.get('activity')) || []).filter((a) => a.hash !== item.hash);
  await local.set('activity', [item, ...list].slice(0, 50));
}
async function track(hash, base) {
  await record({ ...base, hash, state: 'pending', at: Date.now() });
  const r = await wallet.receipt(hash);
  await record({ ...base, hash, state: r ? (r.ok ? 'done' : 'failed') : 'unknown', height: r?.height, at: Date.now() });
}

// ---- approvals ----

async function publishApprovals() {
  const list = [...approvals.entries()].map(([id, a]) => ({ id, origin: a.origin, kind: a.kind, tx: a.tx, what: a.tx ? describeCall(a.tx) : null, value: a.tx ? weiToAeth(a.tx.value_wei) : null }));
  await session.set('approvals', list);
}

function askUser(origin, kind, tx) {
  const mine = [...approvals.values()].filter((a) => a.origin === origin);
  if (kind === 'connect' && mine.some((a) => a.kind === 'connect')) return Promise.reject(err(-32002, 'A connection request from this site is already waiting.'));
  if (mine.length >= MAX_PENDING_PER_ORIGIN) return Promise.reject(err(-32002, 'Too many requests from this site are waiting for approval.'));
  return new Promise((resolve, reject) => {
    const id = crypto.randomUUID();
    approvals.set(id, { origin, kind, tx, resolve, reject });
    publishApprovals();
    chrome.windows.create({ url: `ui/popup.html?approve=${id}`, type: 'popup', width: 380, height: 640, focused: true }, (w) => {
      const a = approvals.get(id);
      if (!a) return;
      if (!w) { settle(id, (x) => x.reject(err(4001, 'The approval window could not open.'))); return; }
      a.windowId = w.id;
      // Closed before we learned its id: onRemoved could not match it.
      chrome.windows.get(w.id).catch(() => { if (approvals.has(id)) settle(id, (x) => x.reject(err(4001, 'The user closed the approval window.'))); });
    });
  });
}

function settle(id, fn) {
  const a = approvals.get(id);
  if (!a) throw new Error('This request is no longer waiting.');
  approvals.delete(id);
  publishApprovals();
  fn(a);
  return a;
}

chrome.windows.onRemoved.addListener((windowId) => {
  for (const [id, a] of approvals) if (a.windowId === windowId) settle(id, (x) => x.reject(err(4001, 'The user closed the approval window.')));
});

/**
 * The fee cap shown in the approval window, from one `aether_status` snapshot
 * that the signed transaction then uses too (what was shown is what is signed).
 */
async function quote(id) {
  const a = approvals.get(id);
  if (!a || a.kind !== 'send') throw new Error('This request is no longer waiting.');
  a.status = await rpc.call('aether_status', []);
  return Wallet.maxFee(a.status, a.tx.gas || (a.tx.data === '0x' ? 21_000 : 3_000_000)).toString();
}

async function approve(id) {
  const a = approvals.get(id);
  if (!a) throw new Error('This request is no longer waiting.');
  if (!(await vault.unlocked())) throw new Error('Unlock first.');
  if (a.kind === 'send' && !a.status) throw new Error('The network fee is still loading. Try again in a moment.');
  // Claim it before any await, so a second click cannot send it twice.
  approvals.delete(id);
  publishApprovals();
  try {
    const info = await vault.info();
    if (a.kind === 'connect') {
      await setSite(a.origin, { address: info.address, at: Date.now() });
      a.resolve([info.address]);
      return { address: info.address };
    }
    const hash = await wallet.send(a.tx, { status: a.status });
    a.resolve(hash);
    track(hash, { title: describeCall(a.tx), origin: a.origin, value: a.tx.value_wei });
    return { hash };
  } catch (e) {
    // Put it back so the user can retry (after refreshing the fee) or reject.
    delete a.status;
    approvals.set(id, a);
    publishApprovals();
    throw e;
  }
}

// ---- page requests ----

async function pageRequest(origin, method, params = []) {
  if (!originAllowed(origin)) throw err(4100, 'Aether Wallet only talks to https pages (or pages served from this computer).');
  if (method === 'eth_chainId') return CHAIN_HEX;
  if (method === 'eth_accounts' || method === 'aether_accounts') {
    const a = await connectedAddress(origin);
    return a ? [a] : [];
  }
  if (method === 'eth_requestAccounts' || method === 'aether_requestAccounts') {
    const a = await connectedAddress(origin);
    return a ? [a] : askUser(origin, 'connect');
  }
  if (method === 'wallet_disconnect' || method === 'aether_disconnect') {
    await setSite(origin, null);
    return [];
  }
  if (SEND_METHODS.has(method)) {
    const address = await connectedAddress(origin);
    if (!address) throw err(4100, 'Connect this page to Aether Wallet first (eth_requestAccounts).');
    const raw = params[0] || {};
    if (raw.from && raw.from.toLowerCase() !== address.toLowerCase()) throw err(4100, '`from` is not the connected account.');
    let tx;
    try { tx = normalizeTx(raw); } catch (e) { throw err(-32602, e.message); }
    return askUser(origin, 'send', tx);
  }
  if (READ_METHODS.has(method)) return rpc.call(method, params);
  throw err(4200, `Aether Wallet does not support ${method}.`);
}

chrome.runtime.onConnect.addListener((port) => {
  if (port.name !== 'aether-page' || !port.sender?.tab) return port.disconnect();
  const origin = port.sender.origin || new URL(port.sender.url).origin;
  const p = { origin, post: (m) => { try { port.postMessage(m); } catch { /* gone */ } } };
  ports.add(p);
  port.onDisconnect.addListener(() => ports.delete(p));
  port.onMessage.addListener(async (m) => {
    if (!m || typeof m.id !== 'string' || typeof m.method !== 'string') return;
    try {
      await ready;
      p.post({ id: m.id, result: await pageRequest(origin, m.method, Array.isArray(m.params) ? m.params : []) });
    } catch (e) {
      p.post({ id: m.id, error: { code: e.code ?? -32603, message: e.message || String(e), data: e.data } });
    }
  });
});

// ---- assets (AETH + ERC-20 tokens, read from the node) ----

let sourcesFile = null; // the bundled token-sources.json, fetched once
async function tokenSources(chainId) {
  if (!sourcesFile) sourcesFile = await (await fetch(chrome.runtime.getURL('token-sources.json'))).json();
  return parseTokenSources(sourcesFile, chainId);
}

/**
 * The token scan runs in the worker, throttled like the app's: `force` (the
 * Assets view opens) re-reads after 5 s, otherwise every 30 s. The catalog and
 * the last holdings live in local storage, so they survive worker restarts and
 * are shown until the next read.
 */
async function refreshAssets({ force = false } = {}) {
  const info = await vault.info();
  const cached = (await local.get('assets')) || {};
  if (!info) return { tokens: [], updated: cached.updated || null, error: null };
  if (cached.address === info.address) {
    const minAge = force ? 5_000 : 30_000;
    if (Date.now() - (cached.updated || 0) < minAge) return { tokens: cached.holdings || [], updated: cached.updated || null, error: cached.error || null };
  }
  if (!refreshAssets.running) {
    refreshAssets.running = (async () => {
      const owner = info.address;
      const out = { address: owner, updated: Date.now(), holdings: cached.address === owner ? cached.holdings || [] : [], error: cached.address === owner ? cached.error || null : null };
      try {
        await local.set('assets', out); // shown as "last read" while scanning
        const status = await rpc.call('aether_status', []);
        const sources = await tokenSources(status.chain_id);
        if (sources) {
          const key = `tokenCatalog.${status.chain_id}`;
          const catalog = (await local.get(key)) || { tokens: {}, rejected: [], factoryRead: 0, pairsRead: 0, launchesRead: 0 };
          const read = (to, data) => rpc.call('eth_call', [{ to, data }, 'latest']);
          const { catalog: cat, held } = await scanTokens({ owner, sources, catalog, read });
          await local.set(key, cat);
          await local.set('assets', { address: owner, updated: Date.now(), holdings: held, error: null });
        } else {
          await local.set('assets', { address: owner, updated: Date.now(), holdings: [], error: null });
        }
      } catch (e) {
        // Try again on the normal cadence, not every poll.
        await local.set('assets', { address: owner, updated: Date.now(), holdings: out.holdings, error: e.message || String(e) }).catch(() => {});
      } finally {
        refreshAssets.running = null;
      }
    })();
  }
  await refreshAssets.running;
  const fresh = (await local.get('assets')) || {};
  return { tokens: fresh.address === info.address ? fresh.holdings || [] : [], updated: fresh.updated || null, error: fresh.error || null };
}

// ---- the popup and approval window ----

async function state() {
  const info = await vault.info();
  return {
    exists: Boolean(info),
    unlocked: info ? await vault.unlocked() : false,
    address: info?.address || null,
    approvals: (await session.get('approvals')) || [],
    lockMinutes: (await local.get('lockMinutes')) || DEFAULT_LOCK_MINUTES,
    rpcs: (await local.get('rpcs')) || [],
    terms: (await local.get('termsVersion')) || 0,
  };
}

const ui = {
  state,
  create: ({ password }) => vault.create(password),
  importKey: ({ secret, password }) => vault.importSecret(secret, password, publicKeyFromSecret),
  unlock: ({ password }) => vault.unlock(password),
  lock: () => vault.lock(),
  approve: ({ id }) => approve(id),
  reject: ({ id }) => { settle(id, (x) => x.reject(err(4001, 'The user rejected the request.'))); return true; },
  account: async () => {
    const info = await vault.info();
    const [balance, status] = await Promise.all([wallet.balance(info.address), rpc.call('aether_status', [])]);
    return { address: info.address, balance: balance.toString(), height: status.height, blockAt: status.timestamp_ms, node: rpc.current };
  },
  assets: ({ force } = {}) => refreshAssets({ force }),
  send: async ({ to, value_wei }) => {
    if (!(await vault.unlocked())) throw new Error('Unlock first.');
    const tx = normalizeTx({ to, value: value_wei });
    const hash = await wallet.send(tx);
    track(hash, { title: 'Send AETH', origin: 'Aether Wallet', value: tx.value_wei, to });
    return { hash };
  },
  faucet: async () => {
    const info = await vault.info();
    const hash = await wallet.faucet(info.address);
    track(hash, { title: 'Test AETH from the faucet', origin: 'Aether Wallet' });
    return { hash };
  },
  quote: ({ id }) => quote(id),
  activity: async () => (await local.get('activity')) || [],
  sites: async () => sites(),
  disconnect: ({ origin }) => setSite(origin, null),
  settings: async ({ lockMinutes, rpcs }) => {
    if (lockMinutes !== undefined) await local.set('lockMinutes', Math.min(Math.max(Number(lockMinutes) || DEFAULT_LOCK_MINUTES, 1), 24 * 60));
    if (rpcs !== undefined) {
      const list = rpcs.filter((u) => { try { return ['http:', 'https:'].includes(new URL(u).protocol); } catch { return false; } });
      await local.set('rpcs', list);
    }
    return state();
  },
  reveal: ({ password }) => vault.revealSecret(password),
  acceptTerms: async () => {
    await local.set('termsVersion', TERMS_VERSION);
    return state();
  },
  erase: async ({ password }) => {
    await vault.decrypt(password);
    await vault.erase();
    await local.set('sites', {});
    broadcast(null, 'accountsChanged', []);
    return true;
  },
};

chrome.runtime.onMessage.addListener((m, sender, reply) => {
  // Only this extension's own pages drive the wallet; content scripts cannot.
  if (sender.id !== chrome.runtime.id || !sender.url?.startsWith(UI_PREFIX) || !m || !ui[m.op]) return false;
  ready
    .then(() => ui[m.op](m.args || {}))
    .then((result) => reply({ ok: true, result }), (e) => reply({ ok: false, error: e.message || String(e) }));
  return true;
});

// The lock timer: expired sessions are cleared once a minute.
chrome.alarms.create('lock', { periodInMinutes: 1 });
chrome.alarms.onAlarm.addListener((a) => { if (a.name === 'lock') vault.unlocked(); });
