#!/usr/bin/env node
// Three actual local devnet nodes, a local iroh relay and a forged read peer.
// Compilation is separate and gated. All owned processes are stopped in finally.
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { once } from 'node:events';
import { spawn, execFileSync } from 'node:child_process';
import { createWriteStream } from 'node:fs';
import { access, mkdir, readFile, writeFile, stat } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const task = path.join(root, 'tmp', 'p2p-read', `devnet-${Date.now()}-${process.pid}`);
await mkdir(task, { recursive: true });
process.env.TMPDIR = task;
const binary = process.env.AETHER_BIN || path.join(root, 'tmp/p2p-read/target/debug/aether');
const examples = path.join(path.dirname(binary), 'examples');
const networkBinary = process.env.AETHER_READ_NETWORK_BIN || path.join(examples, 'public_read_network');
const forgedBinary = process.env.AETHER_FORGED_READ_BIN || path.join(examples, 'public_read_forged');
const relayBinary = process.env.IROH_RELAY_BIN || path.join(root, 'tmp/p2p-read/relay-target/debug/iroh-relay');
const playwrightPath = process.env.PLAYWRIGHT_MODULE || '/Users/kjaylee/.codex/skills/develop-web-game/node_modules/playwright-core/index.mjs';
for (const file of [binary, networkBinary, forgedBinary, relayBinary, playwrightPath]) await access(file);
const { chromium } = await import(pathToFileURL(playwrightPath).href);
const chrome = process.env.CHROME_PATH || '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';
const network = JSON.parse(execFileSync(networkBinary, ['3'], { cwd: root, encoding: 'utf8', timeout: 60_000 }));
const children = []; const logs = []; let server; let browser;
const ports = { relay: 19440, p2p: 19100, rpc: 19500 };
const relay = `http://127.0.0.1:${ports.relay}/`;
const environment = { ...process.env, TMPDIR: task, AETHER_IROH_NO_DHT: '1', AETHER_IROH_RELAY_URL: relay,
  RUST_LOG: 'warn,aether_node=info', CARGO_BUILD_JOBS: '4' };
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
let abort = false;
for (const signal of ['SIGINT', 'SIGTERM']) process.on(signal, () => { abort = true; });

function start(label, executable, args) {
  const log = createWriteStream(path.join(task, `${label}.log`)); logs.push(log);
  const child = spawn(executable, args, { cwd: root, env: environment, stdio: ['ignore', 'pipe', 'pipe'] });
  child.stdout.pipe(log); child.stderr.pipe(log); children.push(child);
  return child;
}
async function waitFor(check, within = 60_000) {
  const deadline = Date.now() + within;
  let last;
  while (Date.now() < deadline && !abort) {
    for (const child of children) if (child.exitCode !== null || child.signalCode !== null) throw new Error(`owned child ${child.pid} exited early; logs: ${task}`);
    try { const value = await check(); if (value) return value; } catch (error) { last = error; }
    await sleep(250);
  }
  throw new Error(`devnet did not become ready: ${last?.message || 'timeout'}; logs: ${task}`);
}
async function rpc(port, method, params = []) {
  const response = await fetch(`http://127.0.0.1:${port}`, { method: 'POST',
    headers: { 'content-type': 'application/json' }, body: JSON.stringify({ jsonrpc: '2.0', id: 1, method, params }),
    signal: AbortSignal.timeout(2_000) });
  const body = await response.json(); if (body.error) throw new Error(body.error.message); return body.result;
}

async function openPage(origin, attempted, pageErrors) {
  browser = await chromium.launch({ executablePath: chrome, headless: true, env: environment });
  const context = await browser.newContext();
  await context.route('**/*', async route => {
    const u = new URL(route.request().url()); attempted.add(`${u.protocol}//${u.host}`);
    if (u.origin === origin) return route.continue();
    return route.abort('blockedbyclient');
  });
  await context.routeWebSocket('**/*', socket => {
    const u = new URL(socket.url()); attempted.add(`${u.protocol}//${u.host}`);
    if (u.host === new URL(relay).host) socket.connectToServer();
    else socket.close({ code: 1008, reason: 'Only the local test relay is allowed' });
  });
  await context.addInitScript(url => {
    localStorage.setItem('aether-explorer.relays', JSON.stringify([url]));
    localStorage.setItem('aether-explorer.gateway', '');
  }, relay);
  const page = await context.newPage();
  page.on('pageerror', error => pageErrors.push(error.message));
  return { context, page };
}

try {
  const config = path.join(task, 'relay.toml');
  await writeFile(config, `http_bind_addr = "127.0.0.1:${ports.relay}"\nenable_metrics = false\nenable_quic_addr_discovery = false\n`);
  start('relay', relayBinary, ['--dev', '--config-path', config]);
  await waitFor(async () => (await fetch(relay, { signal: AbortSignal.timeout(1000) })).ok);
  for (let i = 1; i <= 3; i++) {
    const peers = [1, 2, 3].filter(j => i !== j).map(j => `${j}@127.0.0.1:${ports.p2p + j}`).join(',');
    start(`node${i}`, binary, ['node', '--index', String(i), '--validators', '3', '--port', String(ports.p2p + i),
      '--rpc-port', String(ports.rpc + i), '--data', path.join(task, `node${i}`), '--peers', peers, '--block-time-ms', '500']);
  }
  await waitFor(async () => {
    const statuses = await Promise.all([1, 2, 3].map(i => rpc(ports.rpc + i, 'aether_status')));
    return statuses.every(s => s.height >= 8) && statuses;
  }, 120_000);
  const hintFile = path.join(task, 'forged-peer.json');
  start('forged-peer', forgedBinary, ['--rpc', `http://127.0.0.1:${ports.rpc + 1}`, '--hint', hintFile]);
  const forged = await waitFor(async () => JSON.parse(await readFile(hintFile, 'utf8')));
  const peers = network.validators.map((v, i) => ({ node: v.node, relay, operator: `local-${i}` }));
  const seeds = { chain_id: network.chain_id, peers };
  const appRoot = path.join(root, 'apps/explorer');
  server = createServer(async (request, response) => {
    try {
      const pathname = new URL(request.url, 'http://127.0.0.1').pathname;
      response.setHeader('Cache-Control', 'no-store');
      response.setHeader('Cross-Origin-Opener-Policy', 'same-origin');
      response.setHeader('Cross-Origin-Embedder-Policy', 'require-corp');
      if (pathname === '/network.json' || pathname === '/public-read-peers.json') {
        response.setHeader('Content-Type', 'application/json');
        response.end(JSON.stringify(pathname === '/network.json' ? network : seeds)); return;
      }
      const file = path.resolve(appRoot, `.${decodeURIComponent(pathname === '/' ? '/index.html' : pathname)}`);
      if (!file.startsWith(`${appRoot}${path.sep}`)) { response.writeHead(403); response.end(); return; }
      response.setHeader('Content-Type', { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.wasm': 'application/wasm', '.json': 'application/json' }[path.extname(file)] || 'application/octet-stream');
      response.end(await readFile(file));
    } catch { response.writeHead(404); response.end(); }
  });
  server.listen(0, '127.0.0.1'); await once(server, 'listening');
  const origin = `http://127.0.0.1:${server.address().port}`;
  const samples = []; const attempted = new Set(); const pageErrors = [];
  const runs = Number(process.env.AETHER_READ_COLD_RUNS || 3);
  assert.ok(Number.isInteger(runs) && runs >= 1 && runs <= 5);
  for (let run = 0; run < runs; run++) {
    const { context, page } = await openPage(origin, attempted, pageErrors);
    await page.goto(origin, { waitUntil: 'domcontentloaded' });
    await page.waitForFunction(() => window.aetherReadDiagnostics?.().source?.kind === 'peers'
      && window.aetherReadDiagnostics().livePeers.length >= 3
      && /Finalized height/.test(document.getElementById('view')?.textContent || ''), null, { timeout: 90_000 });
    const head = await page.evaluate(() => ({ totalMs: performance.now(), diagnostics: window.aetherReadDiagnostics(),
      sourceText: document.getElementById('source').textContent }));
    assert.match(head.sourceText, /verified|peers/i);
    const height = Number(await rpc(ports.rpc + 1, 'eth_blockNumber').then(n => parseInt(n, 16))) - 2;
    const began = Date.now();
    await page.evaluate(h => { location.hash = `#/block/${h}`; }, height);
    await page.waitForFunction(() => /verified by committee certificate/.test(document.getElementById('view')?.textContent || ''), null, { timeout: 30_000 });
    const warmBlockMs = Date.now() - began;
    await page.screenshot({ path: path.join(task, `block-${run}.png`), fullPage: true });
    const sample = { coldVerifiedHeadMs: head.diagnostics.firstVerifiedHeadAt, homePageMs: head.totalMs,
      warmBlockPageMs: warmBlockMs, ...head.diagnostics.metrics };
    assert.ok(Number.isFinite(sample.coldVerifiedHeadMs) && sample.coldVerifiedHeadMs >= 0);
    if (run === 0) {
      const rejection = await page.evaluate(async ({ network, peers, forged, relay }) => {
        const mod = await import('./wasm/aether_wasm.js'); await mod.default();
        const { PublicPeerPool } = await import('./js/peers.js');
        const transport = await mod.PublicReadTransport.create(JSON.stringify([relay]), JSON.stringify([relay]));
        const dropped = [];
        const pool = new PublicPeerPool({ network, peers: [{ ...forged, operator: 'forged-test' }, ...peers],
          mod, transport, onPeer: event => dropped.push(event) });
        try {
          const head = await pool.call('aether_status');
          return { head, dropped, live: pool.livePeers, metrics: pool.metrics };
        } finally { await pool.close(); }
      }, { network, peers, forged, relay });
      assert.equal(rejection.head.verified, true);
      assert.ok(rejection.dropped.some(p => p.node === forged.node && p.dropped), 'forged peer was not rejected');
      assert.ok(!rejection.live.some(p => p.node === forged.node), 'forged peer remained live');
      await writeFile(path.join(task, 'forged-rejection.json'), JSON.stringify(rejection, null, 2));
    }
    await context.close(); await browser.close(); browser = null;
    // Measure a direct block URL with a new browser process and empty caches.
    const cold = await openPage(origin, attempted, pageErrors);
    await cold.page.goto(`${origin}/#/block/${height}`, { waitUntil: 'domcontentloaded' });
    await cold.page.waitForFunction(() => window.aetherReadDiagnostics?.().source?.kind === 'peers'
      && window.aetherReadDiagnostics().livePeers.length >= 3
      && /verified by committee certificate/.test(document.getElementById('view')?.textContent || ''), null, { timeout: 90_000 });
    sample.coldBlockPageMs = await cold.page.evaluate(() => performance.now());
    await cold.page.screenshot({ path: path.join(task, `cold-block-${run}.png`), fullPage: true });
    samples.push(sample);
    await cold.context.close(); await browser.close(); browser = null;
  }
  assert.deepEqual(pageErrors, []);
  assert.ok(![...attempted].some(host => host.includes('rpc.eastsea.xyz')));
  const wasm = await stat(path.join(appRoot, 'wasm/aether_wasm_bg.wasm'));
  const result = { network: { chain_id: network.chain_id, validators: 3 }, peers: 3, samples, wasmBytes: wasm.size,
    browser: 'headless Google Chrome', relay, attemptedHttpOrigins: [...attempted], logs: task };
  await writeFile(path.join(task, 'result.json'), JSON.stringify(result, null, 2));
  await writeFile(path.join(root, 'tmp/p2p-read/browser-result.json'), JSON.stringify(result, null, 2));
  console.log(JSON.stringify(result, null, 2));
} finally {
  await browser?.close();
  if (server) await new Promise(resolve => server.close(resolve));
  for (const child of children) if (child.exitCode === null && child.signalCode === null) child.kill('SIGINT');
  await Promise.all(children.map(async child => {
    if (child.exitCode !== null || child.signalCode !== null) return;
    await Promise.race([once(child, 'exit'), sleep(5000)]);
    if (child.exitCode === null && child.signalCode === null) { child.kill('SIGKILL'); await once(child, 'exit'); }
  }));
  for (const log of logs) log.end();
  await writeFile(path.join(task, 'stopped.json'), JSON.stringify(children.map(c => ({ pid: c.pid, exitCode: c.exitCode, signal: c.signalCode })), null, 2));
}
