#!/usr/bin/env node
// Real-browser offline smoke. Uses existing Playwright tooling, no app deps.
// PLAYWRIGHT_MODULE=/absolute/path/to/playwright node scripts/test-live-globe.mjs
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { readFile, mkdir, writeFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { resolve, extname, sep } from 'node:path';
import { createRequire } from 'node:module';

const root = resolve(fileURLToPath(new URL('../', import.meta.url)));
const out = resolve(root, process.env.GLOBE_SCREENSHOTS || 'tmp/live-globe');
await mkdir(out, { recursive: true });
const { chromium } = createRequire(import.meta.url)(process.env.PLAYWRIGHT_MODULE || 'playwright');
const fixture = JSON.parse(await readFile(resolve(root, 'apps/explorer/live-globe/fixture.json'), 'utf8'));
const mime = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.json': 'application/json', '.svg': 'image/svg+xml', '.woff2': 'font/woff2', '.png': 'image/png', '.webp': 'image/webp' };
const server = createServer(async (request, response) => {
  const path = decodeURIComponent(new URL(request.url, 'http://test.invalid').pathname);
  const file = resolve(root, '.' + (path.endsWith('/') ? path + 'index.html' : path));
  if (!file.startsWith(root + sep)) { response.writeHead(403).end(); return; }
  try {
    response.writeHead(200, { 'Content-Type': mime[extname(file)] || 'application/octet-stream' });
    response.end(await readFile(file));
  } catch { response.writeHead(404).end(); }
});
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
const origin = `http://127.0.0.1:${server.address().port}`;
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
    if (url.startsWith(origin + '/')) return route.continue();
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
  await page.locator('.lg-total').filter({ hasText: '24' }).waitFor();
  await page.evaluate(() => document.fonts.ready);
  assert.equal(await page.locator('.lg-region').count(), 8);
  assert.equal(await page.locator('.lg-country').count(), 2);
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false, 'mobile overflow');
  const content = await page.locator('body').innerText();
  assert.equal(/(?:\d{1,3}\.){3}\d{1,3}|latitude|longitude|node[_ -]?id|peer[_ -]?id/i.test(content), false, 'no precise/identifying location text');
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
    assert.equal(await sitePage.locator('.lg-caption').innerText(), '이 노드가 보고 있는 Mac들');
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
  answer = { jsonrpc: '2.0', id: 1, result: { ...fixture, total: 0, roles: { validator: 0, candidate: 0, follower: 0 }, versions: {}, regions: [], recent_blocks: [] } };
  await statePage.reload();
  await statePage.locator('.lg-status[data-state="empty"]').waitFor();
  assert.equal(await statePage.locator('.lg-total').innerText(), '0');
  answer = { jsonrpc: '2.0', id: 1, result: fixture };
  await statePage.reload();
  await verifyPage(statePage);
  answer = { jsonrpc: '2.0', id: 1, error: { code: -32601, message: 'Unavailable' } };
  await statePage.evaluate(() => document.dispatchEvent(new Event('visibilitychange')));
  await statePage.locator('.lg-status[data-state="stale"]').waitFor();
  assert.equal(await statePage.locator('.lg-total').innerText(), '24', 'stale counts carry an honest label');
  await statePage.screenshot({ path: resolve(out, 'explorer-stale.png'), fullPage: true });
  report.screenshots.push('explorer-stale.png');
  assert.deepEqual(states.errors, []);
  await states.ctx.close();
  report.checks.push('site mobile both themes + language toggle, unavailable/empty/stale without raw error echoes');

  const remount = await context();
  const session = await remount.ctx.newPage();
  await session.addInitScript(() => {
    const upload = WebGLRenderingContext.prototype.bufferSubData;
    window.__markerUploads = [];
    WebGLRenderingContext.prototype.bufferSubData = function (target, offset, data) {
      if (data?.length === 35) window.__markerUploads.push(Array.from(data));
      return upload.call(this, target, offset, data);
    };
  });
  await session.goto(origin + '/apps/explorer/#/network');
  await verifyPage(session);
  await session.evaluate(async snapshot => {
    const { mountLiveGlobe } = await import('./live-globe/live-globe.js');
    const host = document.createElement('div');
    document.body.replaceChildren(host);
    const options = { endpoint: 'https://rpc.eastsea.xyz', fetch: async () => ({ ok: true, json: async () => ({ jsonrpc: '2.0', id: 1, result: snapshot }) }) };
    const first = mountLiveGlobe(host, options);
    await new Promise(resolve => setTimeout(resolve, 100));
    first.destroy();
    const second = mountLiveGlobe(host, options);
    await new Promise(resolve => setTimeout(resolve, 100));
    second.destroy();
  }, fixture);
  const uploaded = await session.evaluate(() => window.__markerUploads);
  assert.ok(uploaded.length >= 3);
  assert.deepEqual(uploaded.at(-1), uploaded.at(-2), 'jitter is stable across component mounts within one page session');
  assert.deepEqual(remount.errors, []);
  await remount.ctx.close();
  report.checks.push('same-page component remount preserves deterministic region jitter');

  await writeFile(resolve(out, 'report.json'), JSON.stringify(report, null, 2) + '\n');
  console.log(JSON.stringify(report, null, 2));
} finally {
  await browser.close();
  await new Promise(resolve => server.close(resolve));
}
