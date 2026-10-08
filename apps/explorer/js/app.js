// Entry point: the header (search, sources, theme), the hash router and the
// polling that keeps the home page and a pending transaction current. Reads go
// through an ordered list of sources — the visitor's own node first, then the
// public read-only gateway (docs/ops/read-gateway.md) — and the header badge
// always says which one answered.

import {
  DEFAULT_ENDPOINT, DEFAULT_GATEWAY, FailoverNode,
  loadEndpoint, saveEndpoint, loadGateway, saveGateway,
  orderedSources, sourceLabel, localBlockedText,
} from './rpc.js';
import { parseTokenSources, tokenInfo, tokenOrigin } from './erc20.js';
import { resolveSearch } from './search.js';
import { accountView, blockView, errorView, homeView, notFoundView, tokenView, txView } from './pages.js';
import { detectVerifier } from './verify.js';
import { h, loading, message } from './dom.js';
import { mountLiveGlobe } from '../live-globe/live-globe.js';
import { pollCurrentPage } from './polling.js';

const view = document.getElementById('view');
const top = document.getElementById('top');
const foot = document.getElementById('foot');
const notice = document.getElementById('notice');

// ---- the context every view reads through ----

const ctx = {
  node: null,
  chainId: null, // set once the node answers aether_status
  verifier: null, // set once at boot: {kind, block, account, receipt} (verify.js)
  pollNow: false, // the current page asked to be re-checked (a pending tx)
  tokenCache: new Map(),
  originCache: new Map(),
  sourcesRaw: null, // token-sources.json as fetched

  /** The token sources of the connected chain (null before the chain id is
   * known, or when the file or the chain is missing). */
  sources() {
    return parseTokenSources(this.sourcesRaw, this.chainId);
  },

  /** `eth_call(to, data)` — the reader the ERC-20 helpers take. An arrow, so
   * it keeps the node when handed around as a bare function. */
  read: (to, data) => ctx.node.read(to, data),

  /** Cached ERC-20 metadata (null = not a readable token); three eth_calls
   * per address the first time, none after. */
  async token(address) {
    const a = String(address).toLowerCase();
    if (!this.tokenCache.has(a)) this.tokenCache.set(a, await tokenInfo(a, (to, data) => this.node.read(to, data)));
    return this.tokenCache.get(a);
  },

  /** Cached origin scan (the walk over the launchpad and DEX lists). */
  async origin(address) {
    const a = String(address).toLowerCase();
    if (!this.originCache.has(a)) this.originCache.set(a, await tokenOrigin(a, this.sources(), (to, data) => this.node.read(to, data)));
    return this.originCache.get(a);
  },
};

// ---- header ----

const searchInput = h('input', { id: 'q', type: 'search', placeholder: 'Height, 0x address or tx hash', 'aria-label': 'Search' });
const searchMsg = h('span', { id: 'search-msg', class: 'small' });
const nodeInput = h('input', { id: 'node-url', type: 'url', spellcheck: 'false', 'aria-label': 'Node JSON-RPC endpoint' });
const gatewayInput = h('input', { id: 'gateway-url', type: 'url', spellcheck: 'false', placeholder: DEFAULT_GATEWAY, 'aria-label': 'Public read gateway' });
const nodeMsg = h('span', { class: 'small' });
const chainPill = h('span', { class: 'pill', id: 'chain' }, 'connecting…');
const sourcePill = h('span', { class: 'pill plain', id: 'source', title: 'Where this page reads from; changes when a source does not answer' });
const themeButton = h('button', { class: 'ghost', title: 'Switch theme', onclick: cycleTheme }, '◐');

/** The header badge: which source answered the last read. */
function updateSourcePill(n) {
  const kind = n.kind === 'node' ? 'good' : n.kind === 'gateway' ? 'warn' : 'plain';
  sourcePill.className = `pill ${kind}`;
  sourcePill.title = n.source.url;
  sourcePill.replaceChildren(sourceLabel(n.source));
}

/** The plain-language note under the header when the visitor's own node could
 * not be read (Chrome's local-network prompt denied, Safari's mixed content).
 * Cleared when the node answers again. */
function showLocalNotice(show) {
  notice.replaceChildren(show ? message('warn', localBlockedText()) : []);
}

/** FailoverNode hands us ({from, to}) whenever the source in use changes. */
function onSourceChange(n, { from, to } = {}) {
  if (isNetworkRoute()) return;
  updateSourcePill(n);
  showLocalNotice(from === 'node' && to === 'gateway');
}

top.append(
  h('a', { class: 'brand', href: '#/' },
    h('span', { class: 'logo', 'aria-hidden': 'true' }),
    h('span', { class: 'brand-name' }, 'EastSea Explorer')),
  h('nav', { class: 'explorer-nav', 'aria-label': 'Explorer pages' },
    h('a', { href: '#/' }, 'Blocks'),
    h('a', { href: '#/network' }, 'Live network')),
  chainPill,
  sourcePill,
  h('form', {
    id: 'search',
    role: 'search',
    onsubmit: async (e) => {
      e.preventDefault();
      searchMsg.replaceChildren();
      if (!String(searchInput.value).trim()) return;
      const route = await resolveSearch(searchInput.value, ctx.node);
      if (!route) {
        searchMsg.append(message('error', `Nothing this node knows matches "${String(searchInput.value).trim().slice(0, 80)}" — try a height, a 0x… address or a tx hash.`));
        return;
      }
      searchInput.value = '';
      location.hash = `#/${route.page}/${route.page === 'block' ? route.height : (route.hash || route.address)}`;
    },
  }, searchInput, h('button', { type: 'submit' }, 'Search'), searchMsg),
  h('details', { id: 'settings' },
    h('summary', {}, 'Settings'),
    h('div', { class: 'settings-body' },
      h('label', {}, 'Node JSON-RPC endpoint', nodeInput),
      h('label', {}, 'Public read gateway (tried after the node)', gatewayInput),
      h('div', { class: 'row tight' },
        h('button', {
          onclick: () => {
            try {
              const nodeUrl = saveEndpoint(nodeInput.value, store);
              const gatewayUrl = saveGateway(gatewayInput.value, store);
              connect();
              nodeMsg.replaceChildren(message('ok', `Reading ${nodeUrl}${gatewayUrl ? ` then ${gatewayUrl}` : ' (no gateway fallback)'}.`));
            } catch (e) {
              nodeMsg.replaceChildren(message('error', e.message));
            }
          },
        }, 'Save'),
        h('button', {
          onclick: () => {
            saveEndpoint(DEFAULT_ENDPOINT, store);
            saveGateway(DEFAULT_GATEWAY, store);
            connect();
          },
        }, 'Reset'),
        nodeMsg),
      h('p', { class: 'small muted' }, 'An EastSea node serves JSON-RPC on this Mac at 127.0.0.1:18545 while it runs; when this browser cannot reach it, reads fall back to the public gateway — honest but unverified, and never a write. Empty the gateway field to read from your node only.'))),
  themeButton,
);

foot.append(
  h('p', { class: 'small muted' },
    'Reads go to your own node first, then to the public gateway (Settings). What a committee certificate vouches for is ',
    h('em', {}, 'marked on the page'), '; everything else is node-read and unverified. ',
    'No analytics, no external requests, no prices.'),
);
const defaultFoot = [...foot.childNodes];
const searchForm = document.getElementById('search');
const settings = document.getElementById('settings');

// A storage handle that is null when the browser denies access outright; every
// user of it already treats null as "keep the defaults".
const store = (() => { try { return localStorage; } catch { return null; } })();

// ---- sources, chain pill ----

function connect() {
  if (isNetworkRoute()) { render(); return; }
  const nodeUrl = loadEndpoint(store);
  const gatewayUrl = loadGateway(store);
  nodeInput.value = nodeUrl;
  gatewayInput.value = gatewayUrl || '';
  ctx.node = new FailoverNode(orderedSources(nodeUrl, gatewayUrl), { onSource: onSourceChange });
  ctx.chainId = null;
  ctx.tokenCache.clear();
  ctx.originCache.clear();
  updateSourcePill(ctx.node);
  showLocalNotice(false);
  chainPill.replaceChildren('connecting…');
  chainPill.classList.remove('good');
  ctx.node.call('aether_status')
    .then((s) => {
      ctx.chainId = s.chain_id;
      const net = ctx.sources()?.network;
      chainPill.replaceChildren(`${net || 'chain'} ${s.chain_id}`);
      chainPill.classList.add('good');
    })
    .catch(() => chainPill.replaceChildren('no node'));
  render();
}

// ---- routing ----

const routes = [
  [/^#?\/?$/, () => homeView(ctx)],
  [/^#\/block\/(\d+)$/, (m) => blockView(ctx, Number(m[1]))],
  [/^#\/tx\/((?:0x)?[0-9a-fA-F]{64})$/, (m) => txView(ctx, m[1].toLowerCase().replace(/^0x/, '').replace(/^/, '0x'))],
  [/^#\/account\/(0x[0-9a-fA-F]{40})$/, (m) => accountView(ctx, m[1].toLowerCase())],
  [/^#\/token\/(0x[0-9a-fA-F]{40})$/, (m) => tokenView(ctx, m[1].toLowerCase())],
];

// A slow page never overwrites a newer one: only the newest render may paint.
let renderSeq = 0;
let liveGlobe = null;
let nodeViewReady = false;
let nodeViewBoot = null;

function isNetworkRoute() { return /^#\/network\/?$/.test(location.hash); }

// The globe has a deliberately separate read boundary: no loopback, committee
// pins, peer identifiers or block details are requested on this route.
function renderNetwork() {
  document.documentElement.dataset.page = 'network';
  settings.remove();
  searchForm.remove();
  chainPill.remove();
  nodeInput.value = '';
  gatewayInput.value = '';
  sourcePill.className = 'pill plain';
  sourcePill.textContent = 'Presence · node view, unverified';
  sourcePill.title = 'Aggregated counts from the configured public read gateway';
  notice.replaceChildren();
  foot.replaceChildren(h('p', { class: 'small muted' },
    'Counts are one node’s view, not a network census. No analytics or location services.'));
  const globeRoot = h('div', {});
  view.replaceChildren(h('section', { class: 'network-page', 'aria-labelledby': 'network-title' },
    h('h1', { id: 'network-title' }, 'Live network'),
    h('p', { class: 'network-intro' }, 'Macs keeping EastSea connected, seen a continent at a time.'),
    globeRoot));
  const fixture = new URLSearchParams(location.search).get('globe') === 'fixture';
  liveGlobe = mountLiveGlobe(globeRoot, {
    endpoint: loadGateway(store), fixture,
    ...(fixture ? { seed: 'fixture-smoke' } : {}),
  });
}

function restoreExplorer() {
  delete document.documentElement.dataset.page;
  if (!settings.isConnected) top.insertBefore(settings, themeButton);
  if (!searchForm.isConnected) top.insertBefore(searchForm, settings);
  if (!chainPill.isConnected) top.insertBefore(chainPill, sourcePill);
  nodeInput.value = loadEndpoint(store);
  gatewayInput.value = loadGateway(store) || '';
  foot.replaceChildren(...defaultFoot);
  if (ctx.node) updateSourcePill(ctx.node);
}

async function render() {
  ctx.pollNow = false;
  const mine = ++renderSeq;
  liveGlobe?.destroy();
  liveGlobe = null;
  if (isNetworkRoute()) { renderNetwork(); return; }
  restoreExplorer();
  if (!nodeViewReady) { void bootNodeView(); return; }
  if (!ctx.node) { connect(); return; }
  const hash = location.hash || '#/';
  view.replaceChildren(loading());
  let out;
  try {
    const hit = routes.find(([re]) => re.test(hash));
    out = hit ? await hit[1](hash.match(hit[0])) : notFoundView(ctx, `No page for ${hash.slice(0, 60)}.`);
  } catch (e) {
    out = errorView(ctx, e);
  }
  if (mine === renderSeq) view.replaceChildren(out);
}

window.addEventListener('hashchange', render);

// Keep the home page and a pending transaction current while someone watches;
// a hidden tab or any other page (open disclosure blocks included) is left alone.
pollCurrentPage(render, {
  getState: () => ({ hidden: document.hidden, hash: location.hash, pending: ctx.pollNow }),
});

// ---- theme ----

function themePref() {
  try {
    return localStorage.getItem('aether-explorer.theme') || '';
  } catch {
    return ''; // storage denied (some private modes): follow the system
  }
}

function applyTheme() {
  const pref = themePref();
  if (pref) document.documentElement.dataset.theme = pref;
  else delete document.documentElement.dataset.theme;
  themeButton.textContent = { dark: '☾', light: '☀' }[pref] || '◐';
  themeButton.title = `Theme: ${pref || 'auto'}`;
}

function cycleTheme() {
  const order = ['', 'dark', 'light'];
  const next = order[(order.indexOf(themePref()) + 1) % order.length];
  try {
    localStorage.setItem('aether-explorer.theme', next);
  } catch { /* keep the system theme when storage is denied */ }
  applyTheme();
}

// ---- boot ----

applyTheme();
function bootNodeView() {
  if (nodeViewBoot) return nodeViewBoot;
  view.replaceChildren(loading());
  nodeViewBoot = (async () => {
    try {
      ctx.sourcesRaw = await (await fetch('token-sources.json')).json();
    } catch { /* token origins remain unavailable */ }
    ctx.verifier = await detectVerifier(window);
    nodeViewReady = true;
    if (!isNetworkRoute()) connect();
  })();
  return nodeViewBoot;
}
if (isNetworkRoute()) render();
else void bootNodeView();
