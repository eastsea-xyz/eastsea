#!/usr/bin/env node
// Real-browser offline smoke. Uses existing Playwright tooling, no app deps.
// PLAYWRIGHT_MODULE=/absolute/path/to/playwright node scripts/test-live-globe.mjs
import assert from 'node:assert/strict';
import { readFile, mkdir, writeFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { resolve, extname, sep } from 'node:path';
import { createRequire } from 'node:module';
import { CONTINENTS, GEOGRAPHIES, continentTotals, normalizePresence, regionKey } from '../apps/explorer/live-globe/data.js';
import { qualityMean, summarizeQuality } from '../apps/explorer/live-globe/quality.js';
import { SUBREGION_CODES } from '../apps/explorer/live-globe/subregions.js';

const root = resolve(fileURLToPath(new URL('../', import.meta.url)));
process.env.TMPDIR = resolve(root, 'tmp');
await mkdir(process.env.TMPDIR, { recursive: true });
const out = resolve(root, process.env.GLOBE_SCREENSHOTS || 'tmp/live-globe');
await mkdir(out, { recursive: true });
const { chromium } = createRequire(import.meta.url)(process.env.PLAYWRIGHT_MODULE || 'playwright');
const fixture = JSON.parse(await readFile(resolve(root, 'apps/explorer/live-globe/fixture.json'), 'utf8'));
const example = JSON.parse(await readFile(resolve(root, 'apps/explorer/test/fixtures/presence-example.json'), 'utf8'));
const mime = { '.html': 'text/html', '.js': 'text/javascript', '.mjs': 'text/javascript', '.css': 'text/css', '.json': 'application/json', '.svg': 'image/svg+xml', '.woff2': 'font/woff2', '.png': 'image/png', '.webp': 'image/webp' };
// Fulfill local assets directly so this works in sandboxes that forbid listen().
// No server, outbound request, or port is needed for the browser checks.
const origin = 'http://globe.test.invalid';
const browser = await chromium.launch({ headless: true, channel: process.env.GLOBE_BROWSER_CHANNEL || 'chrome' });
const report = { screenshots: [], checks: [], frameRate: null, environment: `${process.platform}/${process.arch}` };

async function context(options = {}, envelope = () => ({ jsonrpc: '2.0', id: 1, result: fixture })) {
  const ctx = await browser.newContext({ viewport: { width: 1280, height: 900 }, ...options });
  const errors = [];
  ctx.on('page', page => page.on('pageerror', error => errors.push(error.message)));
  const requests = [];
  await ctx.route('**/*', async route => {
    const request = route.request();
    const url = request.url();
    if (url.startsWith(origin + '/')) {
      const path = decodeURIComponent(new URL(url).pathname);
      const file = resolve(root, '.' + (path.endsWith('/') ? path + 'index.html' : path));
      if (!file.startsWith(root + sep)) return route.fulfill({ status: 403, body: '' });
      try { return await route.fulfill({ contentType: mime[extname(file)] || 'application/octet-stream', body: await readFile(file) }); }
      catch { return route.fulfill({ status: 404, body: '' }); }
    }
    assert.equal(url, 'https://rpc.eastsea.xyz/', 'no external scripts/maps/trackers/loopback');
    assert.equal(request.method(), 'POST');
    const body = request.postDataJSON();
    assert.deepEqual(body, { jsonrpc: '2.0', id: 1, method: 'aether_presence', params: [] });
    requests.push({ time: Date.now(), body });
    await route.fulfill({ contentType: 'application/json', body: JSON.stringify(envelope()) });
  });
  return { ctx, errors, requests };
}

async function verifyPage(page) {
  await page.locator('.lg-total').filter({ hasText: /^4$/ }).waitFor();
  await page.evaluate(() => document.fonts.ready);
  assert.deepEqual(await page.locator('.lg-region').evaluateAll(rows => rows.map(row => row.dataset.continent).sort()),
    [...GEOGRAPHIES].sort(), 'all supported subregion/continent rows are present');
  assert.deepEqual(await page.locator('.lg-region:visible').evaluateAll(rows => rows.map(row => row.dataset.continent).sort()),
    [...CONTINENTS].sort(), 'the legacy fixture shows broad regions and hides unreported subregions');
  assert.equal(await page.locator('.lg-country').count(), 1);
  assert.equal(await page.locator('.lg-marker[data-region="asia:KR"]:visible').count(), 1, 'opening view shows Korea’s three-Mac group');
  assert.ok((await page.locator('.lg-role-summary').innerText()).includes('4'));
  await verifyRepresentation(page, fixture);
  assert.equal(await page.locator('.lg-quality-gradient').count(), 1);
  assert.equal(await page.locator('.lg-quality-labels span').count(), 2);
  assert.ok(!/founder|창업자|founder[_-]?operated|tier-count/i.test(await page.locator('.live-globe').innerHTML()), 'no founder/tier fields in the globe DOM');
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false, 'mobile overflow');
  const identifyingText = /(?:\d{1,3}\.){3}\d{1,3}|latitude|longitude|node[_ -]?id|peer[_ -]?id/i;
  assert.equal(identifyingText.test(await page.locator('.live-globe').innerText()), false, 'the globe contains no precise/identifying location text');
  // The landing page also contains valid release strings such as 0.7.3.2.
  // The isolated explorer presence page must remain identifier-free throughout.
  if (new URL(page.url()).pathname.startsWith('/apps/explorer/')) {
    assert.equal(identifyingText.test(await page.locator('body').innerText()), false, 'the isolated presence page contains no identifiers');
  }
}

// Every populated region has a pulse or an ordinary list entry. Continent
// counts include their countries; these markers are deliberately hierarchical.
async function verifyRepresentation(page, snapshot) {
  const model = normalizePresence(snapshot);
  const byContinent = new Map(continentTotals(model).map(region => [region.continent, region]));
  const totals = await page.locator('.lg-region').evaluateAll(elements => elements.map(row => Number(row.dataset.count)));
  assert.equal(totals.reduce((sum, count) => sum + count, 0), model.total);
  const shown = await page.locator('.live-globe').evaluate(root => {
    const markers = Object.fromEntries([...root.querySelectorAll('.lg-marker')].map(marker => [marker.dataset.region, {
      count: Number(marker.dataset.count), quality: Number(marker.dataset.quality), visible: !marker.hidden,
    }]));
    const rows = Object.fromEntries([...root.querySelectorAll('.lg-region, .lg-country')].map(row => [row.dataset.region || row.dataset.continent, {
      count: Number(row.dataset.count), visible: !row.hidden,
    }]));
    return { markers, rows };
  });
  let located = 0;
  for (const region of model.regions) {
    const key = regionKey(region), row = shown.rows[key], marker = shown.markers[key];
    assert.ok(row, `${key}: a populated region has a list entry`);
    if (region.country) assert.equal(row.count, region.count);
    if (marker) {
      const expected = region.country ? region : byContinent.get(region.continent);
      assert.equal(marker.count, expected.count);
      assert.equal(marker.quality, qualityMean(expected.quality, expected.count));
      located += region.count;
      if (!marker.visible) assert.ok(row.visible && row.count > 0, `${key}: far-side counts remain in the list`);
    } else assert.equal(region.continent, 'unknown');
  }
  assert.equal(located, model.total - model.regions.filter(r => r.continent === 'unknown' && !r.country).reduce((sum, r) => sum + r.count, 0));
}

try {
  for (const theme of ['light', 'dark']) {
    for (const surface of ['explorer', 'site']) {
      const { ctx, errors, requests } = await context({ colorScheme: theme, locale: surface === 'site' ? 'ko-KR' : 'en-US' });
      const page = await ctx.newPage();
      const url = surface === 'explorer' ? '/apps/explorer/?globe=fixture#/network' : '/site/?globe=fixture#live-network';
      await page.goto(origin + url);
      await page.locator('.live-globe').scrollIntoViewIfNeeded();
      await verifyPage(page);
      assert.equal(await page.locator('canvas[data-renderer="webgl"]').count(), 1, 'normal motion uses WebGL');
      const target = surface === 'site' ? page.locator('#live-network') : page.locator('body');
      const name = `${surface}-${theme}.png`;
      await target.screenshot({ path: resolve(out, name), animations: 'disabled' });
      report.screenshots.push(name);
      assert.deepEqual(errors, [], 'no page errors');
      assert.equal(requests.length, 0, 'fixture performs no public RPC');
      await ctx.close();
    }
  }
  for (const theme of ['light', 'dark']) {
    const { ctx, errors } = await context({ viewport: { width: 390, height: 844 }, colorScheme: theme, isMobile: true, hasTouch: true });
    const page = await ctx.newPage();
    await page.goto(origin + '/apps/explorer/?globe=fixture#/network');
    await verifyPage(page);
    await page.screenshot({ path: resolve(out, `explorer-mobile-${theme}.png`), fullPage: true });
    report.screenshots.push(`explorer-mobile-${theme}.png`);
    // Playwright waits for a stationary target before tapping. Pause through
    // the real control so idle rotation cannot make this touch check flaky.
    await page.getByRole('button', { name: 'Pause globe', exact: true }).click();
    await page.locator('.lg-marker[data-region="asia"]').tap();
    assert.equal(await page.locator('.lg-region[data-continent="asia"]').getAttribute('data-active'), 'true', 'mobile pulse tap selects list row');
    await page.locator('.lg-region[data-continent="asia"] > .lg-region-value').tap();
    assert.equal(await page.locator('.lg-marker[data-region="asia"]').getAttribute('data-active'), 'true', 'mobile count-cell tap selects pulse');
    assert.deepEqual(errors, []);
    await ctx.close();
    const mobileSite = await context({ viewport: { width: 390, height: 844 }, colorScheme: theme, locale: 'en-US', isMobile: true, hasTouch: true });
    const sitePage = await mobileSite.ctx.newPage();
    await sitePage.goto(origin + '/site/?globe=fixture#live-network');
    await sitePage.locator('.live-globe').scrollIntoViewIfNeeded();
    await verifyPage(sitePage);
    await sitePage.locator('#live-network').screenshot({ path: resolve(out, `site-mobile-${theme}.png`) });
    report.screenshots.push(`site-mobile-${theme}.png`);
    await sitePage.getByRole('button', { name: '한국어' }).click();
    assert.equal(await sitePage.locator('.lg-caption').innerText(), '지금 연결된 맥 4대');
    assert.deepEqual(mobileSite.errors, []);
    await mobileSite.ctx.close();
  }

  const reduced = await context({ reducedMotion: 'reduce' });
  const map = await reduced.ctx.newPage();
  await map.goto(origin + '/apps/explorer/?globe=fixture#/network');
  await verifyPage(map);
  assert.equal(await map.locator('canvas[data-renderer="map"]:visible').count(), 1);
  assert.equal(await map.getByRole('button', { name: 'Pause globe' }).count(), 0);
  await map.screenshot({ path: resolve(out, 'explorer-reduced-motion.png'), fullPage: true });
  report.screenshots.push('explorer-reduced-motion.png');
  assert.deepEqual(reduced.errors, []);
  await reduced.ctx.close();
  report.checks.push('fixture light/dark desktop + mobile, local-only requests, static reduced-motion map');

  // The historical example covers multiple continents; additionally populate
  // Africa and Antarctica to exercise every known anchor plus unknown.
  const allRegions = { ...example, total: 27, versions: { '0.7.4': 27 }, regions: [
    ...example.regions, { continent: 'africa', count: 2, quality: summarizeQuality([.3, .4]) },
    { continent: 'antarctica', count: 1, quality: summarizeQuality([.2]) },
  ] };
  for (const reducedMotion of ['no-preference', 'reduce']) {
    const coverage = await context({ reducedMotion, ...(reducedMotion === 'reduce' ? { viewport: { width: 390, height: 844 } } : {}) }, () => ({ jsonrpc: '2.0', id: 1, result: allRegions }));
    const coveragePage = await coverage.ctx.newPage();
    await coveragePage.goto(origin + '/apps/explorer/#/network');
    await coveragePage.locator('.lg-total').filter({ hasText: /^27$/ }).waitFor();
    await verifyRepresentation(coveragePage, allRegions);
    if (reducedMotion === 'no-preference') await coveragePage.getByRole('button', { name: 'Pause globe', exact: true }).click();
    const asia = coveragePage.locator('.lg-marker[data-region="asia"]');
    await asia.hover();
    assert.equal(await coveragePage.locator('.lg-region[data-continent="asia"]').getAttribute('data-active'), 'true', 'pulse hover selects list row');
    await coveragePage.locator('.lg-region[data-continent="asia"]').hover();
    assert.equal(await asia.getAttribute('data-active'), 'true', 'list hover selects pulse');
    await coveragePage.locator('.lg-region[data-continent="north_america"] > .lg-region-name button').click();
    assert.equal(await coveragePage.locator('.lg-region[data-continent="north_america"]').getAttribute('data-active'), 'true', 'list tap/click selects region');
    if (reducedMotion === 'no-preference') {
      const rotatable = coveragePage.locator('canvas[data-renderer="webgl"]');
      await rotatable.focus();
      for (let step = 0; step < 24; step++) await coveragePage.keyboard.press('ArrowRight');
      await verifyRepresentation(coveragePage, allRegions);
    } else {
      assert.equal(await coveragePage.locator('.lg-marker:visible').count(), 9, 'static map shows countries and continent-only groups');
      const boxes = await coveragePage.locator('.lg-marker:visible .lg-marker-label').evaluateAll(labels => labels.map(label => {
        const rect = label.getBoundingClientRect();
        const stage = label.closest('.lg-stage').getBoundingClientRect();
        return { x: rect.x, y: rect.y, width: rect.width, height: rect.height, stage: { x: stage.x, y: stage.y, right: stage.right, bottom: stage.bottom } };
      }));
      for (const rect of boxes) {
        assert.ok(rect.x >= rect.stage.x && rect.x + rect.width <= rect.stage.right + 1, 'map label fits horizontal bounds');
        assert.ok(rect.y >= rect.stage.y && rect.y + rect.height <= rect.stage.bottom + 1, 'map label fits vertical bounds');
        for (const other of boxes) if (rect !== other) assert.ok(rect.x + rect.width <= other.x || other.x + other.width <= rect.x || rect.y + rect.height <= other.y || other.y + other.height <= rect.y, 'mobile map labels do not overlap');
      }
    }
    assert.deepEqual(coverage.errors, []);
    await coverage.ctx.close();
  }
  report.checks.push('every populated region has a visible marker or highlighted list entry; hover/click links both surfaces');

  // The current cohort contract uses disjoint UN M49 subregions. Broad legacy
  // regions must not create duplicate counts or fabricated quality evidence.
  const regionCounts = Object.fromEntries([...SUBREGION_CODES, 'world', 'unknown'].map(code => [code, 3]));
  const cohortTotal = Object.values(regionCounts).reduce((sum, count) => sum + count, 0);
  const cohort = { schema: 2, available: true, scope: 'unverified cohort observation',
    observed_at: Math.floor(Date.now() / 600_000) * 600, ttl_seconds: 600, minimum_bucket_size: 3,
    total: cohortTotal, by_role: { other: cohortTotal }, by_version: { unknown: cohortTotal }, by_region: regionCounts };
  const subregions = await context({ reducedMotion: 'reduce' }, () => ({ jsonrpc: '2.0', id: 1, result: cohort }));
  const subregionPage = await subregions.ctx.newPage();
  await subregionPage.goto(origin + '/apps/explorer/#/network');
  await subregionPage.locator('.lg-total').filter({ hasText: new RegExp(`^${cohortTotal}$`) }).waitFor();
  const visibleRegions = await subregionPage.locator('.lg-region:visible').evaluateAll(rows => rows.map(row => ({
    code: row.dataset.continent, count: Number(row.dataset.count), quality: row.dataset.quality,
  })));
  assert.deepEqual(visibleRegions.map(row => row.code).sort(), Object.keys(regionCounts).sort(),
    'reported subregions, folded world and unknown buckets remain separate');
  assert.equal(visibleRegions.reduce((sum, row) => sum + row.count, 0), cohortTotal, 'every transport is counted once');
  for (const row of visibleRegions) {
    assert.equal(row.count, regionCounts[row.code]);
    assert.equal(row.quality, '', 'uncommitted cohort counts carry no quality evidence');
  }
  assert.equal(await subregionPage.locator('.lg-country').count(), 0, 'aggregate cohorts contain no country disclosures');
  assert.equal(await subregionPage.locator('.lg-quality-gradient:visible').count(), 0);
  assert.match(await subregionPage.locator('.lg-status').innerText(), /Unverified cohort observations/);
  assert.deepEqual(subregions.errors, []);
  await subregions.ctx.close();
  report.checks.push('UN M49 cohort rows, world/unknown folding, exact disjoint totals and absent quality/country evidence');

  const idle = await context();
  const idlePage = await idle.ctx.newPage();
  await idlePage.clock.install();
  await idlePage.goto(origin + '/apps/explorer/?globe=fixture#/network');
  await verifyPage(idlePage);
  const idleCanvas = idlePage.locator('canvas[data-renderer="webgl"]');
  await idleCanvas.focus();
  await idlePage.keyboard.press('Home');
  await idlePage.mouse.move(1, 1);
  const rotationPosition = () => idlePage.locator('.lg-marker[data-region="asia"]').getAttribute('style');
  const idlePosition = await rotationPosition();
  await idlePage.clock.runFor(9_900);
  assert.equal(await rotationPosition(), idlePosition, 'interaction holds rotation through 9.9s idle');
  await idlePage.clock.runFor(600);
  assert.notEqual(await rotationPosition(), idlePosition, 'auto-rotation resumes after 10s idle');
  await idlePage.locator('.lg-region[data-continent="asia"] > .lg-region-name button').click();
  const selectedPosition = await rotationPosition();
  await idlePage.clock.runFor(9_900);
  assert.equal(await rotationPosition(), selectedPosition, 'row selection pauses auto-rotation');
  await idlePage.clock.runFor(600);
  assert.notEqual(await rotationPosition(), selectedPosition, 'retained selection resumes after 10s idle');
  const countryButton = idlePage.locator('.lg-country[data-region="asia:KR"] .lg-country-button');
  await countryButton.focus();
  await idlePage.evaluate(() => {
    window.__focusedCountryButton = document.activeElement;
    document.documentElement.dataset.theme = 'dark';
  });
  await idlePage.clock.runFor(100);
  assert.equal(await idlePage.evaluate(() => document.activeElement === window.__focusedCountryButton && window.__focusedCountryButton.isConnected), true, 'country focus survives renderer/text refresh');
  await idlePage.getByRole('button', { name: 'Pause globe' }).click();
  const held = await rotationPosition();
  await idlePage.clock.runFor(12_000);
  assert.equal(await rotationPosition(), held, 'manual pause remains paused beyond the idle delay');
  assert.deepEqual(idle.errors, []);
  await idle.ctx.close();
  report.checks.push('largest-region opening, 10s idle rotation delay, manual pause persists');

  // Exercise real polling against intercepted RPC envelopes (never a live node).
  const live = await context();
  const page = await live.ctx.newPage();
  await page.addInitScript(() => {
    const raf = window.requestAnimationFrame.bind(window);
    window.__globeFrames = 0;
    window.requestAnimationFrame = callback => raf(time => { window.__globeFrames++; callback(time); });
    const draw = WebGLRenderingContext.prototype.drawArrays;
    window.__globeDraws = 0;
    WebGLRenderingContext.prototype.drawArrays = function (...args) { window.__globeDraws++; return draw.apply(this, args); };
  });
  await page.goto(origin + '/apps/explorer/#/network');
  await verifyPage(page);
  assert.equal(live.requests.length, 1);
  const before = await page.evaluate(() => ({ frames: window.__globeFrames, time: performance.now() }));
  await page.waitForTimeout(2000);
  const after = await page.evaluate(() => ({ frames: window.__globeFrames, time: performance.now() }));
  report.frameRate = Math.round((after.frames - before.frames) * 1000 / (after.time - before.time));
  assert.ok(report.frameRate >= 30, 'software/browser frame smoke');
  await page.getByRole('button', { name: 'Pause globe' }).click();
  await page.waitForTimeout(100);
  const paused = await page.evaluate(() => window.__globeFrames);
  await page.waitForTimeout(200);
  assert.equal(await page.evaluate(() => window.__globeFrames), paused, 'pause has no animation loop');
  const canvas = page.locator('canvas[data-renderer="webgl"]');
  const box = await canvas.boundingBox();
  const draws = await page.evaluate(() => window.__globeDraws);
  await page.mouse.move(box.x + box.width * .4, box.y + box.height * .5);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width * .65, box.y + box.height * .5, { steps: 4 });
  await page.mouse.up();
  assert.ok(await page.evaluate(() => window.__globeDraws) > draws, 'drag redraws while paused');
  await canvas.focus();
  const keyed = await page.evaluate(() => window.__globeDraws);
  await page.keyboard.press('ArrowRight');
  assert.ok(await page.evaluate(() => window.__globeDraws) > keyed, 'keyboard rotates globe');

  // Simulated browser lifecycle dispatch exercises the actual visibility handler
  // in headless Chromium, whose background-tab policy is not deterministic.
  await page.getByRole('button', { name: 'Resume globe' }).click();
  await page.evaluate(() => { Object.defineProperty(document, 'hidden', { configurable: true, value: true }); document.dispatchEvent(new Event('visibilitychange')); });
  await page.waitForTimeout(100);
  const hidden = await page.evaluate(() => window.__globeFrames);
  const requested = live.requests.length;
  await page.waitForTimeout(10_500);
  assert.equal(await page.evaluate(() => window.__globeFrames), hidden, 'hidden tab schedules no frames');
  assert.equal(live.requests.length, requested, 'hidden tab does not poll');
  await page.evaluate(() => { Object.defineProperty(document, 'hidden', { configurable: true, value: false }); document.dispatchEvent(new Event('visibilitychange')); });
  await page.waitForTimeout(200);
  assert.equal(live.requests.length, requested + 1, 'resume gets fresh presence');
  await page.waitForTimeout(10_200);
  assert.equal(live.requests.length, requested + 2, 'visible polling every 10 seconds');
  const interval = live.requests.at(-1).time - live.requests.at(-2).time;
  assert.ok(interval >= 9500 && interval < 11_000, `poll interval ${interval} ms`);
  report.checks.push('RPC whitelist, 10s polling, drag, keyboard, pause, hidden-tab no frames/no polls, fresh resume');
  assert.deepEqual(live.errors, []);
  await live.ctx.close();

  let answer = { jsonrpc: '2.0', id: 1, error: { code: -32601, message: 'Sensitive server details must never be echoed' } };
  const states = await context({ colorScheme: 'light' }, () => answer);
  const statePage = await states.ctx.newPage();
  await statePage.goto(origin + '/apps/explorer/#/network');
  await statePage.locator('.lg-status[data-state="unavailable"]').waitFor();
  assert.equal(await statePage.locator('.lg-total').innerText(), '—', 'failed reads are never a fake zero/fixture');
  assert.ok(!(await statePage.locator('body').innerText()).includes('Sensitive server details'));
  await statePage.screenshot({ path: resolve(out, 'explorer-unavailable.png'), fullPage: true });
  report.screenshots.push('explorer-unavailable.png');
  answer = { jsonrpc: '2.0', id: 1, result: { ...fixture, total: 0, roles: Object.fromEntries(Object.keys(fixture.roles).map(role => [role, { count: 0 }])), versions: {}, regions: [], recent_blocks: [] } };
  await statePage.reload();
  await statePage.locator('.lg-status[data-state="empty"]').waitFor();
  assert.equal(await statePage.locator('.lg-total').innerText(), '0');
  answer = { jsonrpc: '2.0', id: 1, result: fixture };
  await statePage.reload();
  await verifyPage(statePage);
  answer = { jsonrpc: '2.0', id: 1, error: { code: -32601, message: 'Unavailable' } };
  await statePage.evaluate(() => document.dispatchEvent(new Event('visibilitychange')));
  await statePage.locator('.lg-status[data-state="stale"]').waitFor();
  assert.equal(await statePage.locator('.lg-total').innerText(), '4', 'stale counts carry an honest label');
  await statePage.screenshot({ path: resolve(out, 'explorer-stale.png'), fullPage: true });
  report.screenshots.push('explorer-stale.png');
  assert.deepEqual(states.errors, []);
  await states.ctx.close();
  report.checks.push('site mobile both themes + language toggle, unavailable/empty/stale without raw error echoes');

  const remount = await context();
  const session = await remount.ctx.newPage();
  await session.goto(origin + '/apps/explorer/#/network');
  await verifyPage(session);
  await session.evaluate(async snapshot => {
    const { mountLiveGlobe } = await import('./live-globe/live-globe.js');
    const host = document.createElement('div');
    document.body.replaceChildren(host);
    const options = { endpoint: 'https://rpc.eastsea.xyz', fetch: async () => ({ ok: true, json: async () => ({ jsonrpc: '2.0', id: 1, result: snapshot }) }) };
    const first = mountLiveGlobe(host, options);
    await new Promise(resolve => setTimeout(resolve, 180));
    host.querySelector('.lg-pause').click();
    host.querySelector('canvas').dispatchEvent(new KeyboardEvent('keydown', { key: 'Home', bubbles: true }));
    const position = () => host.querySelector('.lg-marker[data-region="asia"]').style.transform;
    window.__markerPositions = [position()];
    first.destroy();
    const second = mountLiveGlobe(host, options);
    await new Promise(resolve => setTimeout(resolve, 180));
    host.querySelector('.lg-pause').click();
    host.querySelector('canvas').dispatchEvent(new KeyboardEvent('keydown', { key: 'Home', bubbles: true }));
    window.__markerPositions.push(position());
    second.destroy();
  }, fixture);
  const positions = await session.evaluate(() => window.__markerPositions);
  assert.equal(positions.length, 2);
  assert.equal(positions[0], positions[1], 'jitter is stable across component mounts within one page session');
  assert.deepEqual(remount.errors, []);
  await remount.ctx.close();
  report.checks.push('same-page component remount preserves deterministic region jitter');

  await writeFile(resolve(out, 'report.json'), JSON.stringify(report, null, 2) + '\n');
  console.log(JSON.stringify(report, null, 2));
} finally {
  await browser.close();
}
