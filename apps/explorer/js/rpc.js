// JSON-RPC 2.0 client for an Aether node's HTTP endpoint
// (crates/node/src/rpc.rs — POST /, any origin, read-only here: the explorer
// signs nothing and sends nothing the mempool would take). fetch and storage
// are injectable so the tests run without a node (test/rpc.test.mjs).

export const DEFAULT_ENDPOINT = 'http://127.0.0.1:18545';
const STORAGE_KEY = 'aether-explorer.node';

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

/** The saved endpoint, or the default (a node on this Mac). */
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
      throw new RpcError('the node did not answer JSON (is this an Aether node?)', -1);
    }
    if (body?.error) throw new RpcError(body.error.message, body.error.code);
    return body?.result;
  }

  /** `eth_call(to, data) -> hex`, the reader the ERC-20 helpers take. */
  read(to, data) {
    return this.call('eth_call', [{ to, data }, 'latest']);
  }
}
