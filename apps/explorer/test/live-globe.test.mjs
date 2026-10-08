import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile, readdir } from 'node:fs/promises';
import { gzipSync } from 'node:zlib';
import { LAND_POINTS } from '../live-globe/land.js';
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
  assert.ok(LAND_POINTS.length / 3 > 2000);
  for (let i = 0; i < LAND_POINTS.length; i += 3) {
    const point = LAND_POINTS.slice(i, i + 3);
    assert.ok(point.every(Number.isFinite));
    assert.ok(Math.abs(Math.hypot(...point) - 1) < .00002);
  }
});

test('illustrative fixture conforms to the exact public presence contract', async () => {
  const fixture = JSON.parse(await readFile(new URL('fixture.json', source), 'utf8'));
  assert.deepEqual(normalizePresence(fixture), normalizePresence(normalizePresence(fixture)));
  assert.equal(fixture.total, 24);
  const brand = JSON.parse(await readFile(new URL('design/brand/tokens.json', root), 'utf8'));
  const css = await readFile(new URL('globe.css', source), 'utf8');
  for (const key of ['bg', 'accent', 'gold']) assert.ok(css.includes(brand.color.dark[key].value));
});
