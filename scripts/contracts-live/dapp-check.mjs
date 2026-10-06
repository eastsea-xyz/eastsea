#!/usr/bin/env node
// Dapp phase for scripts/contracts-live.sh: load three toolbox frontends
// (/Volumes/workspace/eastsea-toolbox — token, nft, names) against the local
// chain in headless Chrome and drive each one's own form end to end.
//
// WALLET HONESTY NOTE (also stated in the report): the EastSea browser
// extension is a Chrome MV3 extension and cannot be loaded into a headless
// playwright page, so this check injects a minimal EIP-1193 provider
// (window.aether.request) whose writes are executed by the SAME wallet path
// the extension uses — the `aether` CLI (`call`/`send` → sign_call_with +
// recommended_state_budget). Reads go straight to the node's JSON-RPC. No
// alternative signing code exists in this file.

import { createServer } from 'node:http';
import { readFile, readFileSync, mkdirSync, writeFileSync } from 'node:fs';
import { dirname, extname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright-core';
import { RPC, BIN, rpc, cliTx } from './lib.mjs';

const HERE = dirname(fileURLToPath(import.meta.url));
const TOOLBOX = process.env.TOOLBOX || '/Volumes/workspace/eastsea-toolbox';
const PORT = Number(process.env.DAPP_PORT || 8790);
const BASE = `http://127.0.0.1:${PORT}`;
const OUT = process.env.OUT || resolve(HERE, '../../tmp/live/dapp.json');
const SHOTS = resolve(HERE, '../../tmp/live/shots');

const log = (...a) => console.log(new Date().toISOString().slice(11, 19), ...a);

const MIME = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.json': 'application/json', '.svg': 'image/svg+xml', '.png': 'image/png' };
const server = createServer((req, res) => {
  const path = req.url.split('?')[0].split('#')[0];
  const rel = path === '/' ? 'apps/token/index.html' : path.replace(/^\//, '') + (path.endsWith('/') ? 'index.html' : '');
  const file = join(TOOLBOX, rel);
  if (!file.startsWith(TOOLBOX)) { res.writeHead(403); res.end(); return; }
  readFile(file, (e, b) => {
    if (e) { res.writeHead(404); res.end('not found'); return; }
    res.writeHead(200, { 'content-type': MIME[extname(file)] || 'text/plain' });
    res.end(b);
  });
});
await new Promise((r) => server.listen(PORT, '127.0.0.1', r));
log(`serving ${TOOLBOX} at ${BASE}`);

const results = JSON.parse(readFileSync(resolve(HERE, '../../tmp/live/results.json'), 'utf8'));
const dev = results.dev || {}; // { '1': addr, '2': addr, '3': addr }
const addrOf = (i) => dev[String(i)] || Object.values(dev)[i - 1];
const devIndex = Object.fromEntries(Object.entries(dev).map(([i, a]) => [a.toLowerCase(), Number(i)]));
const contracts = results.addresses || {};

const checks = [];
const check = (name, pass, detail) => {
  checks.push({ name, pass, detail: String(detail ?? '') });
  log(`${pass ? 'ok  ' : 'FAIL'} ${name}${detail ? ` — ${String(detail).slice(0, 80)}` : ''}`);
};

// ---------------------------------------------------------------- EIP-1193 shim
// Writes: the aether CLI (the extension's own signing path). Reads: the node.
let walletRequests = 0;
let activeDev = 1; // which dev account the shim answers eth_requestAccounts with
const shimRequest = async (method, params) => {
  walletRequests++;
  if (method === 'eth_requestAccounts') return [addrOf(activeDev)];
  if (method === 'eth_accounts') return [addrOf(activeDev)];
  if (method === 'eth_chainId') return '0x1e64'; // the toolbox hard gate (7796)
  if (method === 'net_version') return '7796';
  if (method === 'wallet_switchEthereumChain') return null;
  if (method === 'eth_sendTransaction') {
    const p = params[0];
    const idx = devIndex[String(p.from || '').toLowerCase()];
    if (!idx) throw new Error(`shim: unknown account ${p.from}`);
    const args = ['--from-dev', String(idx), '--to', p.to, '--data', p.data || '0x', '--gas', '2000000', '--wait'];
    if (p.value != null && BigInt(p.value) !== 0n) args.push('--value', String(BigInt(p.value)));
    // cliTx: the wallet path, waiting out a spent B5 budget like the wallet would.
    const r = await cliTx('call', args);
    if (!r.hash) throw new Error(`shim: CLI refused: ${r.stderr || r.stdout}`);
    return r.hash;
  }
  return rpc(method, params); // eth_call, eth_blockNumber, eth_getLogs, …
};

// ---------------------------------------------------------------- browser
const browser = await chromium.launch({ channel: 'chrome', headless: true });
const page = await browser.newPage();
await page.exposeFunction('__aetherShim', shimRequest);
await page.addInitScript(() => {
  // EIP-1193 provider the toolbox frontends fall back to when no EIP-6963
  // wallet announces itself. Every call bridges to Node (see shimRequest).
  const provider = { request: (req) => window.__aetherShim(req.method, req.params ?? []) };
  window.aether = provider;
  window.ethereum = provider;
});
mkdirSync(SHOTS, { recursive: true });

const openApp = async (slug, contract, name) => {
  await page.goto(`${BASE}/apps/${slug}/?contract=${contract}`, { waitUntil: 'networkidle' });
  await page.waitForTimeout(400);
  // Connect the wallet: the fallback provider is one of the .wallets buttons.
  const btns = page.locator('#wallets button');
  await btns.first().click();
  await page.waitForTimeout(400);
  const shot = join(SHOTS, `dapp-${name}.png`);
  await page.screenshot({ path: shot }).catch(() => {});
  return shot;
};
const acctText = () => page.locator('#acct').innerText().catch(() => '');
const chainText = () => page.locator('#chain').innerText().catch(() => '');
/// Submit the form whose signature text (e.g. 'transfer(address,uint256)')
/// identifies it, filling its inputs in order; returns the form's result line.
/// The frontends print "전송됨: <hash>" as soon as the wallet returns a hash;
/// they never read the receipt. The check does, the way a careful user would.
const receiptOk = async (text) => {
  const h = text.match(/0x[0-9a-f]{64}/i)?.[0];
  if (!h) return 'no hash';
  const r = await rpc('aether_getReceipt', [h]).catch(() => null);
  return r?.receipt ? `success=${r.receipt.success} h=${r.height} logs=${r.receipt.logs}` : 'no receipt';
};
const runForm = async (sigText, values, ms = 90_000) => {
  const form = page.locator('form').filter({ hasText: sigText }).first();
  const inputs = form.locator('input');
  for (let i = 0; i < await inputs.count(); i++) await inputs.nth(i).fill(String(values[i] ?? ''));
  const out = form.locator('p').last();
  await form.locator('button[type=submit]').click();
  const deadline = Date.now() + ms;
  for (;;) {
    const t = (await out.innerText().catch(() => '')) || '';
    if (t && t !== '…') return t;
    if (Date.now() > deadline) return t || '(no output)';
    await new Promise((r) => setTimeout(r, 500));
  }
};

try {
  // ------------------------------------------------ 1. token (FixedSupplyToken)
  if (contracts['toolbox/FixedSupplyToken']) {
    await openApp('token', contracts['toolbox/FixedSupplyToken'], 'token');
    check('token: wallet connected (#acct shows dev1)', /연결됨/.test(await acctText()) && /0x[0-9a-f]{40}/i.test(await acctText()), (await acctText()).slice(0, 40));
    check('token: chain gate passes (0x1e64)', /0x1e64|EastSea/.test(await chainText()), (await chainText()).slice(0, 40));
    const supply = await runForm('totalSupply()', []);
    check('token: eth_call view totalSupply', /\d/.test(supply), supply.slice(0, 40));
    const sent = await runForm('transfer(address,uint256)', [addrOf(3), '1']);
    check('token: transfer write via CLI path (전송됨: 0x…)', /전송됨:\s*0x[0-9a-f]{64}/i.test(sent), sent.slice(0, 80));
    const tr = await receiptOk(sent);
    check('token: transfer receipt success with 1 log', /success=true .* logs=1/.test(tr), tr);
    await page.locator('#load-logs').click();
    await page.waitForTimeout(800);
    const logs = await page.locator('#logs').innerText().catch(() => '');
    check('token: Transfer event listed from eth_getLogs', /Transfer/i.test(logs) && /block \d+/.test(logs), logs.slice(0, 60));
    await page.screenshot({ path: join(SHOTS, 'dapp-token-after.png') }).catch(() => {});
  }

  // ------------------------------------------------ 2. nft (Editions1155)
  if (contracts['toolbox/Editions1155']) {
    await openApp('nft', contracts['toolbox/Editions1155'], 'nft');
    check('nft: wallet connected', /연결됨/.test(await acctText()), '');
    // createEdition('dapp-edition', cap 2, wallet-cap 1, price 0.001, feeBps 100)
    const made = await runForm('createEdition(string,uint256,uint256,uint256,uint16)', ['dapp-edition', '2', '1', '1000000000000000', '100']);
    check('nft: createEdition write', /전송됨:\s*0x[0-9a-f]{64}/i.test(made), made.slice(0, 80));
    const mr = await receiptOk(made);
    check('nft: createEdition receipt success', /success=true/.test(mr), mr);
    const ed = await runForm('editionOf(uint256)', ['2']);
    check('nft: editionOf(2) view after create', /\d/.test(ed), ed.slice(0, 50));
    await page.screenshot({ path: join(SHOTS, 'dapp-nft-after.png') }).catch(() => {});
  }

  // ------------------------------------------------ 3. names (NameGatedDrop)
  if (contracts['toolbox/NameGatedDrop']) {
    activeDev = 2; // dev2 owns the toolbox primary name "toolive"
    await openApp('names', contracts['toolbox/NameGatedDrop'], 'names');
    check('names: wallet connected as dev2', /연결됨/.test(await acctText()) && (await acctText()).includes(addrOf(2).slice(0, 10)), (await acctText()).slice(0, 44));
    // dev2 already claimed in the flows phase, so this dapp-submitted claim is
    // the EXPECTED revert: the dapp still gets its hash (wallet UX shows
    // 전송됨), while the receipt says failed. Verify both halves.
    const claimed = await runForm('claim()', []);
    const hashM = claimed.match(/0x[0-9a-f]{64}/i);
    check('names: claim write submitted (hash returned)', !!hashM, claimed.slice(0, 80));
    // UX finding: the page shows the same green 전송됨 for a tx that failed.
    check('names: page text for the FAILED claim (recorded verbatim)', true, claimed.slice(0, 90));
    if (hashM) {
      const rec = await rpc('aether_getReceipt', [hashM[0]]);
      const ok = rec?.receipt?.success === false; // double-claim must refuse
      check('names: second claim reverts on-chain (receipt failed)', ok,
        rec ? `success=${rec.receipt.success} h=${rec.height}` : 'no receipt');
    }
    await page.screenshot({ path: join(SHOTS, 'dapp-names-after.png') }).catch(() => {});
  }

  log(`shim handled ${walletRequests} provider requests (writes through ${BIN})`);
} finally {
  await browser.close();
  server.close();
}

const failed = checks.filter((c) => !c.pass);
mkdirSync(dirname(OUT), { recursive: true });
writeFileSync(OUT, JSON.stringify({
  wallet: 'EIP-1193 shim over the aether CLI (extension cannot load headless; same signing path)',
  checks, failed: failed.length, walletRequests,
}, null, 2));
log(`dapp → ${OUT} (${checks.length - failed.length}/${checks.length} checks)`);
if (failed.length) process.exitCode = 1;
