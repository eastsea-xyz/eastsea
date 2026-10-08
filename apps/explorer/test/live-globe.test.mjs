import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile, readdir } from 'node:fs/promises';
import { gzipSync } from 'node:zlib';
import { LAND_POINTS, COASTLINE_POINTS } from '../live-globe/land.js';
import { normalizePresence } from '../live-globe/data.js';

const source = new URL('../live-globe/', import.meta.url);
const site = new URL('../../../site/live-globe/', import.meta.url);
const wallet = new URL('../../../apps/wallet/Resources/LiveGlobe/live-globe/', import.meta.url);
const root = new URL('../../../', import.meta.url);

test('site, explorer and wallet deploy the identical self-contained globe without a build', async () => {
  const names = (await readdir(source)).sort();
  for (const destination of [site, wallet]) {
    assert.deepEqual((await readdir(destination)).sort(), names);
    for (const name of names) {
      const [a, b] = await Promise.all([readFile(new URL(name, source)), readFile(new URL(name, destination))]);
      assert.ok(a.equals(b), `${destination.pathname}${name}: run node scripts/sync-live-globe.mjs`);
    }
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
  assert.equal(fixture.quality_version, 1);
  assert.ok(!/founder/i.test(JSON.stringify(fixture)));
  assert.deepEqual(fixture.roles.validator, { count: 4 });
  assert.deepEqual(fixture.roles.wallet, { count: 3 });
  assert.deepEqual(fixture.reserve_keys, { standby: 3, seated: 0 });
  assert.deepEqual(fixture.regions.map(({ continent, country, count }) => ({ continent, ...(country && { country }), count })), [
    { continent: 'asia', country: 'KR', count: 3 }, { continent: 'asia', count: 1 },
  ]);
  assert.equal(fixture.regions.reduce((sum, region) => sum + region.quality.score_sum, 0), 162053);
  assert.deepEqual(fixture.recent_blocks, [], 'today snapshot never invents traffic');
  const brand = JSON.parse(await readFile(new URL('design/brand/tokens.json', root), 'utf8'));
  const css = await readFile(new URL('globe.css', source), 'utf8');
  for (const key of ['bg', 'accent', 'success']) assert.ok(css.includes(brand.color.dark[key].value));
});

test('the 24-Mac illustrative example is bundled only for explicit screenshot fixtures', async () => {
  const example = JSON.parse(await readFile(new URL('fixtures/presence-example.json', import.meta.url), 'utf8'));
  const clean = normalizePresence(example);
  assert.equal(clean.total, 24);
  assert.equal(new Set(clean.regions.map(region => region.continent)).size, 6);
  assert.ok(!(await readdir(source)).includes('presence-example.json'));
  const bundled = await readFile(new URL('apps/wallet/Resources/LiveGlobe/presence-example.json', root));
  assert.ok(bundled.equals(await readFile(new URL('fixtures/presence-example.json', import.meta.url))));
});

test('wallet entry CSP denies connections and includes only local styles and ES modules', async () => {
  const html = await readFile(new URL('apps/wallet/Resources/LiveGlobe/index.html', root), 'utf8');
  assert.ok(html.includes("default-src 'none'"));
  assert.ok(html.includes("connect-src 'none'"));
  assert.ok(html.includes("script-src 'self'"));
  assert.ok(html.includes("script-src-attr 'none'"));
  assert.ok(html.includes("style-src 'self'"));
  assert.ok(html.includes('type="module" src="./wallet-host.js"'));
  const references = [...html.matchAll(/(?:src|href)="([^"]+)"/g)].map(match => match[1]);
  assert.deepEqual(references, ['./live-globe/tokens.css', './live-globe/globe.css', './wallet.css', './wallet-host.js']);
  assert.ok(!/<script[^>]*>(?!\s*<\/script>)/.test(html));
  const entry = await readFile(new URL('apps/wallet/Resources/LiveGlobe/wallet-host.js', root), 'utf8');
  assert.ok(!/fetch\s*\(|XMLHttpRequest|messageHandlers|webkit\.|location\.|localStorage|sessionStorage|cookie/.test(entry));
});


test('both surfaces retain the bilingual same-rules FAQ disclosure', async () => {
  const html = await readFile(new URL('site/index.html', root), 'utf8');
  assert.ok(html.includes('창업자도 다른 사람과 같은 규칙으로 맥을 가동해야 보상을 받습니다.'));
  assert.ok(html.includes('the founder has to run Macs under the same rules as everyone else to receive any.'));
});

test('globe assets have no founder UI, discrete tier names or golden quality endpoints', async () => {
  for (const name of ['data.js', 'live-globe.js', 'globe.js', 'globe.css', 'fixture.json']) {
    const text = await readFile(new URL(name, source), 'utf8');
    assert.ok(!/founder|창업자|founderOperated|founder_operated/i.test(text), name);
    assert.ok(!/bronze|silver|platinum|legend-tier|tier-count/i.test(text), name);
  }
  const css = await readFile(new URL('globe.css', source), 'utf8');
  assert.ok(!css.includes('--lg-pulse'));
});
