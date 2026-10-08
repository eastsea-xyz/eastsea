#!/usr/bin/env node
// Uses an already-installed Playwright and Chrome; installs nothing.
// NODE_PATH=<installed playwright node_modules> node scripts/render-design-previews.mjs [menu|web|all]
import { createRequire } from 'node:module';
import { mkdir, writeFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
const require = createRequire(import.meta.url);
const { chromium } = require('playwright');
const root = fileURLToPath(new URL('../', import.meta.url));
const out = path.join(root, 'docs/design/wallet-redesign/mockups');
const base = process.env.DESIGN_PREVIEW_URL || 'http://127.0.0.1:18774';
const only = process.argv[2] || 'all';
if (!['menu', 'web', 'toolbox', 'all'].includes(only)) throw new Error('Choose menu, web, toolbox, or all');
process.env.TMPDIR = path.join(root, 'tmp');
await mkdir(process.env.TMPDIR, { recursive: true });
const browser = await chromium.launch({ channel: 'chrome', headless: true });
const captures = [];
try {
  const jobs = [];
  if (only === 'menu' || only === 'all') for (const theme of ['light', 'dark']) for (const language of ['en', 'ko']) {
    jobs.push({ name: `menubar-${theme}-${language}`, route: `menubar-${theme}-${language}.html`, width: 464, height: 720, selector: '.board' });
  }
  if (only === 'web' || only === 'all') for (const theme of ['light', 'dark']) {
    jobs.push({ name: `extension-${theme}`, route: `extension-preview.html?theme=${theme}&view=home`, width: 360, height: 720 });
    jobs.push({ name: `extension-assets-${theme}`, route: `extension-preview.html?theme=${theme}&view=assets`, width: 360, height: 720 });
    jobs.push({ name: `extension-unverified-${theme}`, route: `extension-preview.html?theme=${theme}&view=assets&unverified=open`, width: 360, height: 720 });
    jobs.push({ name: `explorer-${theme}`, route: `explorer-preview.html?theme=${theme}`, width: 1440, height: 1000 });
    jobs.push({ name: `explorer-mobile-${theme}`, route: `explorer-preview.html?theme=${theme}`, width: 390, height: 844 });
  }
  if (only === 'toolbox' || only === 'all') for (const theme of ['light', 'dark']) {
    const toolbox = process.env.TOOLBOX_PREVIEW_URL || `${base}/tmp/redesign-system/toolbox/apps`;
    jobs.push({ name: `toolbox-${theme}`, url: `${toolbox}/`, width: 1440, height: 1000 });
    jobs.push({ name: `toolbox-token-${theme}`, url: `${toolbox}/token/`, width: 1440, height: 1000 });
    jobs.push({ name: `toolbox-mobile-${theme}`, url: `${toolbox}/`, width: 390, height: 844 });
  }
  for (const job of jobs) {
    const context = await browser.newContext({ viewport: { width: job.width, height: job.height }, deviceScaleFactor: 2, reducedMotion: 'reduce', colorScheme: job.name.includes('-dark') ? 'dark' : 'light' });
    const page = await context.newPage();
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    page.on('console', message => { if (message.type() === 'error') errors.push(message.text()); });
    await page.goto(job.url || `${base}/docs/design/wallet-redesign/mockups/html/${job.route}`);
    if (job.name.startsWith('extension-assets')) await page.waitForSelector('.holding-amount');
    else if (job.name.startsWith('extension-unverified')) await page.waitForSelector('.unverified-art');
    else if (job.name.startsWith('extension')) await page.waitForFunction(() => document.querySelector('.balance-value')?.textContent !== '…' && document.querySelector('.balance-value'));
    if (job.name.startsWith('explorer')) await page.waitForSelector('.tile.major');
    if (job.name.startsWith('toolbox-token')) await page.waitForFunction(() => !document.querySelector('#wallets')?.textContent.includes('찾는 중'));
    await page.evaluate(() => document.fonts.ready);
    const file = path.join(out, `${job.name}.png`);
    if (job.selector) await page.locator(job.selector).screenshot({ path: file });
    else await page.screenshot({ path: file, fullPage: true });
    if (job.name.startsWith('explorer-mobile')) {
      const ledger = await page.evaluate(() => {
        const table = document.querySelector('table');
        const scroller = table.closest('.tablewrap');
        scroller.scrollLeft = scroller.scrollWidth;
        return { minimumWidth: getComputedStyle(table).minWidth, whiteSpace: getComputedStyle(table.querySelector('td')).whiteSpace, clientWidth: scroller.clientWidth, scrollWidth: scroller.scrollWidth, scrollLeft: scroller.scrollLeft };
      });
      if (ledger.minimumWidth !== '640px' || ledger.whiteSpace !== 'nowrap' || ledger.scrollWidth <= ledger.clientWidth || !ledger.scrollLeft) throw new Error(`Unreadable mobile ledger: ${JSON.stringify(ledger)}`);
      await page.locator('.tablewrap').screenshot({ path: path.join(out, `${job.name.replace('mobile-', 'mobile-ledger-')}.png`) });
    }
    const metrics = await page.evaluate(() => ({ width: innerWidth, scrollWidth: document.documentElement.scrollWidth, background: getComputedStyle(document.body).backgroundColor, theme: document.documentElement.dataset.theme, font: getComputedStyle(document.body).fontFamily }));
    captures.push({ file: path.relative(root, file), ...metrics, errors });
    console.log(path.relative(root, file));
    await context.close();
  }
  await writeFile(path.join(root, `tmp/redesign-system/render-${only}.json`), JSON.stringify(captures, null, 2) + '\n');
  const failures = captures.filter(c => c.errors.length || c.scrollWidth > c.width);
  if (failures.length) throw new Error(JSON.stringify(failures));
} finally { await browser.close(); }
