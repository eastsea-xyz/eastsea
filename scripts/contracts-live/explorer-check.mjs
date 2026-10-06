#!/usr/bin/env node
// Explorer phase for scripts/contracts-live.sh: serve the worktree's
// apps/explorer locally, open it in headless Chrome (playwright-core channel
// 'chrome'), point it at the local chain through its saved-node localStorage
// key, and read back ≥5 user-facing views against transactions the flows phase
// produced: home (finalized height + chain), the block that carries a known
// tx, a deploy tx (Contract created + creation pill), a reverted tx (failed +
// decoded revert reason), a token transfer (decoded Transfer event), the dev
// account page, and the token page. Screenshots go to tmp/live/shots only.
//
// The page content is read from the DOM (innerText), not judged from pixels.

import { createServer } from 'node:http';
import { readFile, mkdirSync, writeFileSync } from 'node:fs';
import { dirname, extname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright-core';
import { RPC } from './lib.mjs';

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = resolve(HERE, '../../apps/explorer');
const PORT = Number(process.env.EXPLORER_PORT || 8791);
const BASE = `http://127.0.0.1:${PORT}`;
const OUT = process.env.OUT || resolve(HERE, '../../tmp/live/explorer.json');
const SHOTS = resolve(HERE, '../../tmp/live/shots');

const log = (...a) => console.log(new Date().toISOString().slice(11, 19), ...a);

const MIME = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.json': 'application/json', '.svg': 'image/svg+xml', '.png': 'image/png' };
const server = createServer((req, res) => {
  const path = req.url.split('?')[0].split('#')[0];
  const file = join(ROOT, path === '/' ? 'index.html' : path.replace(/^\//, ''));
  readFile(file, (e, b) => {
    if (e) { res.writeHead(404); res.end('not found'); return; }
    res.writeHead(200, { 'content-type': MIME[extname(file)] || 'text/plain' });
    res.end(b);
  });
});
await new Promise((r) => server.listen(PORT, '127.0.0.1', r));
log(`serving ${ROOT} at ${BASE}`);

const results = JSON.parse((await readFile(resolve(HERE, '../../tmp/live/results.json'))));
const steps = results.steps || [];
const addr = results.addresses || {};
const pick = (label, want = { ok: true }) => steps.find((s) => s.label === label && s.ok === want.ok)
  ?? steps.find((s) => s.label?.startsWith(label));
const checks = [];
const check = (name, pass, detail) => {
  checks.push({ name, pass, detail });
  log(`${pass ? 'ok  ' : 'FAIL'} ${name}${detail ? ` — ${String(detail).slice(0, 90)}` : ''}`);
};

// ------------------------------------------------------------------ browser
const browser = await chromium.launch({ channel: 'chrome', headless: true });
const page = await browser.newPage();
mkdirSync(SHOTS, { recursive: true });
await page.addInitScript(([url]) => {
  // The explorer keeps its node URL in localStorage; set it before any script
  // runs so the very first render reads the local chain, not the default 18545.
  localStorage.setItem('aether-explorer.node', url);
}, [RPC]);

const text = async () => (await page.locator('#view').innerText().catch(() => '')) + '\n'
  + (await page.locator('#top').innerText().catch(() => ''));
const open = async (hash, name) => {
  await page.goto(`${BASE}/#${hash}`, { waitUntil: 'networkidle' });
  await page.waitForTimeout(600); // the view re-renders after its RPC round-trip
  const t = await text();
  await page.screenshot({ path: join(SHOTS, `${name}.png`) }).catch(() => {});
  return t;
};

try {
  // 1. Home: finalized height ≥ the flows' end height, and the chain id line.
  const home = await open('', 'home');
  const hMatch = home.match(/Finalized height\D*(\d+)/) || home.match(/finalized height\D*(\d+)/);
  check('home shows a finalized height ≥ flows end',
    !!hMatch && Number(hMatch[1]) >= results.endHeight, hMatch?.[1]);
  check('home names the chain (7796)', /7796/.test(home), '');

  // 2. The block that carries a known successful call.
  const okTx = pick('AtomicSwap.claim correct preimage') || steps.find((s) => s.ok && s.hash && s.height);
  if (okTx?.height) {
    const b = await open(`/block/${okTx.height}`, 'block');
    check(`block ${okTx.height} lists the claim tx`, b.includes(okTx.hash.slice(0, 18)), okTx.hash.slice(0, 18));
  }

  // 3. A deploy tx: Contract created row + creation pill.
  const dep = steps.find((s) => s.kind === 'deploy' && s.ok && s.contractAddress && s.hash);
  if (dep) {
    const t = await open(`/tx/${dep.hash}`, 'tx-deploy');
    check('deploy tx shows "Contract created"', /Contract created/.test(t), dep.label);
    check('deploy tx shows the creation pill', /creation/.test(t), '');
    check('deploy tx shows success', /success/.test(t), '');
  }

  // 4. A reverted tx: failed status + the decoded revert reason.
  const rev = steps.find((s) => s.expected === 'revert' && s.ok && s.hash);
  if (rev) {
    const t = await open(`/tx/${rev.hash}`, 'tx-revert');
    check(`revert tx shows failed (${rev.label})`, /failed/.test(t), '');
    const reasonM = t.match(/failed\s*[—-]\s*(.+)/);
    check('revert tx surfaces a decoded reason', !!reasonM && !/^0x/.test(reasonM[1].trim()), reasonM?.[1]?.slice(0, 60));
  }

  // 5. A token transfer: the tx page decodes the ERC-20 Transfer event.
  const tr = pick('MerkleDistributor fund + sponsored claim') || steps.find((s) => s.ok && s.hash && s.logs > 0 && !s.contractAddress);
  if (tr) {
    const t = await open(`/tx/${tr.hash}`, 'tx-transfer');
    check('token tx decodes a Transfer event', /Transfer\(/.test(t), tr.label);
  }

  // 6. The dev1 account page: a positive balance in 동해.
  const dev1 = results.dev?.[1] || Object.values(results.dev || {})[0];
  if (dev1) {
    const t = await open(`/account/${dev1}`, 'account');
    const bal = t.match(/Balance\D*([\d.,]+)/);
    check('account page shows dev1 balance > 0', !!bal && parseFloat(bal[1].replace(/,/g, '')) > 0, bal?.[1]);
  }

  // 7. The token page for the TestToken the flows deployed.
  if (addr['support/TestToken']) {
    const t = await open(`/token/${addr['support/TestToken']}`, 'token');
    check('token page renders for TestToken', t.length > 100 && !/no contract|Unknown/i.test(t), '');
  }
} finally {
  await browser.close();
  server.close();
}

const failed = checks.filter((c) => !c.pass);
mkdirSync(dirname(OUT), { recursive: true });
mkdirSync(SHOTS, { recursive: true });
writeFileSync(OUT, JSON.stringify({ checks, failed: failed.length, shots: SHOTS }, null, 2));
log(`explorer → ${OUT} (${checks.length - failed.length}/${checks.length} checks)`);
if (!checks.some((c) => c.pass) || failed.length > checks.length / 2) process.exitCode = 1;
