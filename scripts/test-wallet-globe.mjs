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
const captions = { en: '24 Macs connected now', ko: '지금 연결된 맥 24대',
  ja: '現在接続中のMac 24台', 'zh-Hans': '当前连接的Mac：24台', es: '24 Macs conectados ahora' };
const browser = await chromium.launch({ headless: true, channel: process.env.GLOBE_BROWSER_CHANNEL || 'chrome' });
const report = { screenshots: [], checks: [], requests: [], geometry: [] };

async function verifyGeometry(page, theme, lang, width) {
  const geometry = await page.evaluate(() => {
    eastseaGlobe.captureFrame();
    const stage = document.querySelector('.lg-stage').getBoundingClientRect();
    const rect = element => {
      const b = element.getBoundingClientRect();
      return { x: b.x - stage.x, y: b.y - stage.y, w: b.width, h: b.height };
    };
    const canvas = document.querySelector('.lg-canvas');
    const markers = [...document.querySelectorAll('.lg-marker:not([hidden])')];
    const labels = markers.filter(marker => !marker.querySelector('.lg-marker-label').hidden)
      .map(marker => ({ ...rect(marker.querySelector('.lg-marker-label')), key: marker.dataset.region }));
    const pulses = markers.map(marker => {
      const b = marker.getBoundingClientRect();
      return { key: marker.dataset.region, x: b.x - stage.x + b.width / 2, y: b.y - stage.y + b.height / 2,
        radius: parseFloat(getComputedStyle(marker).getPropertyValue('--marker-size')) / 2 * 1.035 + 4 };
    });
    let ink;
    if (canvas.dataset.renderer === 'webgl') {
      const gl = canvas.getContext('webgl'), pixels = new Uint8Array(canvas.width * canvas.height * 4);
      gl.readPixels(0, 0, canvas.width, canvas.height, gl.RGBA, gl.UNSIGNED_BYTE, pixels);
      ink = { left: canvas.width, right: 0, bottom: canvas.height, top: 0, count: 0 };
      for (let y = 0; y < canvas.height; y++) for (let x = 0; x < canvas.width; x++) {
        if (pixels[(y * canvas.width + x) * 4 + 3] < 4) continue;
        ink.left = Math.min(ink.left, x); ink.right = Math.max(ink.right, x);
        ink.bottom = Math.min(ink.bottom, y); ink.top = Math.max(ink.top, y); ink.count++;
      }
    }
    return { stage: { w: stage.width, h: stage.height }, canvas: rect(canvas), labels, pulses, ink,
      pixels: { w: canvas.width, h: canvas.height }, hiddenLabels: markers.length - labels.length,
      columns: getComputedStyle(document.querySelector('.live-globe')).gridTemplateColumns.split(' ').length,
      sticky: getComputedStyle(document.querySelector('.lg-figure')).position };
  });
  const detail = `${lang}/${theme}/${width}px`;
  assert.ok(Math.abs(geometry.stage.w - geometry.stage.h) < 1, `${detail}: square globe stage`);
  assert.ok(Math.abs(geometry.canvas.w - geometry.stage.w) < 1 && Math.abs(geometry.canvas.h - geometry.stage.h) < 1, `${detail}: canvas fills the stage`);
  assert.equal(geometry.hiddenLabels, 0, `${detail}: all visible fixture regions have room for labels`);
  assert.equal(geometry.columns, width <= 680 ? 1 : 2, `${detail}: responsive columns`);
  assert.equal(geometry.sticky, width <= 680 ? 'static' : 'sticky', `${detail}: desktop globe stays in view`);
  if (geometry.ink) {
    assert.ok(geometry.ink.count > 0, `${detail}: sphere genuinely rendered`);
    const margin = Math.min(geometry.pixels.w, geometry.pixels.h) * .025;
    assert.ok(geometry.ink.left >= margin && geometry.ink.bottom >= margin
      && geometry.pixels.w - geometry.ink.right >= margin && geometry.pixels.h - geometry.ink.top >= margin, `${detail}: whole sphere and halo have breathing room`);
  }
  for (const label of geometry.labels) {
    assert.ok(label.x >= 3.5 && label.y >= 3.5 && label.x + label.w <= geometry.stage.w - 3.5 && label.y + label.h <= geometry.stage.h - 3.5, `${detail}: ${label.key} stays in bounds`);
    for (const other of geometry.labels) {
      if (label === other) continue;
      assert.ok(label.x + label.w <= other.x || other.x + other.w <= label.x || label.y + label.h <= other.y || other.y + other.h <= label.y, `${detail}: ${label.key} overlaps ${other.key}`);
    }
    for (const pulse of geometry.pulses) {
      const dx = pulse.x - Math.max(label.x, Math.min(pulse.x, label.x + label.w));
      const dy = pulse.y - Math.max(label.y, Math.min(pulse.y, label.y + label.h));
      assert.ok(Math.hypot(dx, dy) >= pulse.radius - .5, `${detail}: ${label.key} covers ${pulse.key} pulse`);
    }
  }
  return { theme, lang, width, ...geometry };
}
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
      assert.equal(await page.locator('.lg-region-position, .lg-quality-strip').count(), 0);
      assert.equal(await page.locator('.lg-role-summary').innerText().then(text => /\b0\b/.test(text)), false);
      assert.equal(await page.locator('.lg-marker[data-region="asia"]').getAttribute('data-count'), '9');
      assert.equal(await page.locator('.lg-region[data-continent="asia"]').getAttribute('data-count'), '9');
      assert.equal(await page.locator('.lg-marker[data-region="asia:KR"]').getAttribute('data-count'), '6');
      report.geometry.push(await verifyGeometry(page, theme, lang, 712));
      if (['en', 'ko'].includes(lang)) {
        // Give the transparent embedded host a standalone reference backdrop.
        await page.evaluate(() => { document.body.style.backgroundColor = 'var(--c-bg)'; });
        const name = `wallet-globe-${lang}-${theme}.png`;
        await page.screenshot({ path: resolve(out, name), fullPage: true, animations: 'disabled' });
        report.screenshots.push(name);
      }
      for (const width of [320, 390, 639, 640, 680, 681, 1024, 1440]) {
        await page.setViewportSize({ width, height: 900 });
        report.geometry.push(await verifyGeometry(page, theme, lang, width));
        assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false);
        if (width === 390 && lang === 'ko' && theme === 'light') {
          const name = 'wallet-globe-ko-light-390.png';
          await page.screenshot({ path: resolve(out, name), fullPage: true, animations: 'disabled' });
          report.screenshots.push(name);
        }
      }
      await page.setViewportSize({ width: 712, height: 900 });
    }
    // Sweep both sides of the sphere after the reference captures. Pausing
    // stops idle motion but still permits the real keyboard rotation control.
    for (const lang of Object.keys(captions)) {
      await page.evaluate(lang => eastseaGlobe.configure({ lang }), lang);
      for (const width of [390, 712]) {
        await page.setViewportSize({ width, height: 900 });
        await page.locator('.lg-canvas[data-renderer="webgl"]').press('Home');
        for (let angle = 0; angle < 7; angle++) {
          for (let step = 0; step < 8; step++) await page.locator('.lg-canvas[data-renderer="webgl"]').press('ArrowRight');
          report.geometry.push({ ...await verifyGeometry(page, theme, lang, width), rotationStep: (angle + 1) * 8 });
        }
      }
    }
    await page.setViewportSize({ width: 712, height: 900 });
    await page.locator('.lg-canvas[data-renderer="webgl"]').press('Home');
    // A longer valid country list makes the document scroll; the figure stays
    // at the viewport inset while the list continues alongside it.
    const { summarizeQuality } = await import('../apps/explorer/live-globe/quality.js');
    const extra = ['AT', 'BE', 'BG', 'CH', 'CZ', 'DK', 'EE', 'ES', 'FI', 'FR', 'GB', 'GR', 'HR', 'HU', 'IE', 'IS', 'IT', 'LT', 'LU', 'LV', 'MT', 'NL', 'NO', 'PL', 'PT', 'RO', 'RS', 'SE', 'SI', 'SK']
      .map(country => ({ continent: 'europe', country, count: 3, quality: summarizeQuality([.2, .3, .4]) }));
    const longList = { ...fixture, total: fixture.total + extra.length * 3, versions: { '0.7.4': fixture.total + extra.length * 3 }, regions: [...fixture.regions, ...extra] };
    await page.evaluate(snapshot => eastseaGlobe.update(snapshot), longList);
    await page.evaluate(() => scrollTo(0, 200));
    assert.ok(Math.abs(await page.locator('.lg-figure').evaluate(el => el.getBoundingClientRect().top) - 16) < 1, 'figure sticks while a longer region list scrolls');
    const far = page.locator('.lg-region[data-continent="north_america"]');
    await far.locator('.lg-region-button').focus();
    assert.equal(await far.getAttribute('data-active'), 'true');
    assert.equal(await far.locator('.lg-region-button').getAttribute('aria-pressed'), 'true');
    await far.locator('.lg-region-button').press('Escape');
    assert.equal(await far.getAttribute('data-active'), 'false');
    await page.evaluate(fixture => { scrollTo(0, 0); eastseaGlobe.update(fixture); }, fixture);
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
  report.checks.push('5 languages × light/dark × 9 widths; seven rotations at desktop/mobile widths; answer-first headline; nonzero roles; shared continent totals; no row machinery; label/pulse collisions; sphere/halo pixel bounds; sticky scrolling and selection; reduced motion; paused RAF; stale/reset/unavailable; unmeasured quality; local-only assets');
  await writeFile(resolve(out, 'report.json'), JSON.stringify(report, null, 2) + '\n');
  console.log(`Wallet globe browser checks passed; ${report.screenshots.length} reference PNGs in tmp/wallet-globe-browser.`);
} finally { await browser.close(); }
