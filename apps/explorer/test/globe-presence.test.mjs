// Privacy boundary and RPC request tests: injected data/fetch, no network.
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  CONTINENTS, normalizePresence, continentTotals, sessionJitter, requestPresence,
} from '../live-globe/data.js';

function presence(regions = [], extras = {}) {
  const total = regions.reduce((sum, region) => sum + region.count, 0);
  return {
    schema_version: 1, scope: 'node', total,
    roles: { validator: total, candidate: 0, follower: 0 },
    versions: { '0.7.4': total }, regions,
    ...extras,
  };
}

const response = (result) => ({ ok: true, json: async () => ({ jsonrpc: '2.0', id: 1, result }) });
const genericDataError = (error) => error.message === 'Invalid presence data.';
const genericRequestError = (error) => error.message === 'Live presence is unavailable.';

test('country thresholds 0, 1, 2 and 3 fold without losing any Macs', () => {
  for (const count of [0, 1, 2, 3]) {
    const model = normalizePresence(presence([{ continent: 'asia', country: 'KR', count }]));
    assert.equal(model.total, count);
    assert.deepEqual(model.regions, count === 0 ? [] : count < 3
      ? [{ continent: 'asia', count }]
      : [{ continent: 'asia', country: 'KR', count }]);
    assert.equal(model.regions.reduce((sum, region) => sum + region.count, 0), count);
    if (count < 3) assert.ok(!JSON.stringify(model).includes('KR'));
  }
});

test('duplicate buckets merge before k=3 and null countries fold into continents', () => {
  const input = presence([
    { continent: 'asia', country: 'KR', count: 1 },
    { continent: 'asia', country: 'KR', count: 2 },
    { continent: 'asia', country: 'JP', count: 1 },
    { continent: 'asia', country: 'JP', count: 1 },
    { continent: 'asia', count: 2 },
    { continent: 'asia', country: null, count: 1 },
    { continent: 'unknown', count: 1 },
  ]);
  const model = normalizePresence(input);
  assert.deepEqual(model.regions, [
    { continent: 'asia', count: 5 },
    { continent: 'asia', country: 'KR', count: 3 },
    { continent: 'unknown', count: 1 },
  ]);
  assert.equal(model.total, 9);
  assert.deepEqual(normalizePresence({ ...input, regions: [...input.regions].reverse() }), model);
  assert.deepEqual(normalizePresence(model), model);
  assert.ok(!JSON.stringify(model).includes('JP'));
});

test('the threshold never releases a small cross-continent country bucket', () => {
  const model = normalizePresence(presence([
    { continent: 'asia', country: 'KR', count: 2 },
    { continent: 'europe', country: 'KR', count: 1 },
  ]));
  assert.deepEqual(model.regions, [{ continent: 'asia', count: 2 }, { continent: 'europe', count: 1 }]);
});

test('roles, versions and regions each preserve the observed total', () => {
  const model = normalizePresence(presence([
    { continent: 'africa', count: 2 },
    { continent: 'europe', country: 'DE', count: 4 },
    { continent: 'oceania', count: 1 },
  ], {
    roles: { validator: 2, candidate: 4, follower: 1 },
    versions: { '0.7.4': 4, '0.7.3': 2, '0.7.5-rc.1+build.2': 1 },
  }));
  for (const parts of [Object.values(model.roles), Object.values(model.versions), model.regions.map((r) => r.count)]) {
    assert.equal(parts.reduce((sum, count) => sum + count, 0), model.total);
  }
  assert.deepEqual(continentTotals(model), [
    { continent: 'africa', count: 2 }, { continent: 'asia', count: 0 },
    { continent: 'europe', count: 4 }, { continent: 'north_america', count: 0 },
    { continent: 'south_america', count: 0 }, { continent: 'oceania', count: 1 },
    { continent: 'antarctica', count: 0 }, { continent: 'unknown', count: 0 },
  ]);
  assert.deepEqual(continentTotals(normalizePresence(presence())).map((r) => r.continent), CONTINENTS);
});

test('unknown, sensitive and coordinate fields never enter the output model', () => {
  const sensitive = {
    ip: '192.0.2.80', city: 'Seoul', coordinates: [37.5, 127],
    lat: 37.5, lon: 127, node_id: 'private-node', relay_url: 'https://private.invalid',
  };
  const model = normalizePresence(presence([
    { continent: 'asia', country: 'KR', count: 3, ...sensitive },
  ], {
    ...sensitive,
    roles: { validator: 3, candidate: 0, follower: 0, ...sensitive },
    recent_blocks: [{ height: 7, continent: 'unknown', country: 'KR', ...sensitive }],
  }));
  assert.deepEqual(model, {
    schema_version: 1, scope: 'node', total: 3,
    roles: { validator: 3, candidate: 0, follower: 0 }, versions: { '0.7.4': 3 },
    regions: [{ continent: 'asia', country: 'KR', count: 3 }],
    recent_blocks: [{ height: 7, continent: 'unknown' }],
  });
  const output = JSON.stringify(model);
  for (const key of Object.keys(sensitive)) assert.ok(!output.includes(key));
  for (const word of ['192.0.2.80', 'Seoul', 'private-node', 'private.invalid']) assert.ok(!output.includes(word));
  assert.deepEqual(continentTotals(model).flatMap(Object.keys), Array(8).fill(['continent', 'count']).flat());
});

test('malformed schema, codes, labels and totals are rejected with generic errors', () => {
  const base = presence([{ continent: 'asia', country: 'KR', count: 3 }]);
  const invalid = [
    null, [], 'private-node',
    { ...base, schema_version: 2 }, { ...base, scope: 'individual' },
    { ...base, total: 4 }, { ...base, total: -1 }, { ...base, total: '3' },
    { ...base, roles: { validator: 2, candidate: 0, follower: 0 } },
    { ...base, roles: { validator: 3, candidate: 0 } },
    { ...base, roles: { validator: 1.5, candidate: 1.5, follower: 0 } },
    { ...base, versions: { '0.7.4': 2 } },
    { ...base, versions: { '192.0.2.1-private-node': 3 } },
    { ...base, versions: { '0.7.4-private-node.invalid<': 3 } },
    { ...base, versions: { '0.7.4-01': 3 } },
    { ...base, versions: { '00.7.4': 3 } },
    { ...base, versions: { [`0.7.4-${'a'.repeat(65)}`]: 3 } },
    { ...base, regions: [{ continent: 'Seoul', count: 3 }] },
    ...['kr', 'KOR', 'XX', '12', ''].map((country) => ({ ...base, regions: [{ continent: 'asia', country, count: 3 }] })),
    { ...base, regions: [{ continent: 'asia', count: '3' }] },
    { ...base, regions: [{ continent: 'asia', count: NaN }] },
    { ...base, regions: [{ continent: 'asia', count: Infinity }] },
    { ...base, regions: [{ continent: 'asia', count: -3 }] },
    { ...base, regions: [{ continent: 'asia', count: Number.MAX_SAFE_INTEGER + 1 }] },
    { ...base, recent_blocks: null },
    { ...base, recent_blocks: [{ height: -1, continent: 'asia' }] },
    { ...base, recent_blocks: [{ height: 1, continent: 'private-node' }] },
  ];
  for (const input of invalid) assert.throws(() => normalizePresence(input), genericDataError);
});

test('collection caps and unsafe cumulative counts are rejected', () => {
  assert.throws(() => normalizePresence(presence(Array.from({ length: 1025 }, () => ({ continent: 'asia', count: 0 })))), genericDataError);
  assert.throws(() => normalizePresence(presence([], {
    versions: Object.fromEntries(Array.from({ length: 129 }, (_, i) => [`0.7.${i}`, 0])),
  })), genericDataError);
  assert.throws(() => normalizePresence(presence([], {
    recent_blocks: Array.from({ length: 9 }, (_, height) => ({ height, continent: 'asia' })),
  })), genericDataError);
  const size = Number.MAX_SAFE_INTEGER;
  assert.throws(() => normalizePresence(presence([{ continent: 'asia', count: size }, { continent: 'asia', count: 1 }])), genericDataError);
  assert.throws(() => normalizePresence(presence([{ continent: 'asia', count: size }], {
    roles: { validator: size, candidate: 1, follower: 0 },
  })), genericDataError);
});

test('prototype tricks, inherited fields and getters cannot populate the model', () => {
  const base = presence([{ continent: 'asia', count: 3 }]);
  const inherited = Object.create(base);
  const injected = JSON.parse('{"__proto__":{"node_id":"private-node"}}');
  const getter = { ...base };
  Object.defineProperty(getter, 'regions', { get() { throw new Error('private-node'); } });
  const regionGetter = { continent: 'asia', count: 3 };
  Object.defineProperty(regionGetter, 'country', { get() { throw new Error('Seoul'); } });
  const arrayHole = [ , ];
  for (const input of [
    inherited, { ...base, ...injected }, { ...base, constructor: 'private-node' },
    { ...base, roles: { ...base.roles, prototype: 'private-node' } },
    { ...base, regions: [{ ...base.regions[0], __proto__: { country: 'KR' } }] },
    { ...base, regions: [regionGetter] }, { ...base, regions: arrayHole }, getter,
    new Proxy(base, { getPrototypeOf() { throw new Error('private-node'); } }),
  ]) assert.throws(() => normalizePresence(input), genericDataError);
  assert.deepEqual(normalizePresence(Object.assign(Object.create(null), base)), normalizePresence(base));
});

test('jitter is deterministic for a page session and independent of data order/counts', () => {
  const seed = 'ephemeral-session-a';
  const input = presence([{ continent: 'europe', count: 2 }, { continent: 'asia', count: 3 }]);
  const before = continentTotals(normalizePresence(input)).map(({ continent }) => sessionJitter(continent, seed));
  const after = continentTotals(normalizePresence(presence([
    { continent: 'asia', count: 300 }, { continent: 'europe', count: 20 },
  ]))).map(({ continent }) => sessionJitter(continent, seed));
  assert.deepEqual(before, after);
  assert.notDeepEqual(sessionJitter('asia', seed), sessionJitter('asia', 'ephemeral-session-b'));
  assert.notDeepEqual(sessionJitter('asia', seed), sessionJitter('europe', seed));
  assert.deepEqual(sessionJitter('asia', 1234), sessionJitter('asia', 1234));
  for (const code of CONTINENTS) {
    for (let i = 0; i < 100; i++) {
      const offsets = sessionJitter(code, `session-${i}`);
      assert.ok(Array.isArray(offsets));
      assert.equal(offsets.length, 2);
      assert.ok(offsets.every((value) => Number.isFinite(value) && Math.abs(value) <= 0.035));
    }
  }
  assert.throws(() => sessionJitter('Seoul', seed), genericDataError);
  assert.throws(() => sessionJitter('asia', { node_id: 'private-node' }), genericDataError);
  assert.throws(() => sessionJitter('asia', Infinity), genericDataError);
});

test('presence requests whitelist the read method without location/session metadata', async () => {
  const controller = new AbortController();
  let seen;
  const expected = normalizePresence(presence([{ continent: 'asia', country: 'KR', count: 2 }]));
  const model = await requestPresence('https://rpc.example.invalid', {
    signal: controller.signal,
    fetch: async (url, options) => {
      seen = { url, options };
      return response(presence([{ continent: 'asia', country: 'KR', count: 2, coordinates: [37.5, 127] }]));
    },
  });
  assert.deepEqual(model, expected);
  assert.equal(seen.url, 'https://rpc.example.invalid');
  assert.deepEqual(JSON.parse(seen.options.body), { jsonrpc: '2.0', id: 1, method: 'aether_presence', params: [] });
  assert.deepEqual(Object.keys(seen.options).sort(), [
    'method', 'headers', 'body', 'credentials', 'cache', 'referrerPolicy', 'redirect', 'signal',
  ].sort());
  assert.deepEqual(seen.options.headers, { 'Content-Type': 'application/json' });
  assert.equal(seen.options.method, 'POST');
  assert.equal(seen.options.credentials, 'omit');
  assert.equal(seen.options.cache, 'no-store');
  assert.equal(seen.options.referrerPolicy, 'no-referrer');
  assert.equal(seen.options.redirect, 'error');
  assert.equal(seen.options.signal, controller.signal);
  for (const field of ['country', 'continent', 'coordinates', 'node_id', 'seed', 'session']) {
    assert.ok(!seen.options.body.includes(field));
  }
});

test('bad envelopes, RPC errors and transport failures never echo their contents', async () => {
  const valid = presence([{ continent: 'unknown', count: 1 }]);
  const envelopes = [
    null, [], { result: valid }, { jsonrpc: '1.0', id: 1, result: valid },
    { jsonrpc: '2.0', id: '1', result: valid }, { jsonrpc: '2.0', id: 2, result: valid },
    { jsonrpc: '2.0', id: 1 },
    { jsonrpc: '2.0', id: 1, result: valid, error: null },
    { jsonrpc: '2.0', id: 1, error: { code: -32601, message: '192.0.2.80 private-node Seoul' } },
    { jsonrpc: '2.0', id: 1, result: { ...valid, scope: 'private-node' } },
  ];
  for (const envelope of envelopes) {
    await assert.rejects(() => requestPresence('https://rpc.example.invalid', {
      fetch: async () => ({ ok: true, json: async () => envelope }),
    }), genericRequestError);
  }
  for (const fetch of [
    async () => { throw new Error('192.0.2.80 private-node Seoul'); },
    async () => ({ ok: false, status: 503, json: async () => { throw new Error('private-node'); } }),
    async () => ({ ok: true, json: async () => { throw new Error('private-node'); } }),
  ]) await assert.rejects(() => requestPresence('https://rpc.example.invalid', { fetch }), genericRequestError);
  for (const endpoint of ['', 'file:///private-node', 'https://user:password@rpc.example.invalid', 'https://rpc.example.invalid#private-node']) {
    await assert.rejects(() => requestPresence(endpoint, {
      fetch: async () => assert.fail('an invalid endpoint must never make a request'),
    }), genericRequestError);
  }
});
