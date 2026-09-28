// Browser end-to-end check: loads this extension into Playwright's Chromium and
// runs create -> faucet -> connect -> reject -> contract call with value -> disconnect
// against a running testnet (this Mac's validators or the app's node).
//   npm i --no-save playwright && npx playwright install chromium
//   scripts/build-extension.sh && node apps/extension/test/e2e.mjs
import { chromium } from 'playwright';
import http from 'node:http';
import fs from 'node:fs';

const EXT = new URL('..', import.meta.url).pathname.replace(/\/$/, '');
const SHOTS = process.env.SHOTS || '/tmp/aether-extension-e2e/';
fs.mkdirSync(SHOTS, { recursive: true });
const PAGE = '<!doctype html><meta charset="utf-8"><title>dapp test</title>\n<pre id="log">loading</pre>\n<script>\nconst log = (...a) => { document.getElementById(\'log\').textContent += \'\\n\' + a.join(\' \'); };\nwindow.results = {};\nwindow.addEventListener(\'load\', () => log(\'aether present:\', !!window.aether, \'isAether:\', window.aether && window.aether.isAether));\nwindow.announced = [];\nwindow.addEventListener(\'eip6963:announceProvider\', (e) => window.announced.push(e.detail.info.rdns));\nwindow.dispatchEvent(new Event(\'eip6963:requestProvider\'));\n</script>\n';
const server = http.createServer((q, r) => { r.setHeader('content-type', 'text/html'); r.end(PAGE); }).listen(8391);
const WAETH = '0xa2521982a17474cb2f8741c85de653b5282d72b0';

const ctx = await chromium.launchPersistentContext('', {
  channel: 'chromium', headless: true,
  args: [`--disable-extensions-except=${EXT}`, `--load-extension=${EXT}`],
});
const fail = (m) => { console.error('FAIL', m); process.exitCode = 1; };
try {
  let [sw] = ctx.serviceWorkers();
  sw ??= await ctx.waitForEvent('serviceworker');
  const id = new URL(sw.url()).host;
  sw.on('console', (m) => console.log('[sw]', m.text()));
  console.log('extension', id);

  const popup = await ctx.newPage();
  await popup.setViewportSize({ width: 360, height: 600 });
  await popup.goto(`chrome-extension://${id}/ui/popup.html`);
  // First run: the one-time notice comes before the onboarding.
  await popup.getByText('Before you use Aether').waitFor();
  await popup.screenshot({ path: SHOTS + '0-notice.png' });
  await popup.getByRole('button', { name: 'I understand' }).click();
  await popup.getByText('Create a wallet').waitFor();
  await popup.screenshot({ path: SHOTS + '1-onboarding.png' });
  const pws = popup.locator('input[type=password]');
  await pws.nth(0).fill('e2e-password-1');
  await pws.nth(1).fill('e2e-password-1');
  await popup.getByRole('button', { name: 'Create wallet' }).click();
  await popup.getByText('Block ').waitFor({ timeout: 20000 });
  await popup.getByRole('button', { name: /Get test AETH/ }).click();
  await popup.locator('.msg').first().waitFor({ timeout: 20000 });
  console.log('faucet msg:', await popup.locator('.msg').first().innerText());
  await popup.waitForTimeout(3500);
  await popup.reload();
  await popup.getByText(/10 AETH/).waitFor({ timeout: 20000 });
  await popup.screenshot({ path: SHOTS + '2-home.png' });

  const page = await ctx.newPage();
  await page.goto('http://localhost:8391/');
  const present = await page.evaluate(() => ({ aether: !!window.aether?.isAether, announced: window.announced, chain: null }));
  console.log('page sees', JSON.stringify(present));
  if (!present.aether || !present.announced.includes('com.pipln.aether')) fail('provider not injected/announced');
  console.log('chainId', await page.evaluate(() => window.aether.request({ method: 'eth_chainId' })));
  console.log('accounts before', JSON.stringify(await page.evaluate(() => window.aether.request({ method: 'eth_accounts' }))));
  const notConnected = await page.evaluate(() => window.aether.request({ method: 'eth_sendTransaction', params: [{ to: '0x000000000000000000000000000000000000dEaD' }] }).then(() => 'sent', (e) => e.code));
  if (notConnected !== 4100) fail('send before connect should be 4100, got ' + notConnected);

  // Connect: an approval window opens.
  const winP = ctx.waitForEvent('page');
  const accountsP = page.evaluate(() => window.aether.request({ method: 'eth_requestAccounts' }));
  const win = await winP;
  await win.waitForLoadState();
  await win.getByRole('button', { name: 'Connect' }).waitFor();
  await win.screenshot({ path: SHOTS + '3-connect.png' });
  await win.getByRole('button', { name: 'Connect' }).click();
  const accounts = await accountsP;
  console.log('connected', accounts);

  // Rejection path.
  const rejWinP = ctx.waitForEvent('page');
  const rejP = page.evaluate(() => window.aether.request({ method: 'eth_sendTransaction', params: [{ to: '0x000000000000000000000000000000000000dEaD', value: '0x1' }] }).then(() => 'sent', (e) => e.code));
  const rejWin = await rejWinP;
  await rejWin.getByRole('button', { name: 'Reject' }).click();
  const rej = await rejP;
  if (rej !== 4001) fail('reject should be 4001, got ' + rej);
  console.log('rejected ->', rej);

  // A contract call with value: WAETH.deposit(0.25 AETH).
  const sendWinP = ctx.waitForEvent('page');
  const hashP = page.evaluate((to) => window.aether.request({ method: 'eth_sendTransaction', params: [{ to, value: '0x3782dace9d90000', data: '0xd0e30db0' }] }), WAETH);
  const sendWin = await sendWinP;
  await sendWin.getByText('Wrap AETH').waitFor();
  await sendWin.getByText(/up to/).waitFor({ timeout: 10000 }).catch(() => {});
  await sendWin.screenshot({ path: SHOTS + '4-approve-call.png' });
  await sendWin.getByRole('button', { name: 'Approve' }).click();
  const hash = await hashP;
  console.log('tx', hash);
  const receipt = await page.evaluate(async (h) => {
    for (let i = 0; i < 80; i += 1) {
      const r = await window.aether.request({ method: 'aether_getReceipt', params: [h] });
      if (r && r.receipt) return r;
      await new Promise((res) => setTimeout(res, 500));
    }
    return null;
  }, hash);
  console.log('receipt', JSON.stringify(receipt?.receipt), 'height', receipt?.height);
  if (!receipt?.receipt?.success) fail('WAETH deposit did not succeed');
  const bal = await page.evaluate(([to, a]) => window.aether.request({ method: 'eth_call', params: [{ to, data: '0x70a08231' + a.slice(2).toLowerCase().padStart(64, '0') }, 'latest'] }), [WAETH, accounts[0]]);
  console.log('WAETH balance wei', BigInt(bal).toString());

  // Popup shows activity and the connected site.
  await popup.reload();
  await popup.getByRole('button', { name: 'Activity' }).click();
  await popup.getByText('Wrap AETH').waitFor({ timeout: 20000 });
  await popup.waitForTimeout(1500);
  await popup.getByRole('button', { name: 'Activity' }).click();
  await popup.screenshot({ path: SHOTS + '5-activity.png' });
  await popup.getByRole('button', { name: 'Sites' }).click();
  await popup.getByText('http://localhost:8391').waitFor();
  await popup.screenshot({ path: SHOTS + '6-sites.png' });

  // Disconnect from the page side -> accountsChanged([]).
  const changed = page.evaluate(() => new Promise((res) => window.aether.on('accountsChanged', res)));
  await page.evaluate(() => window.aether.request({ method: 'wallet_disconnect' }));
  console.log('accountsChanged ->', JSON.stringify(await changed));
  console.log(process.exitCode ? 'E2E FAILED' : 'E2E PASSED');
} catch (e) {
  fail(e.stack || e);
  for (const [i, pg] of ctx.pages().entries()) { await pg.screenshot({ path: SHOTS + `fail-${i}.png` }).catch(() => {}); console.log('page', i, pg.url(), (await pg.locator('body').innerText().catch(() => '')).slice(0, 300)); }
} finally {
  await ctx.close();
  server.close();
}
