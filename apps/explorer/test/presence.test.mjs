// Offline presence RPC, page states, and polling. A zero requires a valid
// observation; an older node or a failed read must not look like an empty net.
import test from 'node:test';
import assert from 'node:assert/strict';
import { homeView } from '../js/pages.js';
import { parsePresence, readPresence } from '../js/presence.js';
import { pollCurrentPage } from '../js/polling.js';

const presence = (observedAt = Math.floor(Date.now() / 1000)) => ({
  schema: 1,
  available: true,
  total: 6,
  by_role: { validator: 4, candidate: 0, follower: 2 },
  by_version: { '0.7.3': 2, '0.7.4': 4 },
  by_region: { asia: 3, europe: 1, north_america: 1, south_america: 0, africa: 0, oceania: 0, unknown: 1 },
  by_country: { KR: 1 },
  nodes: [],
  ttl_seconds: 180,
  observed_at: observedAt,
  observer: '11'.repeat(32),
  scope: 'what this node can see',
});

test('presence schema keeps roles, multiple versions, and all continent counts', () => {
  assert.deepEqual(parsePresence(presence()), {
    total: 6,
    byRole: { validator: 4, candidate: 0, follower: 2 },
    byVersion: { '0.7.3': 2, '0.7.4': 4 },
    byRegion: { asia: 3, europe: 1, north_america: 1, south_america: 0, africa: 0, oceania: 0, unknown: 1 },
    ttlSeconds: 180,
  });
});

test('unavailable, old-schema, partial, and inconsistent snapshots are not zero', () => {
  for (const answer of [null, {}, { ...presence(), available: false }, { ...presence(), schema: 2 },
    { ...presence(), total: -1 }, { ...presence(), observed_at: 'now' },
    { ...presence(), by_role: { validator: 4, follower: 2 } },
    { ...presence(), by_role: { validator: '4', candidate: 0, follower: 2 } },
    { ...presence(), by_version: { '0.7.4': 5 } },
    { ...presence(), by_region: { asia: 6 } },
    { ...presence(), ttl_seconds: 0 },
  ]) assert.equal(parsePresence(answer), null);
});

test('an available zero is retained as a real observation', () => {
  const empty = presence();
  empty.total = 0;
  empty.by_role = { validator: 0, candidate: 0, follower: 0 };
  empty.by_version = {};
  empty.by_region = Object.fromEntries(Object.keys(empty.by_region).map((key) => [key, 0]));
  assert.equal(parsePresence(empty).total, 0);
});

test('live observations expire at 180 seconds and allow at most 60 seconds of future skew', () => {
  const now = 1_791_446_400;
  assert.equal(parsePresence(presence(now), now).total, 6);
  assert.equal(parsePresence(presence(now - 179), now).total, 6);
  assert.equal(parsePresence(presence(now - 180), now), null);
  assert.equal(parsePresence(presence(now - 181), now), null);
  assert.equal(parsePresence(presence(now + 60), now).total, 6);
  assert.equal(parsePresence(presence(now + 61), now), null);
  for (const ttl_seconds of [undefined, null, 0, 179, 181, 600, '180']) {
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
  assert.equal(await readPresence({ call: async () => presence(Math.floor(Date.now() / 1000) - 180) }), null);
  assert.equal(await readPresence({ call: async () => presence(Math.floor(Date.now() / 1000) + 120) }), null);
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

test('home live panel renders Macs, role/version/continent counts, and the limited scope', async () => {
  const home = await homeView(context(presence()));
  const live = find(home, (el) => el.tagName === 'section' && text(el.children[0]) === 'Live network');
  const copy = text(live);
  assert.ok(live);
  for (const needle of ['6 Macs online now', 'what this node can see', 'refreshes every 10 seconds',
    'Validator 4', 'Candidate 0', 'Follower 2', '0.7.4 4', '0.7.3 2',
    'Asia 3', 'Europe 1', 'North America 1', 'South America 0', 'Africa 0', 'Oceania 0', 'Unknown 1',
    'Regions follow home relays', '180 seconds', 'separate from consensus',
  ]) assert.ok(copy.includes(needle), `${needle} is visible`);
  assert.ok(copy.indexOf('0.7.4') < copy.indexOf('0.7.3'), 'newer versions are listed first');
  assert.ok(!copy.includes('KR'), 'country sharing does not turn continent counts into precise locations');
  assert.ok(text(home).includes('Latest blocks'), 'chain data stays visible');
});

test('an unavailable or stale presence leaves the home working and never claims zero Macs', async () => {
  for (const answer of [new Error('method not found'), { ...presence(), available: false }, { schema: 1, total: 0 },
    presence(Math.floor(Date.now() / 1000) - 180), presence(Math.floor(Date.now() / 1000) + 120),
    { ...presence(), ttl_seconds: 600 },
  ]) {
    const home = await homeView(context(answer));
    const live = find(home, (el) => el.tagName === 'section' && text(el.children[0]) === 'Live network');
    assert.ok(text(live).includes('Unavailable'));
    assert.ok(text(live).includes('what this node can see'));
    assert.ok(text(live).includes('Settings'));
    assert.ok(!text(live).includes('0 Macs'));
    assert.ok(text(home).includes('Finalized height'));
  }
});

test('version labels are text and presence node metadata is not displayed', async () => {
  const snapshot = presence();
  snapshot.by_version = { '<img src=x onerror=alert(1)>': 6 };
  snapshot.nodes = [{ node_id: '22'.repeat(32), ip: '192.0.2.1', city: 'Seoul' }];
  const home = await homeView(context(snapshot));
  assert.ok(text(home).includes('<img src=x onerror=alert(1)>'));
  assert.equal(find(home, (el) => el.tagName === 'img'), null);
  assert.ok(!text(home).includes('192.0.2.1'));
  assert.ok(!text(home).includes('Seoul'));
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
