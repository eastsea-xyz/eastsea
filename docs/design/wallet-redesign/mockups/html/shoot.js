// Renders every mockup board to ../<name>.png.
// Run: NODE_PATH=<dir with playwright> node shoot.js [name ...]
// Uses the installed Google Chrome (channel "chrome").
const { chromium } = require('playwright');
const fs = require('fs');
const path = require('path');

(async () => {
  const dir = __dirname;
  const only = process.argv.slice(2);
  const pages = fs.readdirSync(dir).filter(f => /^\d\d-.*\.html$/.test(f))
    .filter(f => !only.length || only.some(o => f.startsWith(o)));
  const browser = await chromium.launch({ channel: 'chrome' });
  const ctx = await browser.newContext({ viewport: { width: 1800, height: 1000 }, deviceScaleFactor: 2 });
  for (const f of pages) {
    const page = await ctx.newPage();
    await page.goto('file://' + path.join(dir, f));
    await page.evaluate(() => document.fonts.ready);
    await page.waitForTimeout(150);
    const out = path.join(dir, '..', f.replace('.html', '.png'));
    await page.locator('.board').screenshot({ path: out });
    console.log('wrote', path.relative(process.cwd(), out));
    await page.close();
  }
  await browser.close();
})();
