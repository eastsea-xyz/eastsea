import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile, readdir } from 'node:fs/promises';
import { gzipSync } from 'node:zlib';
import { LAND_POINTS, COASTLINE_POINTS } from '../live-globe/land.js';
import { normalizePresence } from '../live-globe/data.js';

const source = new URL('../live-globe/', import.meta.url);
const site = new URL('../../../site/live-globe/', import.meta.url);
const root = new URL('../../../', import.meta.url);

test('site and explorer deploy the identical self-contained globe without a build', async () => {
  const names = (await readdir(source)).sort();
  assert.deepEqual((await readdir(site)).sort(), names);
  for (const name of names) {
    const [a, b] = await Promise.all([readFile(new URL(name, source)), readFile(new URL(name, site))]);
    assert.ok(a.equals(b), `${name}: run node scripts/sync-live-globe.mjs`);
  }
  assert.ok((await readFile(new URL('tokens.css', source))).equals(await readFile(new URL('site/tokens.css', root))));
});

test('globe JavaScript stays below the 300 KB gzipped budget including local land data', async () => {
  const names = (await readdir(source)).filter(name => name.endsWith('.js'));
  let bytes = 0;
  for (const name of names) bytes += gzipSync(await readFile(new URL(name, source))).length;
  assert.ok(bytes < 300_000, `${bytes} gzipped bytes exceeds the budget`);
});

test('bundled land artwork is finite unit-sphere geometry, not live geographic records', () => {
  assert.equal(LAND_POINTS.length % 3, 0);
  assert.ok(LAND_POINTS.length / 3 > 10_000);
  assert.equal(COASTLINE_POINTS.length % 6, 0);
  assert.ok(COASTLINE_POINTS.length / 6 > 5_000);
  for (const artwork of [LAND_POINTS, COASTLINE_POINTS]) {
    for (let i = 0; i < artwork.length; i += 3) {
      const point = artwork.slice(i, i + 3);
      assert.ok(point.every(Number.isFinite));
      assert.ok(Math.abs(Math.hypot(...point) - 1) < .00002);
    }
  }
  for (let i = 0; i < COASTLINE_POINTS.length; i += 6) {
    const a = COASTLINE_POINTS.slice(i, i + 3), b = COASTLINE_POINTS.slice(i + 3, i + 6);
    assert.ok(Math.acos(Math.min(1, a.reduce((sum, part, index) => sum + part * b[index], 0))) < 2.1 * Math.PI / 180);
  }
});

test('today’s actual snapshot preserves Mac, overlapping role and standby-key counts', async () => {
  const fixture = JSON.parse(await readFile(new URL('fixture.json', source), 'utf8'));
  assert.deepEqual(normalizePresence(fixture), normalizePresence(normalizePresence(fixture)));
  assert.equal(fixture.total, 4);
  assert.equal(fixture.founder_operated, 4);
  assert.deepEqual(fixture.roles.validator, { count: 4, founder_operated: 4 });
  assert.deepEqual(fixture.roles.wallet, { count: 3, founder_operated: 2 });
  assert.deepEqual(fixture.reserve_keys, { standby: 3, seated: 0 });
  assert.deepEqual(fixture.regions, [
    { continent: 'asia', country: 'KR', count: 3, founder_operated: 3 },
    { continent: 'asia', count: 1, founder_operated: 1 },
  ]);
  assert.deepEqual(fixture.recent_blocks, [], 'today snapshot never invents traffic');
  const brand = JSON.parse(await readFile(new URL('design/brand/tokens.json', root), 'utf8'));
  const css = await readFile(new URL('globe.css', source), 'utf8');
  for (const key of ['bg', 'accent', 'gold']) assert.ok(css.includes(brand.color.dark[key].value));
});

test('the old 24-Mac illustrative example is retained only in the test tree', async () => {
  const example = JSON.parse(await readFile(new URL('fixtures/presence-example.json', import.meta.url), 'utf8'));
  const clean = normalizePresence(example);
  assert.equal(clean.total, 24);
  assert.equal(new Set(clean.regions.map(region => region.continent)).size, 6);
  assert.ok(!(await readdir(source)).includes('presence-example.json'));
});
