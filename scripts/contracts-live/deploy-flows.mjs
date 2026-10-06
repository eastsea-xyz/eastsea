#!/usr/bin/env node
// Runner for the live contract pass (scripts/contracts-live.sh phase "flows").
//
// Deploys every fixture in crates/contracts-onchain/fixtures/artifacts.json
// through the aether CLI wallet path, walks each contract's main user flow,
// then reads the receipts and logs back over the node's JSON-RPC. Writes a
// uniform per-step record list to $OUT (default tmp/live/results.json) for the
// explorer/dapp/stress checks and the final report.
//
// Environment:
//   AETHER_RPC         node JSON-RPC (default http://127.0.0.1:8645)
//   AETHER_BIN         aether binary (default "aether")
//   AETHER_CHAIN_ID    expected chain id; skips flows if mismatched
//   AETHER_ARTIFACTS   artifacts.json (default the repo fixtures copy)
//   OUT                results path (default tmp/live/results.json)

import { readFileSync, writeFileSync, mkdirSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { rpc, height, status, devAddress, send, getLogs, sleep } from './lib.mjs';
import { runFlows } from './flows.mjs';

const HERE = dirname(fileURLToPath(import.meta.url));
const ARTIFACTS = process.env.AETHER_ARTIFACTS
  || resolve(HERE, '../../crates/contracts-onchain/fixtures/artifacts.json');
const OUT = process.env.OUT || resolve(HERE, '../../tmp/live/results.json');

const log = (...a) => console.log(new Date().toISOString().slice(11, 19), ...a);

async function waitForRpc(tries = 60) {
  for (let i = 0; i < tries; i++) {
    try {
      await rpc('aether_status');
      return;
    } catch {
      await sleep(2000);
    }
  }
  throw new Error(`no RPC at ${process.env.AETHER_RPC || 'http://127.0.0.1:8645'}`);
}

function summaryRow(r) {
  const lat = r.submittedMs != null && r.finalizedMs != null ? `${r.finalizedMs - r.submittedMs}ms` : '-';
  return [
    r.kind.padEnd(7),
    (r.ok === true ? 'ok' : r.ok === false ? 'FAIL' : '?').padEnd(4),
    (r.label || '').slice(0, 46).padEnd(46),
    `h=${r.height ?? '-'}`,
    `gas=${r.gas ?? '-'}`,
    `sg=${r.stateGas ?? '-'}`,
    `fee=${r.stateFee ?? '-'}`,
    `lat=${lat}`,
  ].join(' ');
}

async function main() {
  await waitForRpc();
  const st = await status();
  log(`chain ${st.chain_id} height ${st.height} protocol ${st.protocol ?? '?'} — wallet path: ${process.env.AETHER_BIN || 'aether'} CLI`);
  const wantId = Number(process.env.AETHER_CHAIN_ID || 0);
  if (wantId && Number(st.chain_id) !== wantId) throw new Error(`chain id ${st.chain_id} != expected ${wantId}`);

  const art = JSON.parse(readFileSync(ARTIFACTS, 'utf8'));
  log(`artifacts: ${Object.keys(art).length} contracts from ${ARTIFACTS}`);

  const results = [];
  const ctx = { art, log, results, addresses: {} };
  const t0 = performance.now();

  // dev1 holds the genesis faucet supply; dev2/dev3 are funded by plain
  // transfers, which doubles as the native-transfer check.
  const D = { 1: devAddress(1), 2: devAddress(2), 3: devAddress(3) };
  log(`dev1 ${D[1]}\n     dev2 ${D[2]}\n     dev3 ${D[3]}`);
  for (const i of [2, 3]) {
    const r = await send({ dev: 1, to: D[i], value: 10n ** 24n });
    results.push({ kind: 'transfer', ...r, label: `fund dev${i}` });
    if (!r.ok || r.success !== true) throw new Error(`funding dev${i} failed`);
  }

  await runFlows(ctx);

  // Log visibility over eth_getLogs: every deployed address that should have
  // emitted something, checked in one pass over the full range.
  const deployed = Object.entries(ctx.deployed || {}).filter(([, a]) => typeof a === 'string' && a.startsWith('0x'));
  const latest = await height();
  const logCounts = {};
  // The node scans at most the newest 2,000 blocks per eth_getLogs query
  // (crates/node/src/rpc.rs) and clamps a wider ask without saying so, so a
  // whole-run query silently misses everything older. Walk the run in
  // 2,000-block windows, the way a correct client must.
  const WINDOW = 2000;
  for (const [name, addr] of deployed) {
    try {
      let n = 0;
      for (let from = 1; from <= latest; from += WINDOW) {
        const to = Math.min(latest, from + WINDOW - 1);
        const logs = await getLogs({ fromBlock: '0x' + from.toString(16), toBlock: '0x' + to.toString(16), address: addr });
        n += Array.isArray(logs) ? logs.length : 0;
      }
      logCounts[name] = n;
    } catch (e) {
      logCounts[name] = `error: ${e.message}`;
    }
  }
  // The silent clamp itself, recorded: one whole-range ask for the first
  // contract that logged, against the windowed total.
  const probe = deployed.find(([name]) => typeof logCounts[name] === 'number' && logCounts[name] > 0);
  if (probe) {
    const wide = await getLogs({ fromBlock: '0x1', toBlock: '0x' + latest.toString(16), address: probe[1] }).catch((e) => ({ error: e.message }));
    ctx.getLogsClamp = { contract: probe[0], blocks: latest, windowed: logCounts[probe[0]], wholeRange: Array.isArray(wide) ? wide.length : wide };
    log(`eth_getLogs whole-range ask for ${probe[0]}: ${JSON.stringify(ctx.getLogsClamp)}`);
  }
  const withLogs = Object.values(logCounts).filter((n) => typeof n === 'number' && n > 0).length;
  log(`eth_getLogs: ${withLogs}/${deployed.length} deployed contracts emitted events`);

  const secs = Math.round((performance.now() - t0) / 1000);
  const okCount = results.filter((r) => r.ok === true).length;
  log(`flows done in ${secs}s — ${okCount}/${results.length} steps ok`);

  console.log('\nkind    ok   label                                            height  gas  state  fee  latency');
  for (const r of results) console.log(summaryRow(r));

  mkdirSync(dirname(OUT), { recursive: true });
  writeFileSync(OUT, JSON.stringify({
    startedChainId: Number(st.chain_id),
    startHeight: Number(st.height),
    endHeight: latest,
    wallSeconds: secs,
    steps: results.map((r) => ({
      kind: r.kind, label: r.label, ok: r.ok, exit: r.exit, hash: r.hash,
      height: r.height, success: r.success, gas: r.gas, proveGas: r.proveGas,
      stateGas: r.stateGas, stateFee: r.stateFee, contractAddress: r.contractAddress,
      logs: r.logs, cliMs: r.cliMs, submittedMs: r.submittedMs, finalizedMs: r.finalizedMs,
      stdout: r.stdout, stderr: r.stderr, error: r.error, expected: r.expected, revert: r.revert,
      budgetRetries: r.budgetRetries, budgetWaitMs: r.budgetWaitMs, budgetRefusal: r.budgetRefusal,
    })),
    logCounts,
    getLogsClamp: ctx.getLogsClamp ?? null,
    addresses: ctx.deployed,
    dev: ctx.addresses && ctx.addresses.dev,
  }, null, 2));
  log(`results → ${OUT}`);

  const bad = results.filter((r) => r.ok !== true);
  if (bad.length) {
    console.log(`\n${bad.length} steps NOT ok:`);
    for (const r of bad) console.log(' ', r.label, r.error || r.stderr || '');
    process.exitCode = 1;
  }
}

main().catch((e) => {
  console.error('deploy-flows failed:', e && e.stack || e);
  process.exit(1);
});
