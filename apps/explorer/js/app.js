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

const searchInput = h('input', { id: 'q', class: 'es-control', type: 'search', placeholder: 'Height, address or transaction hash', 'aria-label': 'Search blocks, addresses and transactions' });
const searchMsg = h('span', { id: 'search-msg', class: 'small' });
const nodeInput = h('input', { id: 'node-url', class: 'es-control', type: 'url', spellcheck: 'false', 'aria-label': 'Node JSON-RPC endpoint' });
const gatewayInput = h('input', { id: 'gateway-url', class: 'es-control', type: 'url', spellcheck: 'false', placeholder: DEFAULT_GATEWAY, 'aria-label': 'Public read gateway' });
const nodeMsg = h('span', { class: 'small' });
const chainPill = h('span', { class: 'pill es-status', id: 'chain' }, 'connecting…');
const sourcePill = h('span', { class: 'pill es-status plain', id: 'source', title: 'Where this page reads from; changes when a source does not answer' });
const themeButton = h('button', { class: 'ghost es-control', type: 'button', title: 'Switch theme', 'aria-label': 'Switch theme', onclick: cycleTheme });

/** The header badge: which source answered the last read. */
function updateSourcePill(n) {
  const kind = n.kind === 'node' ? 'good' : n.kind === 'gateway' ? 'warn' : 'plain';
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
  updateSourcePill(n);
  showLocalNotice(from === 'node' && to === 'gateway');
}

top.append(
  h('div', { class: 'header-inner' },
  h('div', { class: 'header-identity' },
    h('a', { class: 'brand', href: '#/', 'aria-label': 'EastSea Explorer home' },
      h('img', { class: 'logo', src: 'assets/dawn.svg', width: 32, height: 32, alt: '' }),
      h('span', { class: 'brand-name' }, h('span', { class: 'es-wordmark' }, 'EastSea'),
        h('span', { class: 'brand-surface' }, 'Explorer')))),
  h('div', { class: 'header-source' }, chainPill, sourcePill),
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
  }, searchInput, h('button', { type: 'submit', class: 'es-control' }, 'Search'), searchMsg),
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
));

foot.append(
  h('p', { class: 'small muted' },
    'Reads go to your own node first, then to the public gateway (Settings). What a committee certificate vouches for is ',
    h('em', {}, 'marked on the page'), '; everything else is node-read and unverified. ',
    'No analytics, no external requests, no prices.'),
);

// A storage handle that is null when the browser denies access outright; every
// user of it already treats null as "keep the defaults".
const store = (() => { try { return localStorage; } catch { return null; } })();

// ---- sources, chain pill ----

function connect() {
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

async function render() {
  ctx.pollNow = false;
  const mine = ++renderSeq;
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
setInterval(() => {
  if (document.hidden) return;
  const h0 = location.hash || '#/';
  if (h0 === '#' || h0 === '#/' || ctx.pollNow) render();
}, 12_000);

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
// Sources first, so the first chain pill and origin scan already have them.
try {
  ctx.sourcesRaw = await (await fetch('token-sources.json')).json();
} catch { /* no sources: origin falls back to "not in any list" */ }
// The verifier before the first render: the app's native bridge inside the
// Explore tab, else the wasm module when this deployment carries it, else
// none — pages then badge what was actually verified.
ctx.verifier = await detectVerifier(window);
connect();
