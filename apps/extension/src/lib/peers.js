// Shared by the explorer, site and extension (the packaging script copies
// this file). Peer addresses are discovery hints. Only the bundled network
// identity and the light verifier can authorize data shown to the visitor.

export const READ_ALPN = 'eastsea/read/1';
export const DEFAULT_RELAYS = []; // iroh's n0 public WebSocket relays
export const DEFAULT_PKARR_RELAYS = ['https://pkarr.pubky.app', 'https://pkarr.pubky.org', 'https://relay.pkarr.org'];
const MAX_PEERS = 64;
const ROTATE_MS = 60_000;
const HEAD_CACHE_MS = 4_000;
const ACCOUNT_WAIT_MS = 3_000;
const ACCOUNT_POLL_MS = 250;
const ACCOUNT_POLLS = ACCOUNT_WAIT_MS / ACCOUNT_POLL_MS + 1;
const PEER_BURST = 16;
const PEER_RATE = 8;
const PEER_INFLIGHT = 4;
const TRANSPORT_INFLIGHT = 32;
const monotonicNow = () => globalThis.performance?.now?.() ?? Date.now();
const METHODS = new Set(['aether_status', 'eth_chainId', 'net_version', 'eth_blockNumber',
  'aether_getBlock', 'aether_recentBlocks', 'aether_getAccount', 'eth_getBalance',
  'eth_getTransactionCount', 'aether_getReceipt', 'aether_getReceiptProof', 'aether_presence']);
// JSON fields named "verified" have no authority. Provenance belongs to the
// exact object that passed the verifier, even when other reads switch sources.
const provenance = new WeakMap();

export function readVerdict(value) {
  const v = value && typeof value === 'object' ? provenance.get(value) : null;
  return v?.verified ? { verified: true, height: v.height, reason: '' } : null;
}

export function readSource(value) { return value && typeof value === 'object' ? provenance.get(value)?.source : null; }

export function markHttpRead(value, source) {
  if (value && typeof value === 'object') {
    provenance.set(value, { verified: false, source });
    if (Array.isArray(value)) for (const item of value) markHttpRead(item, source);
  }
  return value;
}

function certifiedRead(value, height, peer) {
  provenance.set(value, { verified: true, height,
    source: { kind: 'peers', url: `iroh://${peer.node}`, verified: true } });
  return value;
}

export function isPublicRead(method) { return METHODS.has(method); }

function peerHint(value) {
  const p = typeof value === 'string' ? { node: value } : value;
  if (!p || !/^[a-f\d]{64}$/i.test(p.node)) return null;
  const hint = { node: p.node.toLowerCase() };
  if (typeof p.relay === 'string') {
    try {
      const u = new URL(p.relay);
      if (u.protocol === 'https:' || (u.protocol === 'http:' && ['localhost', '127.0.0.1', '[::1]'].includes(u.hostname))) hint.relay = u.href;
    } catch { /* ignore malformed discovery hints */ }
  }
  // Operator and relay diversity improve availability, never certificate trust.
  if (typeof p.operator === 'string') hint.operator = p.operator.slice(0, 128);
  return hint;
}

export function compiledPeers(network, release = null) {
  if (release && release.chain_id !== network.chain_id) throw new Error('peer seed file is for another chain');
  const peers = new Map();
  for (const value of [...(network.validators || []), ...(release?.peers || [])]) {
    const peer = peerHint(value);
    if (peer && peers.size < MAX_PEERS) peers.set(peer.node, { ...peers.get(peer.node), ...peer });
  }
  return [...peers.values()];
}

function integer(value, label) {
  if (!Number.isSafeInteger(value) || value < 0) throw new Error(`${label} is missing or invalid`);
  return value;
}

function decoded(value) { return typeof value === 'string' ? JSON.parse(value) : value; }
function hex(value) { return String(value).replace(/^0x/, '').toLowerCase(); }
function same(field, a, b) {
  if (['hash', 'parent', 'parent_state_root', 'proposer'].includes(field)) return hex(a) === hex(b);
  if (field === 'txs') return Array.isArray(a) && a.length === b.length && a.every((v, i) => hex(v) === hex(b[i]));
  return a === b;
}
function unavailable(method) {
  return Object.assign(new Error(`${method} has no supported proof on the public read service`), { code: -32601, unsupported: true });
}
function missingProof(message) { return Object.assign(new Error(message), { unavailable: true }); }

export class PublicPeerPool {
  constructor({ network, peers = [], transport, mod, floorStore = null, now = () => Date.now(),
    timeoutMs = 5_000, target = 3, onPeer = null,
    monotonic = monotonicNow,
    sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms)) } = {}) {
    if (!network?.identity || !Number.isSafeInteger(network.chain_id)) throw new Error('a pinned network identity is required');
    if (!transport?.call || typeof mod?.verifyBlock !== 'function') throw new Error('public reads require the iroh transport and light verifier');
    this.network = network;
    this.transport = transport;
    this.mod = mod;
    this.floorStore = floorStore;
    this.now = now;
    this.timeoutMs = timeoutMs;
    this.target = Math.max(3, Math.min(8, target));
    this.onPeer = onPeer;
    this.sleep = sleep;
    this.monotonic = monotonic;
    this.monotonicStartedAt = this.monotonic();
    this.monotonicFirstVerifiedHeadAt = null;
    this.pacing = new Map();
    this.inflight = 0;
    this.slotWaiters = new Set();
    this.candidates = new Map();
    this.active = new Map();
    this.dropped = new Map();
    this.blocks = new Map();
    this.accounts = new Map();
    this.receipts = new Map();
    this.floorTask = Promise.resolve();
    this.minimumHeight = 0;
    this.readyTask = null;
    this.cursor = 0;
    this.closed = false;
    this.startedAt = this.now();
    this.rotatedAt = this.startedAt;
    this.metrics = { firstVerifiedHeadMs: null, blockPageMs: null, rejectedPeers: 0 };
    this.addPeers(peers);
  }

  addPeers(peers) {
    for (const value of Array.isArray(peers) ? peers.slice(0, MAX_PEERS) : []) {
      const p = peerHint(value);
      if (p && !this.candidates.has(p.node) && this.candidates.size < MAX_PEERS) this.candidates.set(p.node, p);
    }
  }

  get livePeers() { return [...this.active.values()].map(({ peer }) => peer); }
  get source() { return { kind: 'peers', url: `iroh://${this.lastPeer?.node || this.livePeers[0]?.node || 'discovering'}`, peers: this.livePeers.length, verified: true }; }
  get url() { return this.source.url; }
  get kind() { return 'peers'; }

  async reserve(peer, deadline) {
    let bucket = this.pacing.get(peer.node);
    if (!bucket) {
      if (this.pacing.size >= MAX_PEERS) throw missingProof('public peer pacing limit reached');
      bucket = { tokens: PEER_BURST, last: this.monotonic(), firstResponse: false, inflight: 0, queue: Promise.resolve() };
      this.pacing.set(peer.node, bucket);
    }
    let expired = false, wake, timer;
    const deadlineError = () => missingProof('public read deadline expired while waiting for a request slot');
    const reservation = bucket.queue.then(async () => {
      for (;;) {
        if (this.closed) throw new Error('public peer pool is closed');
        const time = this.monotonic();
        const remaining = deadline - time;
        if (expired || remaining <= 0) throw deadlineError();
        if (bucket.inflight >= PEER_INFLIGHT || this.inflight >= TRANSPORT_INFLIGHT) {
          await new Promise((resolve) => {
            wake = () => { this.slotWaiters.delete(wake); wake = null; resolve(); };
            this.slotWaiters.add(wake);
          });
          continue;
        }
        const elapsed = Math.max(0, Math.floor(time - bucket.last));
        // Match the node's integral refill; preserve depleted buckets across
        // connection rotation so reconnecting cannot earn an extra burst.
        const refill = bucket.firstResponse ? Math.floor(elapsed * PEER_RATE / 1000) : 0;
        if (refill > 0) {
          bucket.tokens = Math.min(PEER_BURST, bucket.tokens + refill);
          bucket.last = time;
        }
        if (bucket.tokens > 0) {
          bucket.tokens--;
          bucket.inflight++;
          this.inflight++;
          return bucket;
        }
        const wait = bucket.firstResponse ? Math.max(1, Math.ceil(1000 / PEER_RATE - elapsed)) : 1000 / PEER_RATE;
        if (wait >= remaining) throw missingProof('public read deadline expires before a request slot is available');
        await this.sleep(wait);
      }
    });
    bucket.queue = reservation.catch(() => {});
    // Each caller's deadline also covers time behind earlier queued callers.
    // A cancelled reservation never acquires a slot when that queue resumes.
    const deadlineTask = new Promise((_, reject) => {
      timer = setTimeout(() => { expired = true; wake?.(); reject(deadlineError()); }, Math.max(0, deadline - this.monotonic()));
    });
    return Promise.race([reservation, deadlineTask]).finally(() => clearTimeout(timer));
  }

  async raw(peer, method, params = [], timeoutMs = this.timeoutMs) {
    if (this.closed) throw new Error('public peer pool is closed');
    const deadline = this.monotonic() + timeoutMs;
    const bucket = await this.reserve(peer, deadline);
    const release = () => {
      bucket.inflight--;
      this.inflight--;
      for (const wake of this.slotWaiters) wake();
    };
    let timer, wire;
    try {
      const remaining = deadline - this.monotonic();
      if (this.closed) throw new Error('public peer pool is closed');
      if (remaining <= 0) throw missingProof('public read deadline expired before transport');
      wire = Promise.resolve(this.transport.call(JSON.stringify(peer), method, JSON.stringify(params)));
      // A JS timeout does not cancel an iroh call. Return its permits only when
      // the actual operation settles, including late failures after the race.
      wire.then(release, release);
      const result = await Promise.race([
        wire,
        new Promise((_, reject) => { timer = setTimeout(() => reject(new Error('public peer timed out')), remaining); }),
      ]);
      if (!bucket.firstResponse) {
        // A cold connection may take longer than a refill period to open.
        // Start refill after a response proves that the remote bucket exists.
        bucket.firstResponse = true;
        bucket.last = this.monotonic();
      }
      return decoded(result);
    } catch (e) {
      if (!bucket.firstResponse) {
        bucket.firstResponse = true;
        bucket.last = this.monotonic();
      }
      // Pruning or a legacy block without receipt commitments is lack of data,
      // not evidence that this otherwise certified peer lied about the chain.
      if (/pruned:|receipt proofs unavailable|receipt proof unavailable|receipt certificate unavailable|too many public read calls in flight|public read peer pool is full/i.test(e?.message || '')) e.unavailable = true;
      throw e;
    } finally {
      if (!wire) release();
      clearTimeout(timer);
    }
  }

  async floor() {
    const v = await this.floorStore?.get(`verifiedHeight.${this.network.chain_id}`);
    return Math.max(this.minimumHeight, v == null ? 0 : integer(v, 'stored verified height'));
  }

  async commitHeight(height) {
    integer(height, 'certified height');
    const task = this.floorTask.catch(() => {}).then(async () => {
      if (this.closed) throw new Error('public peer pool is closed');
      if (height < await this.floor()) throw new Error('finalized blocks never go back');
      this.minimumHeight = height;
      await this.floorStore?.set(`verifiedHeight.${this.network.chain_id}`, height);
    });
    this.floorTask = task;
    await task;
  }

  async verifyBlock(peer, status, height, fresh = false) {
    const finalized = await this.raw(peer, 'aether_getFinalized', [height]);
    if (!finalized) throw missingProof(`block ${height} not finalized yet`);
    const block = decoded(await this.mod.verifyBlock(JSON.stringify(this.network), JSON.stringify(status),
      JSON.stringify(finalized), BigInt(height), BigInt(fresh ? await this.floor() : 0), BigInt(this.now()), fresh));
    integer(block?.height, 'certified height');
    if (block.height !== height || block.chain_id !== this.network.chain_id || !Array.isArray(block.txs)) throw new Error('verified block is for another height or chain');
    return block;
  }

  async head(peer, force = false) {
    const old = this.active.get(peer.node);
    if (!force && old && this.now() - old.checkedAt < HEAD_CACHE_MS && old.head.height >= await this.floor()) return old.head;
    const status = await this.raw(peer, 'aether_status');
    const height = integer(status?.height, 'head height');
    const block = await this.verifyBlock(peer, status, height, true);
    await this.commitHeight(block.height);
    const head = { chain_id: block.chain_id, height: block.height, hash: block.hash,
      timestamp_ms: block.timestamp_ms, protocol: block.protocol, hash_function: 'blake3',
      parent_state_root: block.parent_state_root, state_root: null, mempool: null, base_fee: null,
      prover_escrow: null, node_protocol: null, newest_scheduled: null, verified: true };
    this.active.set(peer.node, { peer, head, checkedAt: this.now(), latencyMs: this.now() - (old?.startedAt || this.now()) });
    certifiedRead(head, block.height, peer);
    if (this.metrics.firstVerifiedHeadMs === null) {
      this.monotonicFirstVerifiedHeadAt = this.monotonic();
      this.metrics.firstVerifiedHeadMs = Math.max(0, this.monotonicFirstVerifiedHeadAt - this.monotonicStartedAt);
    }
    return head;
  }

  drop(peer, reason) {
    this.active.delete(peer.node);
    this.dropped.set(peer.node, { until: this.now() + (/timed out|busy|rate limit|cap|not finalized|stale|never go back/i.test(reason) ? 60_000 : 24 * 60 * 60_000), reason });
    this.transport.closePeer?.(peer.node);
    this.metrics.rejectedPeers++;
    this.onPeer?.({ node: peer.node, reason, dropped: true });
  }

  candidate() {
    const active = this.livePeers;
    const possible = [...this.candidates.values()].filter((p) => !this.active.has(p.node)
      && (!this.dropped.has(p.node) || this.dropped.get(p.node).until <= this.now()));
    const score = (p) => (p.operator && !active.some((a) => a.operator === p.operator) ? 2 : 0)
      + (p.relay && !active.some((a) => a.relay === p.relay) ? 1 : 0);
    return possible.sort((a, b) => score(b) - score(a))[0];
  }

  async maintain() {
    if (this.readyTask) return this.readyTask;
    this.readyTask = (async () => {
      if (this.now() - this.rotatedAt >= ROTATE_MS) {
        this.rotatedAt = this.now();
        const replacement = this.candidate();
        if (replacement && this.active.size >= this.target) {
          const oldest = [...this.active.values()].sort((a, b) => a.checkedAt - b.checkedAt)[0];
          this.active.delete(oldest.peer.node);
          this.transport.closePeer?.(oldest.peer.node);
          // Keep the rotated peer available for a later turn, not this refill.
          this.dropped.set(oldest.peer.node, { until: this.now() + ROTATE_MS, reason: 'rotation' });
        }
      }
      for (let attempts = 0; attempts < MAX_PEERS && this.active.size < this.target; attempts++) {
        const p = this.candidate();
        if (!p) break;
        try {
          await this.head(p, true);
          // Discovery is a hint only. A new address earns no trust until its
          // own BLS certificate passes against the bundled network identity.
          this.addPeers(await this.raw(p, 'aether_readPeers').catch(() => []));
        } catch (e) { this.drop(p, e?.message || String(e)); }
      }
    })().finally(() => { this.readyTask = null; });
    return this.readyTask;
  }

  async attempt(fn) {
    await this.maintain();
    const peers = this.livePeers;
    const start = peers.length ? this.cursor++ % peers.length : 0;
    let last = new Error('no public peer served a verified answer');
    for (let i = 0; i < peers.length; i++) {
      const p = peers[(start + i) % peers.length];
      if (!this.active.has(p.node)) continue;
      try {
        const result = await fn(p);
        this.lastPeer = p;
        if (this.active.size < this.target) await this.maintain();
        this.onPeer?.({ node: p.node, verified: true });
        return result;
      } catch (e) {
        if (e?.unsupported) throw e;
        last = e;
        if (!e?.unavailable) this.drop(p, e?.message || String(e));
      }
    }
    throw new Error(`no public peer served a verified answer: ${last?.message || last}`);
  }

  async block(peer, height) {
    integer(height, 'block height');
    const status = await this.head(peer);
    const [certified, summary] = await Promise.all([
      this.verifyBlock(peer, status, height), this.raw(peer, 'aether_getBlock', [height]),
    ]);
    if (!summary) throw missingProof(`block ${height} is unavailable on this peer`);
    for (const field of ['height', 'hash', 'parent', 'timestamp_ms', 'proposer', 'parent_state_root', 'txs', 'gas_used', 'prove_gas']) {
      if (!same(field, summary[field], certified[field])) throw new Error(`block ${field} differs from certificate`);
    }
    // The state after the block and the next base fees require separate state
    // proofs. Do not copy such fields from the peer's display summary.
    const result = { ...certified, state_root: null, base_fee: null, excess: null, verified: true, pruned: false };
    if (this.blocks.size >= 64) this.blocks.delete(this.blocks.keys().next().value);
    this.blocks.set(height, result);
    return certifiedRead(result, height, peer);
  }

  async account(peer, address) {
    if (!/^0x[a-f\d]{40}$/i.test(address)) throw new Error('an account address is required');
    if (typeof this.mod.verifyAccount !== 'function') throw unavailable('aether_getAccount');
    const status = await this.head(peer);
    const answer = await this.raw(peer, 'aether_getAccount', [address]);
    const stateHeight = integer(answer?.height, 'account height');
    const targetHeight = integer(stateHeight + 1, 'account certificate height');
    const deadline = this.monotonic() + ACCOUNT_WAIT_MS;
    let finalized = null;
    // The state proof is fixed at H. A newly executed H needs the child H+1
    // to commit its root; fetching a new account each retry would chase the
    // advancing tip forever. Poll only null, never a malformed certificate.
    for (let poll = 0; poll < ACCOUNT_POLLS; poll++) {
      const remaining = deadline - this.monotonic();
      if (remaining <= 0) break;
      finalized = await this.raw(peer, 'aether_getFinalized', [targetHeight], Math.min(this.timeoutMs, remaining));
      if (finalized !== null) break;
      if (poll < ACCOUNT_POLLS - 1) await this.sleep(Math.max(0, Math.min(ACCOUNT_POLL_MS, deadline - this.monotonic())));
    }
    if (finalized === null) throw missingProof('account state is not finalized yet');
    const trusted = decoded(await this.mod.verifyAccount(JSON.stringify(this.network), JSON.stringify(status),
      JSON.stringify(answer), JSON.stringify(finalized), address, BigInt(await this.floor()), BigInt(this.now())));
    await this.commitHeight(Number(trusted.certified_block));
    // The account proof authenticates balance and nonce, not code_size.
    const result = { address: trusted.address, balance: trusted.balance_wei, nonce: trusted.nonce,
      height: trusted.state_height, state_root: answer.state_root, code_size: null, verified: true,
      certified_block: trusted.certified_block, timestamp_ms: trusted.timestamp_ms };
    if (this.accounts.size >= 64) this.accounts.delete(this.accounts.keys().next().value);
    this.accounts.set(address.toLowerCase(), result);
    return certifiedRead(result, Number(trusted.certified_block), peer);
  }

  async receipt(peer, hash) {
    if (!/^0x[a-f\d]{64}$/i.test(hash)) throw new Error('a transaction hash is required');
    if (typeof this.mod.verifyReceipt !== 'function') throw unavailable('aether_getReceiptProof');
    const status = await this.head(peer);
    const answer = await this.raw(peer, 'aether_getReceiptProof', [hash]);
    if (!answer) throw missingProof('no certified receipt is available for this hash');
    const trusted = decoded(await this.mod.verifyReceipt(JSON.stringify(this.network), JSON.stringify(status),
      JSON.stringify(answer), JSON.stringify(answer.certified_block), 0n, BigInt(this.now())));
    if (hex(trusted.receipt?.tx_hash) !== hex(hash)) throw new Error('receipt proof is for another transaction');
    const result = { ...trusted, verified: true };
    if (this.receipts.size >= 64) this.receipts.delete(this.receipts.keys().next().value);
    this.receipts.set(hash.toLowerCase(), result);
    return certifiedRead(result, Number(trusted.height), peer);
  }

  async call(method, params = []) {
    if (!isPublicRead(method)) throw unavailable(method);
    if (method === 'aether_getBlock') integer(params[0], 'block height');
    if (['aether_getReceipt', 'aether_getReceiptProof'].includes(method) && !/^0x[a-f\d]{64}$/i.test(params[0])) throw new Error('a transaction hash is required');
    if (['aether_getAccount', 'eth_getBalance', 'eth_getTransactionCount'].includes(method) && !/^0x[a-f\d]{40}$/i.test(params[0])) throw new Error('an account address is required');
    if (method === 'aether_presence') return { available: false, committed: false, reason: 'Public presence is an uncommitted peer aggregate.' };
    if (method === 'aether_recentBlocks') {
      const count = Math.min(30, integer(params[0] ?? 10, 'block count'));
      const head = await this.call('aether_status');
      const result = [];
      for (let height = head.height; height > Math.max(0, head.height - count); height--) {
        result.push(await this.call('aether_getBlock', [height]));
      }
      return result;
    }
    const started = this.now();
    const result = await this.attempt(async (peer) => {
      if (method === 'aether_status') return this.head(peer);
      if (method === 'eth_chainId' || method === 'net_version') {
        await this.head(peer);
        return method === 'eth_chainId' ? `0x${this.network.chain_id.toString(16)}` : String(this.network.chain_id);
      }
      if (method === 'eth_blockNumber') return `0x${(await this.head(peer)).height.toString(16)}`;
      if (method === 'aether_getBlock') return this.block(peer, params[0]);
      if (['aether_getReceipt', 'aether_getReceiptProof'].includes(method)) return this.receipt(peer, params[0]);
      const a = await this.account(peer, params[0]);
      if (method === 'eth_getBalance') return `0x${BigInt(a.balance).toString(16)}`;
      if (method === 'eth_getTransactionCount') return `0x${BigInt(a.nonce).toString(16)}`;
      return a;
    });
    if (method === 'aether_getBlock' && this.metrics.blockPageMs === null) this.metrics.blockPageMs = Math.max(0, this.now() - started);
    return result;
  }

  verdict(kind, key) {
    const value = kind === 'block' ? this.blocks.get(key) : kind === 'account' ? this.accounts.get(String(key).toLowerCase()) : this.receipts.get(String(key).toLowerCase());
    return { verified: !!value, height: value ? Number(value.certified_block ?? value.height) : null, reason: value ? '' : 'no verified answer has been read' };
  }

  close() {
    this.closed = true;
    for (const wake of this.slotWaiters) wake();
    this.active.clear();
    this.pacing.clear();
    return this.transport.close?.();
  }
}

export async function createPublicPeerPool(env, { release = null, relays = DEFAULT_RELAYS,
  pkarrRelays = DEFAULT_PKARR_RELAYS, ...options } = {}) {
  if (!env?.mod?.PublicReadTransport || typeof env.mod.verifyBlock !== 'function') return null;
  const transport = await env.mod.PublicReadTransport.create(JSON.stringify(relays), JSON.stringify(pkarrRelays));
  const floorStore = env.floorStore || {
    get: async (key) => {
      try {
        const v = env.storage?.getItem(`aether-explorer.${key}`);
        return v == null ? undefined : Number(v);
      } catch { return undefined; }
    },
    set: async (key, value) => {
      try { env.storage?.setItem(`aether-explorer.${key}`, String(value)); }
      catch { /* the instance still retains the verified floor */ }
    },
  };
  return new PublicPeerPool({ network: env.network, mod: env.mod, transport, floorStore,
    now: env.now, sleep: env.sleep, monotonic: env.monotonic, peers: compiledPeers(env.network, release), ...options });
}

/** Resource loading stays injectable for browser/devnet checks and hosted
 * copies. An absent supplemental seed file leaves the bundled keys intact. */
export async function loadPublicPeerPool(io = {}) {
  let release = io.release ?? null;
  if (io.release === undefined) {
    try {
      const r = await (io.fetch || globalThis.fetch)('public-read-peers.json');
      if (r.ok) release = await r.json();
    } catch { /* known node keys are sufficient bootstrap hints */ }
  }
  const env = io.env || { network: io.network, mod: io.mod, floorStore: io.floorStore,
    storage: io.storage, now: io.now };
  return createPublicPeerPool(env, { ...io, release });
}
