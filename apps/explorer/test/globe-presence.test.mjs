// Privacy boundary and RPC request tests: injected data/fetch, no network.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { CONTINENTS, normalizePresence, continentTotals, sessionJitter, requestPresence } from '../live-globe/data.js';
import { summarizeQuality, qualityMean, qualityScore } from '../live-globe/quality.js';
import { COUNTRY_CENTROIDS } from '../live-globe/countries.js';

const zeroQuality = count => ({ score_sum: 0, histogram: [count, ...Array(19).fill(0)] });
const region = (continent, count, extra = {}) => ({ continent, count, quality: zeroQuality(count), ...extra });
function presence(regions = [], extras = {}) {
  regions = regions.map(r => region(r.continent, r.count, r));
  const total = regions.reduce((sum, r) => sum + r.count, 0);
  return {
    schema_version: 3, scope: 'node', quality_version: 1, total,
    roles: { validator: { count: total }, wallet: { count: 0 }, candidate: { count: 0 }, follower: { count: 0 } },
    versions: { '0.7.4': total }, reserve_keys: { standby: 0, seated: 0 }, regions, ...extras,
  };
}
const response = result => ({ ok: true, json: async () => ({ jsonrpc: '2.0', id: 1, result }) });
const genericDataError = error => error.message === 'Invalid presence data.';
const genericRequestError = error => error.message === 'Live presence is unavailable.';

test('today snapshot has four Macs, separate validators, and scores from the supplied streaks', async () => {
  const fixture = JSON.parse(await readFile(new URL('../live-globe/fixture.json', import.meta.url), 'utf8'));
  const model = normalizePresence(fixture);
  assert.equal(model.total, 4);
  assert.deepEqual(model.roles.validator, { count: 4 });
  assert.deepEqual(model.roles.wallet, { count: 3 });
  assert.equal(Object.values(model.roles).reduce((sum, role) => sum + role.count, 0), 7);
  assert.deepEqual(model.reserve_keys, { standby: 3, seated: 0 });
  const scores = [152, 24, 10, 9].map(streakHours => qualityScore({ streakHours }));
  assert.deepEqual(model.regions, [
    region('asia', 1, { quality: summarizeQuality(scores.slice(3)) }),
    region('asia', 3, { country: 'KR', quality: summarizeQuality(scores.slice(0, 3)) }),
  ]);
  assert.deepEqual(continentTotals(model).find(r => r.continent === 'asia'), region('asia', 4, { quality: summarizeQuality(scores) }));
  assert.deepEqual(model.recent_blocks, []);
  assert.ok(!/founder/i.test(JSON.stringify(model)));
});

test('the former 24-Mac geography is an illustrative test fixture with aggregate quality only', async () => {
  const fixture = JSON.parse(await readFile(new URL('./fixtures/presence-example.json', import.meta.url), 'utf8'));
  const model = normalizePresence(fixture);
  assert.equal(model.total, 24);
  assert.deepEqual(continentTotals(model).map(({ continent, count }) => ({ continent, count })), [
    { continent: 'africa', count: 0 }, { continent: 'asia', count: 9 }, { continent: 'europe', count: 6 },
    { continent: 'north_america', count: 6 }, { continent: 'south_america', count: 1 },
    { continent: 'oceania', count: 1 }, { continent: 'antarctica', count: 0 }, { continent: 'unknown', count: 1 },
  ]);
  assert.ok(!/founder/i.test(JSON.stringify(fixture)));
});

test('country thresholds 0, 1, 2 and 3 fold counts and quality together', () => {
  for (const count of [0, 1, 2, 3]) {
    const model = normalizePresence(presence([region('asia', count, { country: 'KR' })]));
    assert.deepEqual(model.regions, count === 0 ? [] : [region('asia', count, count < 3 ? {} : { country: 'KR' })]);
    assert.equal(model.total, count);
    if (count < 3) assert.ok(!JSON.stringify(model).includes('KR'));
  }
});

test('duplicate country buckets merge before k=3, preserving the full quality distribution', () => {
  const input = presence([
    region('asia', 1, { country: 'KR', quality: summarizeQuality([.2]) }),
    region('asia', 2, { country: 'KR', quality: summarizeQuality([.4, .7]) }),
    region('asia', 2, { country: 'JP', quality: summarizeQuality([.1, .8]) }),
    region('asia', 2, { quality: summarizeQuality([.3, .6]) }),
    region('asia', 1, { country: null, quality: summarizeQuality([.5]) }),
    region('unknown', 1),
  ]);
  const model = normalizePresence(input);
  assert.deepEqual(model.regions, [
    region('asia', 5, { quality: summarizeQuality([.1, .8, .3, .6, .5]) }),
    region('asia', 3, { country: 'KR', quality: summarizeQuality([.2, .4, .7]) }),
    region('unknown', 1),
  ]);
  assert.equal(model.total, 9);
  assert.ok(!JSON.stringify(model).includes('JP'));
  assert.deepEqual(normalizePresence({ ...input, regions: [...input.regions].reverse() }), model);
  assert.deepEqual(normalizePresence(model), model);
  assert.ok(Math.abs(qualityMean(continentTotals(model).find(r => r.continent === 'asia').quality, 8) - .45) < 1e-12);
});

test('countries never combine across continents to pass k=3', () => {
  const model = normalizePresence(presence([region('asia', 2, { country: 'KR' }), region('europe', 1, { country: 'KR' })]));
  assert.deepEqual(model.regions, [region('asia', 2), region('europe', 1)]);
});

test('roles overlap without inflating Mac totals and reserve keys stay separate', () => {
  const model = normalizePresence(presence([region('africa', 2), region('europe', 4, { country: 'DE' }), region('oceania', 1)], {
    roles: { validator: { count: 2 }, wallet: { count: 5 }, candidate: { count: 4 }, follower: { count: 1 } },
    versions: { '0.7.4': 4, '0.7.3': 2, '0.7.5-rc.1+build.2': 1 },
    reserve_keys: { standby: 3, seated: 2 },
  }));
  assert.equal(model.total, 7);
  assert.equal(Object.values(model.roles).reduce((sum, role) => sum + role.count, 0), 12);
  assert.equal(Object.values(model.versions).reduce((sum, count) => sum + count, 0), 7);
  assert.equal(continentTotals(model).reduce((sum, r) => sum + r.count, 0), 7);
  assert.deepEqual(continentTotals(presence()).map(r => r.continent), CONTINENTS);
  for (const role of Object.keys(model.roles)) {
    const roles = { ...model.roles }; delete roles[role];
    assert.throws(() => normalizePresence({ ...model, roles }), genericDataError);
    assert.throws(() => normalizePresence({ ...model, roles: { ...model.roles, [role]: { count: 8 } } }), genericDataError);
  }
  assert.throws(() => normalizePresence({ ...model, reserve_keys: { standby: Number.MAX_SAFE_INTEGER, seated: 1 } }), genericDataError);
  for (const key of ['standby', 'seated']) for (const count of [undefined, null, true, '3', NaN, Infinity, -1, .5]) {
    assert.throws(() => normalizePresence({ ...model, reserve_keys: { ...model.reserve_keys, [key]: count } }), genericDataError);
  }
});

test('schema v3 requires valid continuous quality aggregates, never silently inventing them', () => {
  const base = presence([region('asia', 3)]);
  for (const quality of [undefined, null, [], 1,
    { score_sum: -1, histogram: zeroQuality(3).histogram },
    { score_sum: .5, histogram: zeroQuality(3).histogram },
    { score_sum: 3_000_001, histogram: zeroQuality(3).histogram },
    { score_sum: 0, histogram: [3] },
    { score_sum: 0, histogram: zeroQuality(2).histogram },
    { score_sum: 0, histogram: [1.5, 1.5, ...Array(18).fill(0)] },
    { score_sum: 0, histogram: [0, 3, ...Array(18).fill(0)] },
  ]) assert.throws(() => normalizePresence({ ...base, regions: [{ ...base.regions[0], quality }] }), genericDataError);
  for (const version of [undefined, 0, 2, '1']) assert.throws(() => normalizePresence({ ...base, quality_version: version }), genericDataError);
  for (const version of [undefined, 1, 2]) assert.throws(() => normalizePresence({ ...base, schema_version: version }), genericDataError);
});

test('founder, self-asserted quality, identifiers and locations never enter the public model', () => {
  const sensitive = {
    founder_operated: 3, founder_operator_id: 'founder', operator_id: 'private-operator',
    quality_score: 1, tier: 'best', ip: '192.0.2.80', city: 'Seoul', coordinates: [37.5, 127],
    lat: 37.5, lon: 127, node_id: 'private-node', relay_url: 'https://private.invalid',
  };
  const input = presence([region('asia', 3, { country: 'KR', ...sensitive, quality: { ...zeroQuality(3), ...sensitive } })], {
    ...sensitive, roles: { validator: { count: 3, ...sensitive }, wallet: { count: 0 }, candidate: { count: 0 }, follower: { count: 0 } },
    reserve_keys: { standby: 3, seated: 1, ...sensitive },
    recent_blocks: [{ height: 7, continent: 'unknown', country: 'KR', ...sensitive }],
  });
  const model = normalizePresence(input);
  assert.deepEqual(model, { ...presence([region('asia', 3, { country: 'KR' })]), reserve_keys: { standby: 3, seated: 1 }, recent_blocks: [{ height: 7, continent: 'unknown' }] });
  for (const key of Object.keys(sensitive)) assert.ok(!JSON.stringify(model).includes(key), key);
  assert.deepEqual(continentTotals(model).flatMap(Object.keys), Array(8).fill(['continent', 'count', 'quality']).flat());
});

test('bundled country centroids cover all current ISO codes without location data requests', () => {
  assert.equal(Object.keys(COUNTRY_CENTROIDS).length, 249);
  for (const [country, point] of Object.entries(COUNTRY_CENTROIDS)) {
    assert.deepEqual(normalizePresence(presence([region('asia', 3, { country })])).regions[0].country, country);
    assert.ok(point.every(Number.isFinite));
    assert.ok(Math.abs(point[0]) <= 180 && Math.abs(point[1]) <= 90);
    assert.ok(Object.isFrozen(point));
    const jitter = sessionJitter(`asia:${country}`, 'session');
    assert.ok(jitter.every(n => Math.abs(n) <= .035));
  }
  assert.notDeepEqual(sessionJitter('asia:KR', 'session'), sessionJitter('asia', 'session'));
});

test('malformed schema, codes, labels and totals are rejected with generic errors', () => {
  const base = presence([{ continent: 'asia', country: 'KR', count: 3 }]);
  const invalid = [
    null, [], 'private-node',
    { ...base, schema_version: 1 }, { ...base, scope: 'individual' },
    { ...base, total: 4 }, { ...base, total: -1 }, { ...base, total: '3' },
    { ...base, roles: { ...base.roles, validator: { count: 4} } },
    { ...base, roles: { validator: base.roles.validator, candidate: base.roles.candidate } },
    { ...base, roles: { ...base.roles, validator: { count: 1.5} } },
    { ...base, roles: { validator: 3, wallet: 0, candidate: 0, follower: 0 } },
    { ...base, versions: { '0.7.4': 2 } },
    { ...base, versions: { '192.0.2.1-private-node': 3 } },
    { ...base, versions: { '0.7.4-private-node.invalid<': 3 } },
    { ...base, versions: { '0.7.4-01': 3 } },
    { ...base, versions: { '00.7.4': 3 } },
    { ...base, versions: { [`0.7.4-${'a'.repeat(65)}`]: 3 } },
    { ...base, regions: [{ continent: 'Seoul', count: 3}] },
    ...['kr', 'KOR', 'XX', '12', ''].map((country) => ({ ...base, regions: [{ continent: 'asia', country, count: 3}] })),
    { ...base, regions: [{ continent: 'asia', count: '3'}] },
    { ...base, regions: [{ continent: 'asia', count: NaN}] },
    { ...base, regions: [{ continent: 'asia', count: Infinity}] },
    { ...base, regions: [{ continent: 'asia', count: -3}] },
    { ...base, regions: [{ continent: 'asia', count: Number.MAX_SAFE_INTEGER + 1}] },
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
    versions: { '0.7.4': size, '0.7.3': 1 },
  })), genericDataError);
  // Summing overlapping roles would overflow, but each role fits the Mac total.
  const overlapping = normalizePresence(presence([{ continent: 'asia', count: size }], {
    roles: Object.fromEntries(['validator', 'wallet', 'candidate', 'follower'].map((role) => [
      role, { count: size},
    ])),
  }));
  assert.equal(overlapping.total, size);
  assert.equal(overlapping.roles.wallet.count, size);
});

test('prototype tricks, inherited fields and getters cannot populate the model', () => {
  const base = presence([{ continent: 'asia', count: 3 }]);
  const inherited = Object.create(base);
  const injected = JSON.parse('{"__proto__":{"node_id":"private-node"}}');
  const getter = { ...base };
  Object.defineProperty(getter, 'regions', { get() { throw new Error('private-node'); } });
  const regionGetter = { continent: 'asia', count: 3};
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

test('nested quality, role and reserve fields reject getters without reading them', () => {
  const base = presence([{ continent: 'asia', count: 3 }]);
  let calls = 0;
  const getter = () => { calls++; throw new Error('private-node'); };
  const summary = { ...base.regions[0].quality };
  Object.defineProperty(summary, 'score_sum', { get: getter });
  const histogram = [...base.regions[0].quality.histogram];
  Object.defineProperty(histogram, '0', { get: getter });
  const role = { ...base.roles.validator };
  Object.defineProperty(role, 'count', { get: getter });
  const reserve = { ...base.reserve_keys };
  Object.defineProperty(reserve, 'standby', { get: getter });
  for (const input of [
    { ...base, regions: [{ ...base.regions[0], quality: summary }] },
    { ...base, regions: [{ ...base.regions[0], quality: { ...base.regions[0].quality, histogram } }] },
    { ...base, roles: { ...base.roles, validator: role } },
    { ...base, reserve_keys: reserve },
    { ...base, regions: [{ ...base.regions[0], quality: Object.create(base.regions[0].quality) }] },
    { ...base, roles: { ...base.roles, validator: Object.create(base.roles.validator) } },
    { ...base, reserve_keys: Object.create(base.reserve_keys) },
  ]) assert.throws(() => normalizePresence(input), genericDataError);
  assert.equal(calls, 0);
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
    { jsonrpc: '2.0', id: 1, result: { ...valid, schema_version: 1 } },
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
