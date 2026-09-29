import { Brand } from './lib/brand.js';
// The extension's service worker: answers pages (through content.js), opens
// an approval window for anything that needs the user, and serves the popup.
// The origin of a page request always comes from Chrome (the port's sender),
// never from the page.

import init, { accountAddress, prepareTx, attachSignature, publicKeyFromSecret } from '../wasm/aether_wasm.js';
import { Vault, DEFAULT_LOCK_MINUTES } from './lib/vault.js';
import { Rpc, RpcError, DEFAULT_RPCS } from './lib/rpc.js';
import { networkSettings } from './lib/network.js';
import { Wallet } from './lib/wallet.js';
import { READ_METHODS, SEND_METHODS, normalizeTx, describeCall, originAllowed } from './lib/methods.js';
import { weiToAeth } from './lib/units.js';
import { parseTokenSources, scanTokens, formatTokenAmount, call, SEL, wordAddress, uintAt } from './lib/tokens.js';
import { addressRisk, revertReason, splitHoldings, tokenShort } from './lib/safety.js';
import { TERMS_VERSION } from './lib/terms.js';
import { linkedAddress, describeHistory, mergeHistory } from './lib/history.js';

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

let defaultNetwork;
let activeNetwork;
const configured = (async () => {
  defaultNetwork = await (await fetch(chrome.runtime.getURL('network.json'))).json();
  for (const key of ['activity', 'assets']) {
    const old = await local.get(key);
    if (old !== undefined && await local.get(`${key}.7780`) === undefined) await local.set(`${key}.7780`, old);
    if (old !== undefined) await local.remove(key);
  }
  await applySettings();
})();
async function applySettings() {
  const next = networkSettings(defaultNetwork, {
    developerMode: await local.get('developerMode'),
    developmentNetwork: await local.get('developmentNetwork'),
    developmentPort: (await local.get('developmentPort')) || 18546,
    rpcs: (await local.get('rpcs')) || [],
  });
  const switched = activeNetwork && activeNetwork.chainId !== next.chainId;
  activeNetwork = next;
  rpc.setChain(next.chainId, next.urls);
  if (switched) {
    wallet.lastNonce = null;
    for (const [id, pending] of approvals) {
      approvals.delete(id);
      pending.reject(err(4901, 'The wallet network changed. Please try again.'));
    }
    publishApprovals();
    broadcast(null, 'chainChanged', `0x${next.chainId.toString(16)}`);
  }
}
chrome.storage.onChanged.addListener((c, a) => {
  if (a === 'local' && ['rpcs', 'developerMode', 'developmentNetwork', 'developmentPort'].some((key) => c[key])) configured.then(applySettings);
});
const activityKey = () => `activity.${rpc.chainId}`;
const assetsKey = () => `assets.${rpc.chainId}`;

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

async function record(item, chainId = rpc.chainId) {
  const key = `activity.${chainId}`;
  const list = ((await local.get(key)) || []).filter((a) => a.hash !== item.hash);
  await local.set(key, [item, ...list].slice(0, 50));
}

async function activityPage(cursors = null) {
  const info = await vault.info();
  const own = info?.address;
  const linked = (await local.get('linkedWallets')) || [];
  const addresses = own ? [own, ...linked] : linked;
  const selected = cursors ? addresses.filter((a) => Object.hasOwn(cursors, a)) : addresses;
  let sources = {}, catalog = {};
  try {
    const status = await rpc.call('aether_status', []);
    sources = (await tokenSources(status.chain_id)) || {};
    catalog = ((await local.get(`tokenCatalog.${status.chain_id}`)) || {}).tokens || {};
  } catch { /* the local pending list still opens without a node */ }
  const pages = await Promise.all(selected.map(async (address) => {
    try {
      const page = await rpc.call('aether_accountHistory', [address, cursors?.[address] || null, 200]);
      return { address, page };
    } catch { return { address, page: { entries: [], next_cursor: null, history_start: 0 } }; }
  }));
  const chain = pages.flatMap(({ page }) => (page.entries || []).map((row) => describeHistory(row, { sources, catalog })));
  const localItems = cursors ? [] : ((await local.get(activityKey())) || []);
  return { items: mergeHistory(localItems, chain), cursors: Object.fromEntries(pages.filter(({ page }) => page.next_cursor).map(({ address, page }) => [address, page.next_cursor])),
    starts: Object.fromEntries(pages.map(({ address, page }) => [address, page.history_start])) };
}
async function track(hash, base) {
  const chainId = rpc.chainId;
  await record({ ...base, hash, state: 'pending', at: Date.now() }, chainId);
  if (rpc.chainId !== chainId) return;
  const r = await wallet.receipt(hash);
  if (rpc.chainId === chainId) await record({ ...base, hash, state: r ? (r.ok ? 'done' : 'failed') : 'unknown', height: r?.height, at: Date.now() }, chainId);
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
    const what = describeCall(a.tx);
    // A token approval moves that token: it counts as this wallet's own action
    // for the display policy (swaps go through the router, so they cannot be
    // attributed to a token without a log history).
    track(hash, { title: what, origin: a.origin, value: a.tx.value_wei, to: a.tx.to || undefined, token: what.startsWith('Token approval') ? a.tx.to : undefined });
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
  if (!originAllowed(origin)) throw err(4100, `${Brand.project} Wallet only talks to https pages (or pages served from this computer).`);
  if (method === 'eth_chainId') return `0x${rpc.chainId.toString(16)}`;
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
    if (!address) throw err(4100, `Connect this page to ${Brand.project} Wallet first (eth_requestAccounts).`);
    const raw = params[0] || {};
    if (raw.from && raw.from.toLowerCase() !== address.toLowerCase()) throw err(4100, '`from` is not the connected account.');
    let tx;
    try { tx = normalizeTx(raw); } catch (e) { throw err(-32602, e.message); }
    return askUser(origin, 'send', tx);
  }
  if (READ_METHODS.has(method)) return rpc.call(method, params);
  throw err(4200, `${Brand.project} Wallet does not support ${method}.`);
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
      await Promise.all([ready, configured]);
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
  const key = assetsKey();
  const cached = (await local.get(key)) || {};
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
        await local.set(key, out); // shown as "last read" while scanning
        const status = await rpc.call('aether_status', []);
        const sources = await tokenSources(status.chain_id);
        if (sources) {
          const catalogKey = `tokenCatalog.${status.chain_id}`;
          const catalog = (await local.get(catalogKey)) || { tokens: {}, rejected: [], factoryRead: 0, pairsRead: 0, launchesRead: 0 };
          const read = (to, data) => rpc.call('eth_call', [{ to, data }, 'latest']);
          const { catalog: cat, held } = await scanTokens({ owner, sources, catalog, read });
          await local.set(catalogKey, cat);
          await local.set(key, { address: owner, updated: Date.now(), holdings: held, error: null });
        } else {
          await local.set(key, { address: owner, updated: Date.now(), holdings: [], error: null });
        }
      } catch (e) {
        // Try again on the normal cadence, not every poll.
        await local.set(key, { address: owner, updated: Date.now(), holdings: out.holdings, error: e.message || String(e) }).catch(() => {});
      } finally {
        refreshAssets.running = null;
      }
    })();
  }
  await refreshAssets.running;
  const fresh = (await local.get(key)) || {};
  return { tokens: fresh.address === info.address ? fresh.holdings || [] : [], updated: fresh.updated || null, error: fresh.error || null };
}

// ---- the display policy (token-spam-2026.md §6) ----

/**
 * The sets the main list is decided from, all derived (never stored): tokens
 * this wallet's own transactions touched, official addresses from the bundled
 * list, and — the only part kept, on this device — the user's hide/show
 * choices. These checks read public chain data and settings on this device.
 * Nothing new is written on chain.
 */
async function displaySets(chainId) {
  const activity = (await local.get(activityKey())) || [];
  const sources = await tokenSources(chainId);
  const choices = (await local.get(`tokenChoices.${chainId}`)) || {};
  const official = sources ? [...(sources.seed || []), ...(sources.waeth ? [sources.waeth] : [])] : [];
  const key = `tokenCatalog.${chainId}`;
  const catalog = (await local.get(key)) || { tokens: {} };
  return {
    touched: activity.map((a) => a.token).filter(Boolean),
    official,
    hidden: choices.hidden || [],
    shown: choices.shown || [],
    officialSymbols: [{ symbol: Brand.coinTicker, name: Brand.coinName },
      ...official.map((a) => catalog.tokens[a.toLowerCase()]).filter(Boolean).map((t) => ({ symbol: t.symbol, name: t.name }))],
  };
}

async function chooseToken(address, { hide = false, show = false } = {}) {
  const status = await rpc.call('aether_status', []);
  const a = String(address || '').toLowerCase();
  const c = (await local.get(`tokenChoices.${status.chain_id}`)) || {};
  const hidden = new Set(c.hidden || []);
  const shown = new Set(c.shown || []);
  if (hide) { hidden.add(a); shown.delete(a); }
  if (show) { shown.add(a); hidden.delete(a); }
  const out = { hidden: [...hidden], shown: [...shown] };
  await local.set(`tokenChoices.${status.chain_id}`, out);
  return out;
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
    developerMode: Boolean(await local.get('developerMode')),
    developmentNetwork: activeNetwork?.development || false,
    developmentPort: activeNetwork?.port || (await local.get('developmentPort')) || 18546,
    chainId: rpc.chainId,
    defaultChainId: Number(defaultNetwork.chain_id),
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
  assets: async ({ force } = {}) => {
    const base = await refreshAssets({ force });
    const status = await rpc.call('aether_status', []).catch(() => null);
    if (!status) return { ...base, unverified: [], officialSymbols: [{ symbol: Brand.coinTicker, name: Brand.coinName }] };
    const sets = await displaySets(status.chain_id);
    const { main, unverified } = splitHoldings(base.tokens, sets);
    return { ...base, tokens: main, unverified, officialSymbols: sets.officialSymbols };
  },
  /** Everything the Send form checks before signing: the recipient against
   * the addresses this wallet sent to before, and the transfer itself as an
   * eth_call from this account (a honeypot reverts here). Never stored. */
  sendCheck: async ({ recipient, to, value_wei, data }) => {
    const info = await vault.info();
    const sent = ((await local.get(activityKey())) || []).map((a) => a.to).filter(Boolean);
    const risk = addressRisk(recipient, sent);
    let dry = { state: 'unchecked' };
    try {
      const result = await rpc.call('eth_call', [{ from: info.address, to, value: `0x${BigInt(value_wei || 0).toString(16)}`, data: data || '0x' }, 'latest']);
      dry = result === '0x' || /^0x0*1$/.test(result) ? { state: 'ok' } : { state: 'reverted', message: 'the token contract did not confirm the transfer' };
    } catch (e) {
      dry = { state: 'reverted', message: revertReason(e.message) };
    }
    return { risk, dry };
  },
  send: async ({ to, value_wei, data, gas, token }) => {
    if (!(await vault.unlocked())) throw new Error('Unlock first.');
    const tx = normalizeTx({ to: token ? token.address : to, value: token ? 0 : value_wei, data, gas: gas ?? (token ? 100_000 : 0) });
    if (token) {
      // Token balances are the node's answer: ask once more, so a balance that
      // moved since the popup read it cannot be spent twice.
      const info = await vault.info();
      const balance = await uintAt(await rpc.call('eth_call', [{ to: token.address, data: call(SEL.balanceOf, wordAddress(info.address)) }, 'latest']));
      if (balance < BigInt(token.amount)) throw new Error(`Not enough ${token.symbol}: the balance is ${formatTokenAmount(balance, token.decimals)}`);
    }
    const hash = await wallet.send(tx);
    if (token) {
      track(hash, { title: `Sent ${formatTokenAmount(token.amount, token.decimals)} ${token.symbol} to ${tokenShort(to)}`, origin: `${Brand.project} Wallet`, value: tx.value_wei, to, token: token.address });
    } else {
      track(hash, { title: `Send ${Brand.coinTicker}`, origin: `${Brand.project} Wallet`, value: tx.value_wei, to });
    }
    return { hash };
  },
  hideToken: ({ address }) => chooseToken(address, { hide: true }),
  showToken: ({ address }) => chooseToken(address, { show: true }),
  faucet: async () => {
    if (!(await local.get('developerMode'))) throw new Error('The faucet is available only on the local development network.');
    const info = await vault.info();
    if (!activeNetwork?.development) throw new Error('The faucet is available only on the local development network.');
    const hash = await wallet.faucet(info.address);
    track(hash, { title: `Test ${Brand.coinTicker} from the faucet`, origin: `${Brand.project} Wallet` });
    return { hash };
  },
  quote: ({ id }) => quote(id),
  activity: async () => (await activityPage()).items,
  activityPage: ({ cursors } = {}) => activityPage(cursors || null),
  linkedWallets: async () => (await local.get('linkedWallets')) || [],
  linkWallet: async ({ address }) => {
    const own = (await vault.info())?.address;
    const current = (await local.get('linkedWallets')) || [];
    if (current.length >= 8) throw new Error('You can link up to 8 view-only wallets.');
    const next = [...current, linkedAddress(address, own, current)];
    await local.set('linkedWallets', next);
    return next;
  },
  unlinkWallet: async ({ address }) => {
    const next = ((await local.get('linkedWallets')) || []).filter((a) => a.toLowerCase() !== String(address).toLowerCase());
    await local.set('linkedWallets', next);
    return next;
  },
  sites: async () => sites(),
  disconnect: ({ origin }) => setSite(origin, null),
  settings: async ({ lockMinutes, rpcs, developerMode, developmentNetwork, developmentPort }) => {
    if (developmentNetwork && developerMode === false) throw new Error('Turn on Developer mode first.');
    if (developmentNetwork) networkSettings(defaultNetwork, { developerMode: true, developmentNetwork, developmentPort });
    if (lockMinutes !== undefined) await local.set('lockMinutes', Math.min(Math.max(Number(lockMinutes) || DEFAULT_LOCK_MINUTES, 1), 24 * 60));
    if (developerMode !== undefined) await local.set('developerMode', Boolean(developerMode));
    if (developerMode === false) developmentNetwork = false;
    if (developmentNetwork !== undefined) await local.set('developmentNetwork', Boolean(developmentNetwork));
    if (developmentPort !== undefined) await local.set('developmentPort', Number(developmentPort));
    if (rpcs !== undefined) {
      const list = rpcs.filter((u) => { try { return ['http:', 'https:'].includes(new URL(u).protocol); } catch { return false; } });
      await local.set('rpcs', list);
    }
    await applySettings();
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
  Promise.all([ready, configured])
    .then(() => ui[m.op](m.args || {}))
    .then((result) => reply({ ok: true, result }), (e) => reply({ ok: false, error: e.message || String(e) }));
  return true;
});

// The lock timer: expired sessions are cleared once a minute.
chrome.alarms.create('lock', { periodInMinutes: 1 });
chrome.alarms.onAlarm.addListener((a) => { if (a.name === 'lock') vault.unlocked(); });
