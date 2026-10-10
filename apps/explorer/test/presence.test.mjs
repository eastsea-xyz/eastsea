// Offline presence RPC, page states, and polling. A zero requires a valid
// observation; an older node or a failed read must not look like an empty net.
import test from 'node:test';
import assert from 'node:assert/strict';
import { homeView } from '../js/pages.js';
import { parsePresence, readPresence } from '../js/presence.js';
import { pollCurrentPage } from '../js/polling.js';

const presence = (observedAt = Math.floor(Date.now() / 600_000) * 600) => ({
  schema: 2,
  available: true,
  total: 6,
  by_role: { validator: 3, other: 3 },
  by_version: { unknown: 6 },
  by_region: { asia: 3, world: 3 },
  ttl_seconds: 600,
  minimum_bucket_size: 3,
  observed_at: observedAt,
  scope: 'unverified cohort observation',
});

test('aggregate schema retains only safe, complete role and region partitions', () => {
  assert.deepEqual(parsePresence(presence()), {
    total: 6,
    byRole: { validator: 3, other: 3 },
    byVersion: { unknown: 6 },
    byRegion: { asia: 3, world: 3 },
    ttlSeconds: 600,
  });
});

test('unavailable, old-schema, partial, and inconsistent snapshots are not zero', () => {
  for (const answer of [null, {}, { ...presence(), available: false }, { ...presence(), schema: 1 },
    { ...presence(), total: -1 }, { ...presence(), observed_at: 'now' },
    { ...presence(), by_role: { validator: 4, follower: 2 } },
    { ...presence(), by_role: { validator: '3', other: 3 } },
    { ...presence(), by_version: { unknown: 5 } },
    { ...presence(), by_region: { asia: 5 } },
    { ...presence(), by_region: { asia: 3, europe: 2, unknown: 1 } },
    { ...presence(), nodes: [] }, { ...presence(), observer: '11'.repeat(32) },
    { ...presence(), by_country: { KR: 6 } }, { ...presence(), country: 'KR' },
    { ...presence(), minimum_bucket_size: 2 },
    { ...presence(), scope: 'six physical Macs' },
    { ...presence(), ttl_seconds: 0 },
  ]) assert.equal(parsePresence(answer), null);
});

test('an available small count is withheld rather than reported as zero', () => {
  const empty = presence();
  empty.total = null;
  empty.by_role = {};
  empty.by_version = {};
  empty.by_region = {};
  assert.equal(parsePresence(empty).total, null);
  for (const total of [0, 1, 2]) assert.equal(parsePresence({ ...empty, total }), null);
});

test('only rounded ten-minute snapshots are accepted, with bounded future skew', () => {
  const now = 1200;
  assert.equal(parsePresence(presence(now), now).total, 6);
  assert.equal(parsePresence(presence(now), now + 599).total, 6);
  assert.equal(parsePresence(presence(now), now + 600), null);
  assert.equal(parsePresence(presence(now + 1), now), null, 'exact timestamps are rejected');
  assert.equal(parsePresence(presence(now), now - 60).total, 6);
  assert.equal(parsePresence(presence(now), now - 61), null);
  for (const ttl_seconds of [undefined, null, 0, 180, 599, 601, '600']) {
    assert.equal(parsePresence({ ...presence(now), ttl_seconds }, now), null, `TTL ${ttl_seconds} is unknown`);
  }
});

test('presence is an explicit read with empty params; old methods and transport failures are unavailable', async () => {
  const calls = [];
  const node = { call: async (...args) => { calls.push(args); return presence(); } };
  assert.equal((await readPresence(node)).total, 6);
  assert.deepEqual(calls, [['aether_presence', []]]);
  for (const error of [new Error('method not found'), new Error('connection refused')]) {
    assert.equal(await readPresence({ call: async () => { throw error; } }), null);
  }
  assert.equal(await readPresence({ call: async () => ({ ...presence(), available: false }) }), null);
  assert.equal(await readPresence({ call: async () => presence(presence().observed_at - 600) }), null);
  assert.equal(await readPresence({ call: async () => presence(presence().observed_at + 1200) }), null);
});

// The same minimal text-only DOM used by the explorer's manual live smoke.
class El {
  constructor(tag) { this.tagName = tag; this.children = []; this.attrs = {}; }
  setAttribute(key, value) { this.attrs[key] = value; }
  set className(value) { this.attrs.class = value; }
  addEventListener() {}
  append(...children) { this.children.push(...children.flat()); }
}
globalThis.Node = El;
globalThis.document = { createElement: (tag) => new El(tag) };
const text = (el) => typeof el === 'string' ? el : (el?.children || []).map(text).join(' ');
function find(el, predicate) {
  if (!(el instanceof El)) return null;
  if (predicate(el)) return el;
  for (const child of el.children) {
    const found = find(child, predicate);
    if (found) return found;
  }
  return null;
}
function context(answer) {
  return { node: {
    url: 'http://offline.test',
    async call(method) {
      if (method === 'aether_presence') {
        if (answer instanceof Error) throw answer;
        return answer;
      }
      if (method === 'aether_status') return {
        height: 42, timestamp_ms: 1_791_446_400_000, protocol: 7, node_protocol: 7, newest_scheduled: 7,
        chain_id: 7780, mempool: 0, base_fee: { exec: '1', prove: '1' }, hash_function: 'Blake3',
        state_root: 'aa'.repeat(32), prover_escrow: '0',
      };
      if (method === 'aether_recentBlocks') return [];
      if (method === 'aether_candidates' || method === 'aether_proverStatus') return null;
      throw new Error(`Unexpected method ${method}`);
    },
  } };
}

test('home live panel renders safe role/region counts and the unverified scope', async () => {
  const home = await homeView(context(presence()));
  const live = find(home, (el) => el.tagName === 'section' && text(el.children[0]) === 'Live network');
  const copy = text(live);
  assert.ok(live);
  for (const needle of ['6 observed transports', 'unverified cohort observation', 'checks every 10 seconds',
    'Validator 3', 'Other 3', 'Asia 3', 'All regions 3',
    'Small groups are merged', '600 seconds', 'separate from consensus',
  ]) assert.ok(copy.includes(needle), `${needle} is visible`);
  assert.ok(!copy.includes('Country'));
  assert.ok(!copy.includes('Candidate 0'), 'absent/suppressed groups do not become zeros');
  assert.ok(text(home).includes('Latest blocks'), 'chain data stays visible');
});

test('an unavailable or stale presence leaves the home working and never claims zero Macs', async () => {
  for (const answer of [new Error('method not found'), { ...presence(), available: false }, { schema: 1, total: 0 },
    presence(presence().observed_at - 600), presence(presence().observed_at + 1200),
    { ...presence(), ttl_seconds: 180 },
  ]) {
    const home = await homeView(context(answer));
    const live = find(home, (el) => el.tagName === 'section' && text(el.children[0]) === 'Live network');
    assert.ok(text(live).includes('Unavailable'));
    assert.ok(text(live).includes('unverified cohort observation'));
    assert.ok(text(live).includes('Settings'));
    assert.ok(!text(live).includes('0 Macs'));
    assert.ok(text(home).includes('Finalized height'));
  }
});

test('identifying old records and arbitrary quality labels cannot render', async () => {
  const snapshot = presence();
  snapshot.by_version = { '<img src=x onerror=alert(1)>': 6 };
  snapshot.nodes = [{ node_id: '22'.repeat(32), ip: '192.0.2.1', city: 'Seoul' }];
  const home = await homeView(context(snapshot));
  assert.ok(text(home).includes('Unavailable'));
  assert.equal(find(home, (el) => el.tagName === 'img'), null);
  assert.ok(!text(home).includes('192.0.2.1'));
  assert.ok(!text(home).includes('Seoul'));
});

test('withheld cohorts never claim there are zero transports or Macs', async () => {
  const withheld = { ...presence(), total: null, by_role: {}, by_version: {}, by_region: {} };
  const home = await homeView(context(withheld));
  const live = find(home, (el) => el.tagName === 'section' && text(el.children[0]) === 'Live network');
  assert.ok(text(live).includes('Count withheld'));
  assert.ok(text(live).includes('Groups smaller than three'));
  assert.ok(!text(live).includes('0 observed'));
  assert.ok(text(home).includes('Latest blocks'));
});

test('polling runs every ten seconds for visible home pages or pending transactions', () => {
  let tick;
  let interval;
  let refreshes = 0;
  const state = { hidden: false, hash: '#/', pending: false };
  const timer = pollCurrentPage(() => { refreshes++; }, {
    getState: () => state,
    schedule: (fn, ms) => { tick = fn; interval = ms; return 99; },
  });
  assert.equal(timer, 99);
  assert.equal(interval, 10_000);
  for (const hash of ['', '#', '#/']) {
    state.hash = hash;
    tick();
  }
  assert.equal(refreshes, 3);
  state.hidden = true;
  tick();
  assert.equal(refreshes, 3, 'hidden home does not read');
  state.hidden = false;
  state.hash = '#/block/42';
  tick();
  assert.equal(refreshes, 3, 'another page does not read');
  state.hash = '#/tx/example';
  state.pending = true;
  tick();
  assert.equal(refreshes, 4, 'a pending transaction still updates');
  state.hidden = true;
  tick();
  assert.equal(refreshes, 4, 'a hidden pending transaction does not read');
});
