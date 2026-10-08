// A live walk through every page, against a real node (the local one by
// default), rendering into a minimal DOM stub — not part of `npm test`, which
// stays offline. Run: `node test/live.mjs [endpoint]`

import { Node } from '../js/rpc.js';
import { parseTokenSources, tokenInfo, tokenOrigin } from '../js/erc20.js';
import { accountView, blockView, homeView, tokenView, txView } from '../js/pages.js';
import { resolveSearch } from '../js/search.js';
import { readFileSync } from 'node:fs';

// ---- the smallest DOM that dom.js and pages.js need ----

class El {
  constructor(tag) { this.tagName = tag; this.children = []; this.attrs = {}; this.listeners = {}; }
  setAttribute(k, v) { this.attrs[k] = v; }
  get className() { return this.attrs.class || ''; }
  set className(v) { this.attrs.class = v; }
  addEventListener(k, fn) { (this.listeners[k] ||= []).push(fn); }
  append(...cs) { this.children.push(...cs.flat()); return this; }
  replaceChildren(...cs) { this.children = [...cs.flat()]; return this; }
}
globalThis.Node = El;
globalThis.document = { createElement: (t) => new El(t) };

/** Every string a tree renders, in order — what a page "says". */
function text(el) {
  if (el == null) return '';
  if (typeof el === 'string') return el;
  return el.children.map(text).join(' ');
}
function says(el, needle) {
  return text(el).includes(needle);
}

// ---- the context, as app.js builds it ----

const node = new Node(process.argv[2] || 'http://127.0.0.1:18545');
const sourcesRaw = JSON.parse(readFileSync(new URL('../token-sources.json', import.meta.url)));
const ctx = {
  node,
  chainId: null,
  pollNow: false,
  tokenCache: new Map(),
  originCache: new Map(),
  sourcesRaw,
  sources() { return parseTokenSources(this.sourcesRaw, this.chainId); },
  read: (to, data) => ctx.node.read(to, data),
  async token(address) {
    const a = String(address).toLowerCase();
    if (!this.tokenCache.has(a)) this.tokenCache.set(a, await tokenInfo(a, (to, data) => this.node.read(to, data)));
    return this.tokenCache.get(a);
  },
  async origin(address) {
    const a = String(address).toLowerCase();
    if (!this.originCache.has(a)) this.originCache.set(a, await tokenOrigin(a, this.sources(), (to, data) => this.node.read(to, data)));
    return this.originCache.get(a);
  },
};

const checks = [];
const check = (name, ok, note = '') => {
  checks.push(ok);
  console.log(`${ok ? 'ok' : 'FAIL'}  ${name}${note ? ` — ${note}` : ''}`);
};

const status = await node.call('aether_status');
ctx.chainId = status.chain_id;
const blocks = await node.call('aether_recentBlocks', [5]);
const newest = blocks[0];

// home
const home = await homeView(ctx);
check('home', says(home, 'Finalized height') && says(home, 'Latest blocks') && says(home, 'Committee'), `height ${status.height}`);

// a recent block, the genesis height (this node keeps no summary for it), and one not built yet
for (const [label, h, expect] of [
  ['newest block', newest.height, 'Header'],
  ['genesis height', 0, 'knows no block'], // summaries start above genesis on this node
  ['unbuilt block', status.height + 5, 'Not built yet'],
]) {
  const page = await blockView(ctx, h);
  check(label, expect == null || says(page, expect), `height ${h}`);
}

// the newest block's first transaction
if (newest.txs.length) {
  const tx = await txView(ctx, newest.txs[0]);
  check('tx page (receipt)', says(tx, 'Receipt'), newest.txs[0].slice(0, 18) + '…');
  const r = await resolveSearch(newest.txs[0], node);
  check('search finds the tx', r?.page === 'tx');
} else {
  console.log('     (no transactions in the newest block; skipping the tx page)');
}

// an account: the newest proposer
const account = await accountView(ctx, newest.proposer);
check('account page', says(account, 'Balance') && says(account, 'Nonce'), newest.proposer);

// a transaction with an ERC-20 Transfer in it, if the window has one: exercises
// the decode path (amount + symbol + both addresses)
try {
  const head = parseInt(String(await node.call('eth_blockNumber')).replace(/^0x/, ''), 16);
  const logs = await node.call('eth_getLogs', [{
    fromBlock: `0x${Math.max(0, head - 2000).toString(16)}`, toBlock: 'latest',
    topics: ['0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef'],
  }]);
  const last = logs?.[logs.length - 1];
  if (last) {
    const tx = await txView(ctx, last.transactionHash);
    check('tx page (ERC-20 transfer)', says(tx, 'Transfer') && says(tx, 'Events'), last.transactionHash.slice(0, 18) + '…');
  } else {
    console.log('     (no ERC-20 transfer in the log window; skipping the decode check)');
  }
} catch (e) {
  console.log('     (log scan failed, skipping the decode check:', e.message, ')');
}

// a token from the bundled sources
const sources = ctx.sources();
const token = sources?.seed?.[0] || sources?.waeth;
if (token) {
  const page = await tokenView(ctx, token);
  check('token page', says(page, 'Metadata') && says(page, 'Total supply'), token);
  const found = await resolveSearch(token, node);
  check('search finds the address', found?.page === 'account');
  const byHeight = await resolveSearch(String(newest.height), node);
  check('search finds the height', byHeight?.page === 'block');
  check('text search routes to apps and names', (await resolveSearch('not a thing', node))?.page === 'search');
} else {
  console.log('     (no token sources for this chain; skipping the token page)');
}

const failed = checks.filter((c) => !c).length;
console.log(failed ? `\n${failed} check(s) failed` : '\nall live checks passed');
process.exit(failed ? 1 : 0);
