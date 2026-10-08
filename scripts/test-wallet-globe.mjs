#!/usr/bin/env node
// Offline browser checks for the same host bundled into WKWebView. No server,
// node or external request is used. Native fixture PNGs remain wallet-screens.sh.
import assert from 'node:assert/strict';
import { readFile, mkdir, writeFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { extname, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(fileURLToPath(new URL('../', import.meta.url)));
process.env.TMPDIR = resolve(root, 'tmp');
const out = resolve(root, 'tmp/wallet-globe-browser');
await mkdir(out, { recursive: true });
const { chromium } = createRequire(import.meta.url)(process.env.PLAYWRIGHT_MODULE || 'playwright');
const fixture = JSON.parse(await readFile(resolve(root, 'apps/explorer/test/fixtures/presence-example.json')));
const bundle = resolve(root, 'apps/wallet/Resources/LiveGlobe');
const origin = 'http://wallet-globe.test.invalid';
const mime = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css' };
const captions = { en: 'Macs this node can see', ko: '이 노드가 보고 있는 Mac들',
  ja: 'このノードから見えるMac', 'zh-Hans': '此节点可见的Mac', es: 'Macs que este nodo puede ver' };
const browser = await chromium.launch({ headless: true, channel: process.env.GLOBE_BROWSER_CHANNEL || 'chrome' });
const report = { screenshots: [], checks: [], requests: [] };
try {
  for (const theme of ['light', 'dark']) {
    const context = await browser.newContext({ viewport: { width: 712, height: 900 }, colorScheme: theme });
    const errors = [];
    await context.addInitScript(() => {
      globalThis.globeAnimationFrames = 0;
      const requestFrame = globalThis.requestAnimationFrame.bind(globalThis);
      globalThis.requestAnimationFrame = callback => requestFrame(time => {
        globalThis.globeAnimationFrames++;
        callback(time);
      });
    });
    await context.route('**/*', async route => {
      const url = new URL(route.request().url());
      assert.equal(url.origin, origin, 'no outside server, CDN, RPC, fonts or tracker');
      const file = resolve(bundle, '.' + url.pathname);
      assert.ok(file.startsWith(bundle + sep));
      assert.ok(mime[extname(file)], 'only static host modules and CSS are requested');
      report.requests.push(url.pathname);
      await route.fulfill({ contentType: mime[extname(file)], body: await readFile(file) });
    });
    const page = await context.newPage();
    page.on('pageerror', error => errors.push(error.message));
    page.on('console', message => { if (message.type() === 'error') errors.push(message.text()); });
    await page.goto(origin + '/index.html', { waitUntil: 'networkidle' });
    await page.waitForFunction(() => globalThis.eastseaGlobe?.ready);
    for (const lang of Object.keys(captions)) {
      assert.equal(await page.evaluate(({ fixture, lang, theme }) => {
        eastseaGlobe.configure({ paused: true, reducedMotion: false, theme, lang, fixture: true, evidenceAvailable: true });
        return eastseaGlobe.update(fixture);
      }, { fixture, lang, theme }), true);
      assert.equal(await page.locator('.lg-caption').innerText(), captions[lang]);
      assert.equal(await page.locator('.lg-total').innerText(), '24');
      assert.equal(await page.locator('.lg-region').count(), 8);
      assert.equal(await page.locator('.lg-quality-gradient').isVisible(), true);
      assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false);
      if (['en', 'ko'].includes(lang)) {
        // Give the transparent embedded host a standalone reference backdrop.
        await page.evaluate(() => { document.body.style.backgroundColor = 'var(--c-bg)'; });
        const name = `wallet-globe-${lang}-${theme}.png`;
        await page.screenshot({ path: resolve(out, name), fullPage: true, animations: 'disabled' });
        report.screenshots.push(name);
      }
    }
    await page.evaluate(() => eastseaGlobe.configure({ reducedMotion: true }));
    assert.equal(await page.locator('[data-renderer="map"]:visible').count(), 1);
    assert.equal(await page.locator('.lg-pause').isVisible(), false);
    await page.evaluate(() => eastseaGlobe.configure({ reducedMotion: false, paused: true }));
    await page.waitForTimeout(150);
    const frames = await page.evaluate(() => globeAnimationFrames);
    await page.waitForTimeout(300);
    assert.equal(await page.evaluate(() => globeAnimationFrames), frames, 'paused globe schedules no animation frames');
    assert.equal(await page.evaluate(() => {
      Object.defineProperty(document, 'hidden', { configurable: true, value: true });
      const drawn = eastseaGlobe.captureFrame();
      delete document.hidden;
      return drawn;
    }), true, 'fixture draws a genuine still frame even when its native window is offscreen');
    await page.waitForTimeout(150);
    assert.equal(await page.evaluate(() => globeAnimationFrames), frames, 'still capture starts no animation loop');
    await page.evaluate(() => eastseaGlobe.configure({ state: 'stale' }));
    assert.equal(await page.locator('.lg-status').getAttribute('data-state'), 'stale');
    assert.equal(await page.locator('.lg-total').innerText(), '24');
    await page.evaluate(() => eastseaGlobe.configure({ evidenceAvailable: false }));
    assert.equal(await page.locator('.lg-quality-gradient').isVisible(), false);
    assert.equal(await page.locator('.lg-quality-status').isVisible(), true);
    assert.equal(await page.locator('.lg-role-summary').innerText().then(text => /Reserve|예비|予備|备用|reserva/.test(text)), false);
    assert.equal(await page.evaluate(() => eastseaGlobe.reset()), true);
    assert.equal(await page.locator('.lg-total').innerText(), '—');
    assert.equal(await page.locator('.lg-country').count(), 0);
    assert.ok(await page.locator('.lg-marker').evaluateAll(markers => markers.every(marker => Number(marker.dataset.count || 0) === 0)));
    await page.evaluate(() => eastseaGlobe.configure({ state: 'unavailable' }));
    assert.equal(await page.locator('.lg-status').getAttribute('data-state'), 'unavailable');
    await page.setViewportSize({ width: 380, height: 900 });
    await page.evaluate(fixture => { eastseaGlobe.configure({ reducedMotion: true, evidenceAvailable: true }); eastseaGlobe.update(fixture); }, fixture);
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false, 'narrow wallet has no overflow');
    assert.deepEqual(errors, []);
    await context.close();
  }
  report.checks.push('5 languages × light/dark; counts/regions/gradient; reduced motion; paused RAF; stale/reset/unavailable; unmeasured quality; narrow layout; local-only assets');
  await writeFile(resolve(out, 'report.json'), JSON.stringify(report, null, 2) + '\n');
  console.log(`Wallet globe browser checks passed; ${report.screenshots.length} reference PNGs in tmp/wallet-globe-browser.`);
} finally { await browser.close(); }
