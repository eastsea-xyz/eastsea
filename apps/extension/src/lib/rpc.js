import { Brand } from './brand.js';
import { isPublicRead } from './peers.js';
// JSON-RPC to Aether nodes. The first endpoint that answers on the right chain
// is used; one that does not answer sleeps 5 s, doubling to 60 s.

/** The app's node follows the network.json bundled with the app. */
export const DEFAULT_RPCS = ['http://127.0.0.1:18545'];
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
  constructor(urls = DEFAULT_RPCS, { chainId = 7780, fetchImpl = (...a) => fetch(...a), now = () => Date.now(), verifyAccount = null, network = null, floorStore = null, peerPool = null } = {}) {
    this.urls = [...new Set(urls.filter(Boolean))];
    this.chainId = chainId;
    this.fetch = fetchImpl;
    this.now = now;
    this.backoff = new Map(); // url -> {until, delay}
    this.current = null;
    this.verifyAccount = verifyAccount;
    this.network = network;
    this.floorStore = floorStore;
    if (verifyAccount && !floorStore) throw new Error('Verified reads require persistent height storage.');
    this.floorTask = Promise.resolve();
    this.generation = 0;
    this.verifiedAccounts = new Map();
    this.peerPool = peerPool;
  }

  setVerifier(network) {
    this.network = network;
    this.generation++;
  }

  setPeerPool(pool) {
    this.peerPool?.close?.();
    this.peerPool = pool;
  }

  async publicRead(method, params, failure) {
    if (!this.peerPool || !isPublicRead(method)) throw failure;
    const pool = this.peerPool;
    const generation = this.generation;
    const chainId = this.chainId;
    try {
      const result = await pool.call(method, params);
      if (generation !== this.generation || chainId !== this.chainId || pool !== this.peerPool) throw new Error('network changed during peer verification');
      this.current = null; // writes and HTTP-local services still need an HTTP node
      if (['aether_getAccount', 'eth_getBalance', 'eth_getTransactionCount'].includes(method)) {
        const a = this.peerPool.accounts?.get(String(params[0]).toLowerCase());
        if (a) this.verifiedAccounts.set(String(params[0]).toLowerCase(), { height: Number(a.certified_block), timestampMs: Number(a.timestamp_ms) });
      }
      return result;
    } catch (e) { throw new RpcError(`No peer served a verified read: ${e?.message || e}`, 4900); }
  }

  setUrls(urls) {
    this.urls = [...new Set(urls.filter(Boolean))];
    this.current = null;
    this.backoff.clear();
  }

  setChain(chainId, urls) {
    this.setPeerPool(null);
    this.chainId = chainId;
    this.generation++;
    this.verifiedAccounts.clear();
    this.setUrls(urls);
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
        if (j.result && parseInt(j.result, 16) === this.chainId) {
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
        const chain = await this.post(url, 'eth_chainId', [], PROBE_TIMEOUT);
        if (parseInt(chain.result, 16) !== this.chainId) continue;
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
    if (this.verifyAccount && this.network && ['aether_getAccount', 'eth_getBalance', 'eth_getTransactionCount'].includes(method)) {
      try { return await this.callVerifiedAccount(method, params); }
      catch (e) { return this.publicRead(method, params, e); }
    }
    let url;
    try { url = await this.endpoint(); }
    catch (e) { return this.publicRead(method, params, e); }
    let j;
    try {
      j = await this.post(url, method, params, TIMEOUT);
    } catch (e) {
      this.fail(url);
      return this.publicRead(method, params, new RpcError(`node ${url} did not answer: ${e.message || e}`, 4900));
    }
    // An older node may not have this method yet: ask the others.
    if (j.error && j.error.code === -32601 && this.urls.length > 1) return this.callAny(method, params);
    if (j.error) throw new RpcError(j.error.message || 'node error', j.error.code ?? -32603, j.error.data);
    return j.result;
  }

  /**
   * Ask every configured endpoint the same question and answer only when the
   * ones that answer all agree — used for a token's first-seen metadata
   * (audit A3: one untrusted RPC must not decide the units a transfer signs).
   * Off-line endpoints and wrong-chain answers are skipped; an endpoint that
   * answers differently is a disagreement, not a vote. Returns
   * `{ result, sources }`, `sources` being how many endpoints agreed.
   */
  async callAgreed(method, params = []) {
    const answers = [];
    for (const url of this.urls) {
      const b = this.backoff.get(url);
      if (b && this.now() < b.until) continue;
      try {
        const chain = await this.post(url, 'eth_chainId', [], PROBE_TIMEOUT);
        if (parseInt(chain.result, 16) !== this.chainId) continue;
        const j = await this.post(url, method, params, TIMEOUT);
        if (j.error) continue;
        answers.push(j.result);
      } catch { /* not answering */ }
    }
    if (!answers.length) throw new RpcError(`No ${Brand.project} node answers. Turn on the node in the ${Brand.project} app, or add a node in Settings.`, 4900);
    const first = JSON.stringify(answers[0]);
    if (answers.some((a) => JSON.stringify(a) !== first)) {
      const e = new RpcError('The nodes did not agree on one answer.', -32603);
      e.disagreed = true;
      throw e;
    }
    return { result: answers[0], sources: answers.length };
  }

  async checkedPost(url, method, params) {
    const answer = await this.post(url, method, params, TIMEOUT);
    if (answer.error) throw new RpcError(answer.error.message || 'node error', answer.error.code ?? -32603, answer.error.data);
    if (!Object.hasOwn(answer, 'result')) throw new Error(`missing ${method} result`);
    return answer.result;
  }

  async heightFloor(key) {
    const stored = await this.floorStore.get(key);
    if (stored === undefined) return 0;
    if (!Number.isSafeInteger(stored) || stored < 0) throw new Error('Stored verified height is invalid.');
    return stored;
  }

  async certifiedAt(url, height) {
    for (let attempt = 0; attempt < 40; attempt++) {
      const answer = await this.checkedPost(url, 'aether_getFinalized', [height]);
      if (answer != null) return answer;
      if (attempt < 39) await new Promise((resolve) => setTimeout(resolve, 250));
    }
    throw new Error(`block ${height} not finalized yet`);
  }

  parkVerified(url, kind) {
    const limits = { busy: [1_000, 30_000], error: [2_000, 300_000], stale: [5_000, 60_000], lying: [600_000, 21_600_000] };
    const [base, cap] = limits[kind];
    const previous = this.backoff.get(url);
    const delay = previous?.kind === kind ? Math.min(previous.delay * 2, cap) : base;
    this.backoff.set(url, { kind, until: this.now() + delay, delay });
    if (this.current === url) this.current = null;
  }

  async callVerifiedAccount(method, params) {
    const address = params[0];
    if (typeof address !== 'string') throw new RpcError('An account address is required.', -32602);
    const generation = this.generation;
    let last = new RpcError('No node served a verified account.', 4900);
    for (const url of this.urls) {
      const parked = this.backoff.get(url);
      if (parked && this.now() < parked.until) continue;
      try {
        const account = await this.checkedPost(url, 'aether_getAccount', [address]);
        const height = account?.height;
        if (!Number.isSafeInteger(height) || height < 0) {
          const bad = new Error('account height is missing');
          bad.verification = true;
          throw bad;
        }
        const [finalized, status] = await Promise.all([
          this.certifiedAt(url, height + 1),
          this.checkedPost(url, 'aether_status', []),
        ]);
        const floorKey = `verifiedHeight.${this.chainId}`;
        const floor = await this.heightFloor(floorKey);
        let verified;
        try {
          verified = JSON.parse(await this.verifyAccount(JSON.stringify(this.network), JSON.stringify(status),
            JSON.stringify(account), JSON.stringify(finalized), address, BigInt(floor), BigInt(this.now())));
        } catch (e) {
          const failure = new Error(e?.message || String(e));
          failure.verification = true;
          throw failure;
        }
        if (generation !== this.generation) throw new Error('network changed during verification');
        const certified = Number(verified.certified_block);
        if (!Number.isSafeInteger(certified) || certified < floor) {
          const failure = new Error('invalid certified height');
          failure.verification = true;
          throw failure;
        }
        const commit = this.floorTask.catch(() => {}).then(async () => {
          const latest = await this.heightFloor(floorKey);
          if (generation !== this.generation || certified < latest) throw new Error('finalized blocks never go back');
          if (certified > latest) await this.floorStore.set(floorKey, certified);
        });
        this.floorTask = commit;
        await commit;
        this.backoff.delete(url);
        this.current = url;
        if (this.verifiedAccounts.size >= 16) this.verifiedAccounts.delete(this.verifiedAccounts.keys().next().value);
        this.verifiedAccounts.set(address.toLowerCase(), { height: certified, timestampMs: Number(verified.timestamp_ms) });
        if (method === 'eth_getBalance') return `0x${BigInt(verified.balance_wei).toString(16)}`;
        if (method === 'eth_getTransactionCount') return `0x${BigInt(verified.nonce).toString(16)}`;
        return account;
      } catch (e) {
        last = new RpcError(`node ${url} did not serve a verified account: ${e.message || e}`, 4900);
        const kind = /server busy/i.test(e.message) ? 'busy'
          : /stale|never go back|not finalized yet/i.test(e.message) ? 'stale'
            : e.verification ? 'lying' : 'error';
        this.parkVerified(url, kind);
      }
    }
    throw last;
  }
}
