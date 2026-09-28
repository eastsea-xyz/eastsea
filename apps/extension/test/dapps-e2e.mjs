// The three projects together, in Chromium with this extension: DEX (swap, create a
// token, approve + add AETH liquidity) and the launchpad (launch, buy on the curve),
// all through their own UIs and confirmed on the testnet.
//   DEX:       cd ../aether-dex && node scripts/serve.mjs                 (:8080)
//   launchpad: cd ../aether-launchpad-demo && python3 script/serve.py --port 8081
//   npm i --no-save playwright && npx playwright install chromium
//   scripts/build-extension.sh && node apps/extension/test/dapps-e2e.mjs
import { chromium } from 'playwright';
import fs from 'node:fs';
const EXT = new URL('..', import.meta.url).pathname.replace(/\/$/, '');
const SHOTS = process.env.SHOTS || '/tmp/aether-extension-e2e/';
fs.mkdirSync(SHOTS, { recursive: true });
const ctx = await chromium.launchPersistentContext('', { channel: 'chromium', headless: true, viewport: { width: 1280, height: 900 },
  args: [`--disable-extensions-except=${EXT}`, `--load-extension=${EXT}`] });
const approved = [];
ctx.on('page', async (p) => {
  try { await p.waitForURL(/approve=/, { timeout: 8000 }); } catch { return; }
  const b = p.getByRole('button', { name: /^(Connect|Approve)$/ });
  await b.waitFor({ timeout: 15000 }).catch(() => {});
  await p.getByText(/up to|Account/).first().waitFor({ timeout: 10000 }).catch(() => {});
  approved.push((await p.locator('.kv strong').count()) ? await p.locator('.kv strong').first().innerText() : 'connect');
  await b.click().catch(() => {});
});
const toastsOf = (page) => page.locator('#toasts > *, .toast').allInnerTexts().then((a) => a.join(' | ').replace(/\s+/g, ' ')).catch(() => '');
async function waitToast(page, re, label, ms = 60000) {
  const end = Date.now() + ms;
  while (Date.now() < end) { const t = await toastsOf(page); if (re.test(t)) { console.log(`OK ${label}`); return t; } await page.waitForTimeout(1000); }
  console.log(`FAIL ${label}: ${await toastsOf(page)}`); process.exitCode = 1; await page.screenshot({ path: `${SHOTS}fail-${label.replace(/\W+/g, '-')}.png` });
  return null;
}
try {
  let [sw] = ctx.serviceWorkers(); sw ??= await ctx.waitForEvent('serviceworker');
  const id = new URL(sw.url()).host;
  const popup = await ctx.newPage();
  await popup.goto(`chrome-extension://${id}/ui/popup.html`);
  // First run: accept the one-time notice, then create the wallet.
  await popup.getByRole('button', { name: 'I understand' }).click();
  const pws = popup.locator('input[type=password]');
  await pws.nth(0).fill('e2e-password-1'); await pws.nth(1).fill('e2e-password-1');
  await popup.getByRole('button', { name: 'Create wallet' }).click();
  await popup.getByText('Block ').waitFor({ timeout: 20000 });
  await popup.getByRole('button', { name: /Get test AETH/ }).click();
  await popup.locator('.msg').first().waitFor({ timeout: 20000 });
  await popup.waitForTimeout(3000); await popup.close();

  // ---- DEX ----
  const dex = await ctx.newPage();
  await dex.goto('http://localhost:8080/'); await dex.waitForTimeout(3500);
  await dex.locator('#connectBtn').click();
  await waitToast(dex, /Wallet connected/, 'dex connect');
  // swap AETH -> NEB so we hold NEB
  await dex.locator('#pickFrom').click();
  await dex.locator('.tokrow', { has: dex.locator('.s', { hasText: /^AETH$/ }) }).first().click();
  await dex.locator('#pickTo').click();
  await dex.locator('.tokrow', { has: dex.locator('.s', { hasText: /^NEB$/ }) }).first().click();
  await dex.locator('#amtIn').fill('1'); await dex.waitForTimeout(1500);
  await dex.locator('#swapBtn').click();
  await waitToast(dex, /Swap 1 AETH → NEB confirmed/, 'dex swap AETH->NEB');
  // create a token
  await dex.locator('button[data-view=create]').click();
  await dex.locator('[data-k=name]').fill('E2E Token'); await dex.locator('[data-k=symbol]').fill('E2ET');
  await dex.locator('#createBtn').click();
  await waitToast(dex, /Create E2ET[^|]*confirmed/, 'dex create token');
  await dex.waitForTimeout(1000);
  console.log('dex toasts:', (await toastsOf(dex)).slice(0, 300));
  // add liquidity to NEB/AETH
  await dex.locator('button[data-view=pools]').click(); await dex.waitForTimeout(2500);
  const card = dex.locator('.card.pool', { hasText: /NEB\s*\/\s*AETH|AETH\s*\/\s*NEB/ }).first();
  await card.locator('[data-add]').click();
  await dex.locator('[data-x]').fill(/AETH/.test(await dex.locator('[data-pa]').innerText()) ? '0.05' : '5'); await dex.waitForTimeout(1500);
  console.log('add liq: A', await dex.locator('[data-pa]').innerText(), 'B', await dex.locator('[data-pb]').innerText(), 'y', await dex.locator('[data-y]').inputValue());
  for (let step = 0; step < 3; step += 1) {
    const label = (await dex.locator('[data-go]').innerText()).trim();
    console.log('liquidity button:', label);
    if (!/^Approve|^Add liquidity$/.test(label)) break;
    await dex.locator('[data-go]').click();
    if (label === 'Add liquidity') break;
    // wait for the approval to land and the button to change
    const end = Date.now() + 60000;
    while (Date.now() < end && (await dex.locator('[data-go]').innerText().catch(() => '')).trim() === label) await dex.waitForTimeout(1000);
  }
  await waitToast(dex, /(Add|liquidity)[^|]*confirmed/i, 'dex add liquidity', 90000);
  await dex.screenshot({ path: `${SHOTS}flows-dex.png` });

  // ---- launchpad ----
  const lp = await ctx.newPage();
  await lp.goto('http://localhost:8081/'); await lp.waitForTimeout(3500);
  await lp.locator('#walletBtn').click(); await lp.waitForTimeout(3000);
  await lp.locator('#lfName').fill('E2E Rocket'); await lp.locator('#lfSym').fill('E2ER');
  await lp.locator('#lfGo').click();
  await waitToast(lp, /success|launched/i, 'launchpad launch', 90000);
  await lp.waitForTimeout(3000);
  console.log('launchpad url after launch:', lp.url());
  if (!/#\/t\//.test(lp.url())) {
    await lp.goto('http://localhost:8081/#/'); await lp.waitForTimeout(3000);
    await lp.locator('a.tcard', { hasText: 'E2ER' }).first().click();
  }
  await lp.locator('#amt').waitFor({ timeout: 20000 });
  await lp.locator('#amt').fill('0.1'); await lp.waitForTimeout(1200);
  const before = (await toastsOf(lp)).split('success').length;
  await lp.locator('#go').click();
  const end = Date.now() + 60000;
  while (Date.now() < end && (await toastsOf(lp)).split('success').length <= before) await lp.waitForTimeout(1000);
  console.log((await toastsOf(lp)).split('success').length > before ? 'OK launchpad curve buy' : 'FAIL launchpad curve buy');
  console.log('launchpad toasts:', (await toastsOf(lp)).slice(0, 300));
  await lp.screenshot({ path: `${SHOTS}flows-launchpad.png` });
  console.log('approved:', approved.join(', '));
} catch (e) { console.error('FAIL', e.stack || e); process.exitCode = 1; }
finally { await ctx.close(); }
