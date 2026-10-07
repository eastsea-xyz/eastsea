// Renders design/og/og-image.html to site/assets/og-image.png (1200x630).
// Usage from the repo root: node design/og/render-og.cjs
// Needs the `playwright` package and its Chromium (npx playwright install chromium).
const path = require("node:path");
const { chromium } = require("playwright");

(async () => {
  const root = path.resolve(__dirname, "..", "..");
  const browser = await chromium.launch();
  try {
    const page = await browser.newPage({ viewport: { width: 1200, height: 630 } });
    await page.goto("file://" + path.join(root, "design/og/og-image.html"));
    await page.evaluate(() => document.fonts.ready);
    await page.waitForTimeout(300);
    const out = path.join(root, "site/assets/og-image.png");
    await page.screenshot({ path: out, clip: { x: 0, y: 0, width: 1200, height: 630 } });
    console.log("wrote", out);
  } finally {
    await browser.close();
  }
})().catch((e) => { console.error(e); process.exit(1); });
