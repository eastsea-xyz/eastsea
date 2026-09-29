import { Brand } from './brand.js';
// JSON-RPC to Aether nodes. The first endpoint that answers on the right chain
// is used; one that does not answer sleeps 5 s, doubling to 60 s.

import { CHAIN_ID } from './methods.js';

/** The app's node on this computer first, then this Mac's testnet validators. */
export const DEFAULT_RPCS = ['http://127.0.0.1:18545', 'http://127.0.0.1:8601', 'http://127.0.0.1:8602', 'http://127.0.0.1:8603', 'http://127.0.0.1:8604'];
const TIMEOUT = 8_000;
const PROBE_TIMEOUT = 2_500;

export class RpcError extends Error {
  constructor(message, code = -32603, data) {
    super(message);
    this.code = code;
    this.data = data;
  }
}

export class Rpc {
  /** `urls`: endpoints in order; `fetchImpl` for tests. */
  constructor(urls = DEFAULT_RPCS, { fetchImpl = (...a) => fetch(...a), now = () => Date.now() } = {}) {
    this.urls = [...new Set(urls.filter(Boolean))];
    this.fetch = fetchImpl;
    this.now = now;
    this.backoff = new Map(); // url -> {until, delay}
    this.current = null;
  }

  setUrls(urls) {
    this.urls = [...new Set(urls.filter(Boolean))];
    this.current = null;
    this.backoff.clear();
  }

  fail(url) {
    const prev = this.backoff.get(url);
    const delay = prev ? Math.min(prev.delay * 2, 60_000) : 5_000;
    this.backoff.set(url, { until: this.now() + delay, delay });
    if (this.current === url) this.current = null;
  }

  async post(url, method, params, timeout) {
    const ctl = new AbortController();
    const timer = setTimeout(() => ctl.abort(), timeout);
    try {
      const r = await this.fetch(url, {
        method: 'POST',
        signal: ctl.signal,
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ jsonrpc: '2.0', id: 1, method, params }),
      });
      return await r.json();
    } finally {
      clearTimeout(timer);
    }
  }

  /** The endpoint in use, found again if needed. Throws when none answers. */
  async endpoint() {
    if (this.current) return this.current;
    for (const url of this.urls) {
      const b = this.backoff.get(url);
      if (b && this.now() < b.until) continue;
      try {
        const j = await this.post(url, 'eth_chainId', [], PROBE_TIMEOUT);
        if (j.result && parseInt(j.result, 16) === CHAIN_ID) {
          this.backoff.delete(url);
          this.current = url;
          return url;
        }
      } catch { /* not answering */ }
      this.fail(url);
    }
    throw new RpcError(`No ${Brand.project} node answers. Turn on the node in the ${Brand.project} app, or add a node in Settings.`, 4900);
  }

  /**
   * Ask every endpoint in turn until one serves `method` (a node-local
   * service such as the testnet faucet runs on only some nodes).
   */
  async callAny(method, params = [], { notHere = /does not run|not supported|method not found/i } = {}) {
    let last = new RpcError(`No ${Brand.project} node answers.`, 4900);
    for (const url of this.urls) {
      try {
        const j = await this.post(url, method, params, TIMEOUT);
        if (!j.error) return j.result;
        last = new RpcError(j.error.message || 'node error', j.error.code ?? -32603, j.error.data);
        if (!notHere.test(last.message)) throw last;
      } catch (e) {
        if (e instanceof RpcError && !notHere.test(e.message)) throw e;
        if (!(e instanceof RpcError)) last = new RpcError(`node ${url} did not answer`, 4900);
      }
    }
    throw last;
  }

  /** Call `method`; a node error comes back as RpcError with the node's code. */
  async call(method, params = []) {
    const url = await this.endpoint();
    let j;
    try {
      j = await this.post(url, method, params, TIMEOUT);
    } catch (e) {
      this.fail(url);
      throw new RpcError(`node ${url} did not answer: ${e.message || e}`, 4900);
    }
    // An older node may not have this method yet: ask the others.
    if (j.error && j.error.code === -32601 && this.urls.length > 1) return this.callAny(method, params);
    if (j.error) throw new RpcError(j.error.message || 'node error', j.error.code ?? -32603, j.error.data);
    return j.result;
  }
}
