// Soak traffic: a steady trickle of transfers, plus a burst every few hours,
// against the testnet, logging one JSON line a minute (sent, confirmed,
// confirmation latency, errors, mempool). Built on the browser extension's own
// wallet code, so it exercises the same path users take.
//   node scripts/soak/traffic.mjs            (run it from launchd, see README)
// Env: AETHER_SOAK_RPC (comma list), AETHER_SOAK_RATE (tx/s, default 2),
//      AETHER_SOAK_BURST (txs, default 1000), AETHER_SOAK_BURST_HOURS (default 6),
//      AETHER_SOAK_LOG (dir, default ~/aether-soak)
import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import { Vault } from '../../apps/extension/src/lib/vault.js';
import { Wallet } from '../../apps/extension/src/lib/wallet.js';
import { Rpc } from '../../apps/extension/src/lib/rpc.js';
import { loadWasm } from '../../apps/extension/test/helpers.mjs';

const env = process.env;
const DIR = env.AETHER_SOAK_LOG || path.join(os.homedir(), 'aether-soak');
const RPCS = (env.AETHER_SOAK_RPC || 'http://127.0.0.1:8601,http://127.0.0.1:8602,http://127.0.0.1:8603,http://127.0.0.1:8604').split(',');
const RATE = Number(env.AETHER_SOAK_RATE || 2);
const BURST = Number(env.AETHER_SOAK_BURST || 1000);
const BURST_MS = Number(env.AETHER_SOAK_BURST_HOURS || 6) * 3600_000;
const SINK = '0x000000000000000000000000000000000000dEaD';
const ONE = 10n ** 18n;
fs.mkdirSync(DIR, { recursive: true });
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const log = (o) => fs.appendFileSync(path.join(DIR, 'traffic.log'), JSON.stringify({ at: new Date().toISOString(), ...o }) + '\n');

/** chrome.storage-like area kept in a 0600 file, so the key survives restarts. */
function fileArea(file) {
  const read = () => { try { return JSON.parse(fs.readFileSync(file, 'utf8')); } catch { return {}; } };
  const write = (d) => fs.writeFileSync(file, JSON.stringify(d), { mode: 0o600 });
  return {
    get: async (k) => read()[k],
    set: async (k, v) => { const d = read(); d[k] = v; write(d); },
    remove: async (k) => { const d = read(); delete d[k]; write(d); },
  };
}
function memArea() { const m = new Map(); return { get: async (k) => m.get(k), set: async (k, v) => { m.set(k, v); }, remove: async (k) => { m.delete(k); } }; }

const wasm = await loadWasm();
const vault = new Vault({ local: fileArea(path.join(DIR, 'traffic-key.json')), session: memArea(), addressOf: wasm.accountAddress, now: () => 0 });
const PASS = 'aether-soak-traffic';
const me = (await vault.exists()) ? await vault.unlock(PASS, 10 ** 9) : await vault.create(PASS);
const rpc = new Rpc(RPCS);
const w = new Wallet({ wasm, rpc, vault });

const stats = { sent: 0, confirmed: 0, failed: 0, errors: 0, capWaits: 0, latencyMs: [] };
async function topUp() {
  if ((await w.balance(me)) >= ONE) return;
  try {
    await w.receipt(await w.faucet(me), { timeoutMs: 60_000 });
    log({ event: 'faucet', balance: (await w.balance(me)).toString() });
  } catch (e) { log({ event: 'faucet-failed', error: e.message }); }
}
async function sendOne(track) {
  try {
    const t0 = Date.now();
    const h = await w.send({ to: SINK, value_wei: '1', data: '0x', gas: 0 });
    stats.sent += 1;
    if (track) w.receipt(h, { timeoutMs: 60_000 }).then((r) => {
      if (r && r.ok) { stats.confirmed += 1; stats.latencyMs.push(Date.now() - t0); } else stats.failed += 1;
    });
  } catch (e) {
    if (/pending transactions/.test(e.message)) { stats.capWaits += 1; await sleep(500); } else { stats.errors += 1; w.lastNonce = null; if (stats.errors % 50 === 1) log({ event: 'error', error: e.message }); await sleep(1000); }
  }
}
async function burst() {
  log({ event: 'burst-start', size: BURST });
  const t0 = Date.now();
  for (let i = 0; i < BURST; i += 1) await sendOne(i % 50 === 0);
  log({ event: 'burst-end', size: BURST, seconds: (Date.now() - t0) / 1000 });
}

log({ event: 'start', account: me, rpcs: RPCS, rate: RATE });
let nextBurst = Date.now() + BURST_MS;
let nextReport = Date.now() + 60_000;
for (;;) {
  await topUp();
  const tick = Date.now();
  for (let i = 0; i < RATE; i += 1) await sendOne(i === 0);
  if (Date.now() >= nextBurst) { await burst(); nextBurst = Date.now() + BURST_MS; }
  if (Date.now() >= nextReport) {
    const st = await rpc.call('aether_status', []).catch(() => ({}));
    const l = stats.latencyMs.sort((a, b) => a - b);
    log({ event: 'minute', sent: stats.sent, confirmed: stats.confirmed, failed: stats.failed, errors: stats.errors, capWaits: stats.capWaits,
      p50ms: l[Math.floor(l.length / 2)] ?? null, p95ms: l[Math.floor(l.length * 0.95)] ?? null, height: st.height, mempool: st.mempool });
    Object.assign(stats, { sent: 0, confirmed: 0, failed: 0, errors: 0, capWaits: 0, latencyMs: [] });
    nextReport = Date.now() + 60_000;
  }
  await sleep(Math.max(0, 1000 - (Date.now() - tick)));
}
