// JSON-RPC 2.0 client for an EastSea node's HTTP endpoint
// (crates/node/src/rpc.rs — POST /, any origin, read-only here: the explorer
// signs nothing and sends nothing the mempool would take). Reads try the
// visitor's own node, verified public peers, then an optional personal gateway.
// fetch and storage are
// injectable so the tests run without a node (test/rpc.test.mjs).

import { markHttpRead } from './peers.js';

export const DEFAULT_ENDPOINT = 'http://127.0.0.1:18545';
/** A gateway exists only when the visitor enters their own in Settings. */
export const DEFAULT_GATEWAY = '';
const STORAGE_KEY = 'aether-explorer.node';
const GATEWAY_KEY = 'aether-explorer.gateway';

/** How long a source that failed to answer is skipped before it is tried
 * again — a visitor whose browser blocks the loopback must not wait out the
 * node's timeout on every single call, but a node that comes up later is
 * picked back up within a minute. */
const SOURCE_SKIP_MS = 60_000;

/** Throws unless `s` is an http(s) URL a browser can POST to. */
export function normalizeEndpoint(s) {
  const url = String(s || '').trim().replace(/\/+$/, '');
  let parsed;
  try {
    parsed = new URL(url);
  } catch {
    throw new Error('the endpoint is not a URL (http://host:port)');
  }
  if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') throw new Error('the endpoint must be http:// or https://');
  return url;
}

/** The saved node endpoint, or the default (a node on this Mac). */
export function loadEndpoint(storage) {
  try {
    const saved = storage?.getItem(STORAGE_KEY);
    return saved ? normalizeEndpoint(saved) : DEFAULT_ENDPOINT;
  } catch {
    return DEFAULT_ENDPOINT;
  }
}

export function saveEndpoint(url, storage) {
  const normalized = normalizeEndpoint(url);
  storage?.setItem(STORAGE_KEY, normalized);
  return normalized;
}

/** The saved gateway, the default one, or null when the visitor turned the
 * fallback off (an empty saved value). */
export function loadGateway(storage) {
  try {
    const saved = storage?.getItem(GATEWAY_KEY);
    if (saved === '') return null; // explicitly off
    return saved ? normalizeEndpoint(saved) : null;
  } catch {
    return null;
  }
}

export function saveGateway(url, storage) {
  const trimmed = String(url || '').trim();
  const normalized = trimmed === '' ? '' : normalizeEndpoint(trimmed);
  storage?.setItem(GATEWAY_KEY, normalized);
  return normalized;
}

/** The sources a page reads through, in order: the visitor's own node first,
 * the public gateway after it. The gateway is left out only when it is null. */
export function orderedSources(nodeUrl, gatewayUrl) {
  const node = normalizeEndpoint(nodeUrl);
  const sources = [{ kind: node === DEFAULT_ENDPOINT ? 'node' : 'custom', url: node }];
  if (gatewayUrl) sources.push({ kind: 'gateway', url: normalizeEndpoint(gatewayUrl) });
  return sources;
}

/** The header badge text for a source: honest about what verified means. */
export function sourceLabel(source) {
  if (source.kind === 'node') return "Your Mac's node";
  if (source.kind === 'peers') return `Public peers · verified${source.peers ? ` · ${source.peers} connected` : ''}`;
  if (source.kind === 'gateway') return 'Your gateway · not verified';
  return 'Your node';
}

/** The plain-language help shown when the visitor's own node cannot be read
 * from this page (Chrome's local-network permission was denied, or Safari
 * blocked the mixed http content). Kept here so the tests can hold it to its
 * promises: name the two browser reasons, offer the two ways out. */
export function localBlockedText() {
  return 'This page could not read your node at 127.0.0.1 — the browser blocked it (Chrome asks for local network access; Safari blocks http from an https page). '
    + 'Install the EastSea app so your own node runs, answer Allow when Chrome asks, or read from verified public peers. Settings also accepts your own optional gateway.';
}

export class RpcError extends Error {
  constructor(message, code, status) {
    super(message);
    this.name = 'RpcError';
    this.code = code;
    this.status = status;
  }
}

let nextId = 1;

export class Node {
  /**
   * @param {string} url the node's HTTP endpoint
   * @param {{fetch?: Function, timeoutMs?: number}} opts `fetch` defaults to
   *   the global one, bound so injected stubs can replace it in tests.
   */
  constructor(url, { fetch = (...a) => globalThis.fetch(...a), timeoutMs = 8000 } = {}) {
    this.url = normalizeEndpoint(url);
    this.fetchFn = fetch;
    this.timeoutMs = timeoutMs;
  }

  /** One JSON-RPC call -> its `result`. Throws `RpcError` on a transport
   * failure, a non-2xx answer or an error object from the node. */
  async call(method, params = []) {
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), this.timeoutMs);
    let res;
    try {
      res = await this.fetchFn(this.url, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ jsonrpc: '2.0', id: nextId++, method, params }),
        signal: controller.signal,
      });
    } catch (e) {
      const why = e?.name === 'AbortError' ? `the node did not answer in ${this.timeoutMs} ms` : `could not reach the node (${e?.message || e})`;
      throw new RpcError(why, -1);
    } finally {
      clearTimeout(timer);
    }
    if (!res.ok) throw new RpcError(`the node answered HTTP ${res.status}`, -1, res.status);
    let body;
    try {
      body = await res.json();
    } catch {
      throw new RpcError('the node did not answer JSON (is this an EastSea node?)', -1);
    }
    if (body?.error) throw new RpcError(body.error.message, body.error.code);
    return markHttpRead(body?.result, { kind: this.url === DEFAULT_ENDPOINT ? 'node' : 'custom', url: this.url });
  }

  /** `eth_call(to, data) -> hex`, the reader the ERC-20 helpers take. */
  read(to, data) {
    return this.call('eth_call', [{ to, data }, 'latest']);
  }
}

/**
 * Reads through an ordered list of sources: the visitor's own node first, the
 * public gateway after it. Only a transport failure moves on to the next
 * source — a source that answers owns its answer, error included: a gateway
 * refusal to serve a write is shown, not routed around. A source that failed
 * is skipped for SOURCE_SKIP_MS so a blocked loopback costs one timeout, not
 * one per call; the order is re-evaluated every call, so a node that comes
 * back is preferred again.
 */
export class FailoverNode {
  /**
   * @param {Array<{kind: string, url: string}>} sources in preference order
   * @param {{fetch?: Function, timeoutMs?: number, onSource?: Function, now?: Function}} opts
   *   `onSource(node)` fires whenever the source in use changes; `now` is
   *   injectable so tests need no clock.
   */
  constructor(sources, { fetch, timeoutMs, onSource, peerPool = null, now = () => Date.now() } = {}) {
    if (!sources?.length) throw new Error('at least one source is needed');
    this.sources = sources.map((s) => ({ kind: s.kind, url: normalizeEndpoint(s.url), failedAt: null }));
    this.pool = this.sources.map((s) => new Node(s.url, { fetch, timeoutMs }));
    this.i = 0;
    this.onSource = onSource;
    this.now = now;
    this.peerPool = peerPool;
    this.usingPeers = false;
  }

  /** The source in use — what the header badge and source lines should say. */
  get source() { return this.usingPeers ? this.peerPool.source : { kind: this.sources[this.i].kind, url: this.sources[this.i].url }; }

  /** The endpoint in use (sourceLine and friends read this). */
  get url() { return this.source.url; }

  get kind() { return this.source.kind; }

  label() { return sourceLabel(this.source); }

  verdict(kind, key) { return this.usingPeers ? this.peerPool.verdict(kind, key) : null; }

  selectHttp(k) {
    const from = this.kind;
    const changed = this.usingPeers || k !== this.i;
    this.usingPeers = false;
    this.i = k;
    if (changed) this.onSource?.(this, { from, to: this.kind });
  }

  async call(method, params = []) {
    let peerError = null;
    const tryPeers = async () => {
      if (!this.peerPool) return { answered: false };
      try {
        const result = await this.peerPool.call(method, params);
        const from = this.kind;
        this.usingPeers = true;
        this.onSource?.(this, { from, to: 'peers' });
        return { answered: true, result };
      } catch (e) { peerError = e; return { answered: false }; }
    };
    let triedPeers = false;
    for (let k = 0; k < this.sources.length; k++) {
      const s = this.sources[k];
      if (s.kind === 'gateway' && !triedPeers) {
        triedPeers = true;
        const answer = await tryPeers();
        if (answer.answered) return answer.result;
      }
      if (s.failedAt !== null && this.now() - s.failedAt < SOURCE_SKIP_MS) continue; // cooling down
      try {
        const result = await this.pool[k].call(method, params);
        this.selectHttp(k);
        return markHttpRead(result, this.source);
      } catch (e) {
        if (!(e instanceof RpcError) || e.code !== -1) throw e; // the source answered
        s.failedAt = this.now();
      }
    }
    if (!triedPeers) {
      const answer = await tryPeers();
      if (answer.answered) return answer.result;
    }
    throw new RpcError(`no source answered (your node, verified public peers and any configured gateway)${peerError ? `: ${peerError.message || peerError}` : ''}`, -1);
  }

  read(to, data) {
    return this.call('eth_call', [{ to, data }, 'latest']);
  }
}
