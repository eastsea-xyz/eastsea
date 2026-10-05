// JSON-RPC 2.0 client for an EastSea node's HTTP endpoint
// (crates/node/src/rpc.rs — POST /, any origin, read-only here: the explorer
// signs nothing and sends nothing the mempool would take). Reads go through
// an ordered list of sources — the visitor's own node first, then the public
// read-only gateway (docs/ops/read-gateway.md). fetch and storage are
// injectable so the tests run without a node (test/rpc.test.mjs).

export const DEFAULT_ENDPOINT = 'http://127.0.0.1:18545';
/** The public read-only gateway (a follower behind a tunnel, allowlisted and
 * capped by the node itself). Its answers are honest but unverified. */
export const DEFAULT_GATEWAY = 'https://rpc.eastsea.xyz';
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
    return saved ? normalizeEndpoint(saved) : DEFAULT_GATEWAY;
  } catch {
    return DEFAULT_GATEWAY;
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
  if (source.kind === 'gateway') return 'Public gateway · not verified';
  return 'Your node';
}

/** The plain-language help shown when the visitor's own node cannot be read
 * from this page (Chrome's local-network permission was denied, or Safari
 * blocked the mixed http content). Kept here so the tests can hold it to its
 * promises: name the two browser reasons, offer the two ways out. */
export function localBlockedText() {
  return 'This page could not read your node at 127.0.0.1 — the browser blocked it (Chrome asks for local network access; Safari blocks http from an https page). '
    + 'Install the EastSea app so your own node runs, answer Allow when Chrome asks, or keep reading through the public gateway below (unverified, but never a write).';
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
    return body?.result;
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
  constructor(sources, { fetch, timeoutMs, onSource, now = () => Date.now() } = {}) {
    if (!sources?.length) throw new Error('at least one source is needed');
    this.sources = sources.map((s) => ({ kind: s.kind, url: normalizeEndpoint(s.url), failedAt: null }));
    this.pool = this.sources.map((s) => new Node(s.url, { fetch, timeoutMs }));
    this.i = 0;
    this.onSource = onSource;
    this.now = now;
  }

  /** The source in use — what the header badge and source lines should say. */
  get source() { return { kind: this.sources[this.i].kind, url: this.sources[this.i].url }; }

  /** The endpoint in use (sourceLine and friends read this). */
  get url() { return this.sources[this.i].url; }

  get kind() { return this.sources[this.i].kind; }

  label() { return sourceLabel(this.source); }

  async call(method, params = []) {
    for (let k = 0; k < this.sources.length; k++) {
      const s = this.sources[k];
      if (s.failedAt !== null && this.now() - s.failedAt < SOURCE_SKIP_MS) continue; // cooling down
      if (k !== this.i) {
        const from = this.sources[this.i].kind;
        this.i = k;
        this.onSource?.(this, { from, to: s.kind }); // the badge, and the loopback-help note
      }
      try {
        return await this.pool[k].call(method, params);
      } catch (e) {
        if (!(e instanceof RpcError) || e.code !== -1) throw e; // the source answered
        s.failedAt = this.now();
      }
    }
    throw new RpcError('no source answered (the node on this Mac and the public gateway are both unreachable)', -1);
  }

  read(to, data) {
    return this.call('eth_call', [{ to, data }, 'latest']);
  }
}
