// Entry point: the header (search, sources, theme), the hash router and the
// polling that keeps the home page and a pending transaction current. Reads go
// from the visitor's node first, then verified public peers. The header badge
// always says which source answered; a personal HTTP gateway is optional.

import {
  DEFAULT_ENDPOINT, DEFAULT_GATEWAY, FailoverNode,
  loadEndpoint, saveEndpoint, loadGateway, saveGateway,
  orderedSources, sourceLabel, localBlockedText,
} from './rpc.js';
import { parseTokenSources, tokenInfo, tokenOrigin } from './erc20.js';
import { resolveSearch, searchRoute, decodeSearchQuery } from './search.js';
import { appSearchView } from './app-search.js';
import { resolveSearchLocale, searchText } from './search-catalog.js';
import { accountView, blockView, errorView, homeView, notFoundView, seaLinkView, tokenView, txView } from './pages.js';
import { parseSeaURL, externalNameMessage, suggestedHTTPS } from './sea-url.mjs';
import { detectVerifier } from './verify.js';
import { loadPublicPeerPool } from './peers.js';
import { h, loading, message } from './dom.js';
import { pollCurrentPage } from './polling.js';
import { mountLiveGlobe } from '../live-globe/live-globe.js';

const view = document.getElementById('view');
const top = document.getElementById('top');
const foot = document.getElementById('foot');
const notice = document.getElementById('notice');

// The explorer uses hashes for routes; focusing the main content must not
// change the route to #view.
document.querySelector('.skip')?.addEventListener('click', (event) => {
  event.preventDefault();
  view.focus();
  view.scrollIntoView({ block: 'start' });
});

// ---- the context every view reads through ----

const ctx = {
  node: null,
  locale: resolveSearchLocale(navigator.languages || navigator.language),
  chainId: null, // set once the node answers aether_status
  verifier: null, // set once at boot: {kind, block, account, receipt} (verify.js)
  peerPool: null,
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
const peerEvents = [];

// ---- header ----

const searchInput = h('input', { id: 'q', class: 'es-control', type: 'search', placeholder: searchText(ctx.locale, 'placeholder'), 'aria-label': searchText(ctx.locale, 'search') });
const searchMsg = h('span', { id: 'search-msg', class: 'small' });
const nodeInput = h('input', { id: 'node-url', class: 'es-control', type: 'url', spellcheck: 'false', 'aria-label': 'Node JSON-RPC endpoint' });
const gatewayInput = h('input', { id: 'gateway-url', class: 'es-control', type: 'url', spellcheck: 'false', placeholder: 'https://your-gateway.example (optional)', 'aria-label': 'Your optional read gateway' });
const relayInput = h('input', { id: 'relay-urls', class: 'es-control', type: 'text', spellcheck: 'false', placeholder: 'n0 public relays (default)', 'aria-label': 'WebSocket relay URLs, comma separated' });
const nodeMsg = h('span', { class: 'small' });
const chainPill = h('span', { class: 'pill es-status', id: 'chain' }, 'connecting…');
const sourcePill = h('span', { class: 'pill es-status plain', id: 'source', title: 'Where this page reads from; changes when a source does not answer' });
const themeButton = h('button', { class: 'ghost es-control', type: 'button', title: 'Switch theme', 'aria-label': 'Switch theme', onclick: cycleTheme });

/** The header badge: which source answered the last read. */
function updateSourcePill(n) {
  const kind = ['node', 'peers'].includes(n.kind) ? 'good' : n.kind === 'gateway' ? 'warn' : 'plain';
  sourcePill.className = `pill es-status ${kind}`;
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
  showLocalNotice(to === 'peers' || (from === 'node' && to === 'gateway'));
}

top.append(
  h('div', { class: 'header-inner' },
  h('div', { class: 'header-identity' },
    h('a', { class: 'brand', href: '#/', 'aria-label': 'EastSea Explorer home' },
      h('img', { class: 'logo', src: 'assets/dawn.svg', width: 32, height: 32, alt: '' }),
      h('span', { class: 'brand-name' }, h('span', { class: 'es-wordmark' }, 'EastSea'),
        h('span', { class: 'brand-surface' }, 'Explorer'))),
    h('nav', { class: 'explorer-nav', 'aria-label': 'Explorer pages' },
      h('a', { href: '#/' }, 'Blocks'),
      h('a', { href: '#/network' }, 'Live network'))),
  h('div', { class: 'header-source' }, chainPill, sourcePill),
  h('form', {
    id: 'search',
    role: 'search',
    lang: ctx.locale,
    onsubmit: async (e) => {
      e.preventDefault();
      searchMsg.replaceChildren();
      if (!String(searchInput.value).trim()) return;
      let route;
      try {
        route = await resolveSearch(searchInput.value, ctx.node, ctx.chainId ?? 1);
      } catch (error) {
        const text = error.code === 'externalTLD' ? externalNameMessage(navigator.language) : error.message;
        searchMsg.append(message('error', text));
        const https = suggestedHTTPS(searchInput.value);
        if (https) searchMsg.append(h('a', { href: https, target: '_blank', rel: 'noopener noreferrer' }, 'Open with https://'));
        return;
      }
      if (!route) {
        searchMsg.append(message('error', searchText(ctx.locale, 'notFound', { query: String(searchInput.value).trim().slice(0, 80) })));
        return;
      }
      searchInput.value = '';
      location.hash = searchRoute(route);
    },
  }, searchInput, h('button', { type: 'submit', class: 'es-control' }, searchText(ctx.locale, 'search')), searchMsg),
  h('details', { id: 'settings' },
    h('summary', { class: 'es-control' }, 'Settings'),
    h('div', { class: 'settings-body' },
      h('label', {}, 'Node JSON-RPC endpoint', nodeInput),
      h('label', {}, 'Your read gateway (optional, after public peers)', gatewayInput),
      h('label', {}, 'WebSocket relays (comma separated; empty uses n0)', relayInput),
      h('div', { class: 'row tight' },
        h('button', {
          class: 'es-control primary',
          onclick: async () => {
            try {
              const nodeUrl = saveEndpoint(nodeInput.value, store);
              const gatewayUrl = saveGateway(gatewayInput.value, store);
              const relays = String(relayInput.value).split(',').map((s) => s.trim()).filter(Boolean);
              for (const relay of relays) {
                const u = new URL(relay);
                if (!['https:', 'http:'].includes(u.protocol)) throw new Error('relays must use http:// or https://');
              }
              store?.setItem('aether-explorer.relays', JSON.stringify(relays));
              await setupPeers(relays);
              connect();
              nodeMsg.replaceChildren(message('ok', `Reading ${nodeUrl}, then verified peers${gatewayUrl ? `, then ${gatewayUrl}` : ''}.`));
            } catch (e) {
              nodeMsg.replaceChildren(message('error', e.message));
            }
          },
        }, 'Save'),
        h('button', {
          class: 'es-control',
          onclick: async () => {
            saveEndpoint(DEFAULT_ENDPOINT, store);
            saveGateway(DEFAULT_GATEWAY, store);
            store?.removeItem('aether-explorer.relays');
            await setupPeers([]);
            connect();
          },
        }, 'Reset'),
        nodeMsg),
      h('p', { class: 'small muted' }, 'Your node at 127.0.0.1:18545 is tried first. Public peers serve certificate-verified blocks and state proofs over n0 relays. A personal gateway is optional. Token calls and node metrics need your own node.'))),
  themeButton,
));

foot.append(
  h('p', { class: 'small muted' },
    'Reads go to your own node first, then to verified public peers over WebSocket relays. ',
    'Committee certificates and Merkle proofs verify public chain data. Uncommitted metrics are unavailable on public peers. ',
    'No analytics, no prices.'),
);
const defaultFoot = [...foot.childNodes];
const searchForm = document.getElementById('search');
const settings = document.getElementById('settings');

// A storage handle that is null when the browser denies access outright; every
// user of it already treats null as "keep the defaults".
const store = (() => { try { return localStorage; } catch { return null; } })();

// ---- sources, chain pill ----

async function setupPeers(relays = []) {
  ctx.peerPool?.close();
  ctx.peerPool = null;
  relayInput.value = relays.join(', ');
  try {
    ctx.peerPool = await loadPublicPeerPool({ env: ctx.verifier?.env, relays, onPeer: event => {
      if (event.dropped) {
        peerEvents.push({ ...event, at: performance.now() });
        if (peerEvents.length > 64) peerEvents.shift();
      }
    } });
  } catch (e) {
    nodeMsg.replaceChildren(message('warn', `Public peer reads are unavailable: ${e?.message || e}`));
  }
}

function connect() {
  if (isNetworkRoute()) { render(); return; }
  const nodeUrl = loadEndpoint(store);
  const gatewayUrl = loadGateway(store);
  nodeInput.value = nodeUrl;
  gatewayInput.value = gatewayUrl || '';
  ctx.node = new FailoverNode(orderedSources(nodeUrl, gatewayUrl), { onSource: onSourceChange, peerPool: ctx.peerPool });
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
  [/^#\/search\/(.*)$/, (m) => appSearchView(ctx, decodeSearchQuery(m[1]))],
  [/^#\/name\/(.+)$/, async (m) => {
    const link = parseSeaURL(decodeURIComponent(m[1]), ctx.chainId ?? 1);
    if (link.kind === 'action') return seaLinkView(link);
    return h('div', { class: 'stack' }, seaLinkView(link), await appSearchView(ctx, link.registryName));
  }],
];

// A slow page never overwrites a newer one: only the newest render may paint.
let renderSeq = 0;
let activeRenders = 0;
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
  sourcePill.title = 'Aggregated counts from the configured public presence source';
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
    // This is the globe's separate unverified presence feed, never a chain read.
    endpoint: loadGateway(store) || 'https://rpc.eastsea.xyz', fixture,
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
  activeRenders++;
  try {
    const hash = location.hash || '#/';
    view.replaceChildren(loading(hash.startsWith('#/search/') ? searchText(ctx.locale, 'loading') : undefined));
    let out;
    try {
      const hit = routes.find(([re]) => re.test(hash));
      out = hit ? await hit[1](hash.match(hit[0])) : notFoundView(ctx, `No page for ${hash.slice(0, 60)}.`);
    } catch (e) {
      out = errorView(ctx, e);
    }
    if (mine === renderSeq) view.replaceChildren(out);
  } finally { activeRenders--; }
}

window.addEventListener('hashchange', render);

// Keep the home page and a pending transaction current while someone watches;
// a hidden tab or any other page (open disclosure blocks included) is left alone.
pollCurrentPage(render, {
  getState: () => ({ hidden: document.hidden || activeRenders > 0, hash: location.hash, pending: ctx.pollNow }),
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
  themeButton.replaceChildren(themeIcon(pref));
  themeButton.title = `Theme: ${pref || 'auto'}`;
  themeButton.setAttribute('aria-label', `Theme: ${pref || 'auto'}. Switch theme`);
}

/** The same 18 px stroke language in all three theme states. */
function themeIcon(pref) {
  const ns = 'http://www.w3.org/2000/svg';
  const svg = document.createElementNS(ns, 'svg');
  svg.setAttribute('viewBox', '0 0 24 24');
  svg.setAttribute('aria-hidden', 'true');
  const part = (tag, attrs) => {
    const el = document.createElementNS(ns, tag);
    for (const [key, value] of Object.entries(attrs)) el.setAttribute(key, value);
    svg.append(el);
  };
  if (pref === 'dark') {
    part('path', { d: 'M20 14.4A8.5 8.5 0 0 1 9.6 4 8.5 8.5 0 1 0 20 14.4Z' });
  } else if (pref === 'light') {
    part('circle', { cx: 12, cy: 12, r: 4 });
    part('path', { d: 'M12 2v2m0 16v2M2 12h2m16 0h2M5 5l1.5 1.5m11 11L19 19M5 19l1.5-1.5m11-11L19 5' });
  } else {
    part('circle', { cx: 12, cy: 12, r: 8 });
    part('path', { d: 'M12 4a8 8 0 0 1 0 16Z', fill: 'currentColor', stroke: 'none' });
  }
  return svg;
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
    let savedRelays = [];
    try {
      const value = JSON.parse(store?.getItem('aether-explorer.relays') || '[]');
      if (Array.isArray(value) && value.every((url) => typeof url === 'string')) savedRelays = value;
    } catch { /* use public relays */ }
    await setupPeers(savedRelays);
    nodeViewReady = true;
    if (!isNetworkRoute()) connect();
  })();
  return nodeViewBoot;
}
if (isNetworkRoute()) render();
else void bootNodeView();
window.addEventListener('pagehide', () => ctx.peerPool?.close());
// Bounded diagnostics for the devnet browser measurement; no telemetry.
window.aetherReadDiagnostics = () => ({ source: ctx.node?.source, livePeers: ctx.peerPool?.livePeers || [],
  peerEvents,
  droppedPeers: [...(ctx.peerPool?.dropped || [])].map(([node, detail]) => ({ node, ...detail })),
  firstVerifiedHeadAt: ctx.peerPool?.monotonicFirstVerifiedHeadAt ?? null, metrics: ctx.peerPool?.metrics || null });
