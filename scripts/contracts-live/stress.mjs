#!/usr/bin/env node
// Stress phase for scripts/contracts-live.sh: a burst of 200 transfers to
// fresh accounts plus 20 large (EastSeaAccount, ~3.4 M gas) deploys submitted
// in a short window through the wallet path (the aether CLI, explicit nonces,
// no --wait so submissions overlap). Verifies the three things that must hold
// under B5 state-budget pressure:
//   1. block production never stops (height keeps advancing while the burst drains),
//   2. refused/queued transactions are classified (included, in-block refusal,
//      client-side refusal, never seen) instead of disappearing,
//   3. the wallet-facing error text a user would read is captured verbatim.
// Writes tmp/live/stress.json.

import { writeFileSync, mkdirSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { keccak256 } from '@noble/hashes/sha3';
import { rpc, height, cli, devAddress, nonceOf, sleep } from './lib.mjs';

const HERE = dirname(fileURLToPath(import.meta.url));
const OUT = process.env.OUT || resolve(HERE, '../../tmp/live/stress.json');

const log = (...a) => console.log(new Date().toISOString().slice(11, 19), ...a);

const TRANSFERS = Number(process.env.STRESS_TRANSFERS || 200);
const DEPLOYS = Number(process.env.STRESS_DEPLOYS || 20);
const WAVE = Number(process.env.STRESS_WAVE || 20);
// EastSeaAccount runtime bytecode from the fixture: a "large deploy" that also
// creates a state slot per contract. Read from the repo's artifacts copy.
import { readFileSync } from 'node:fs';
const ART = JSON.parse(readFileSync(process.env.AETHER_ARTIFACTS
  || resolve(HERE, '../../crates/contracts-onchain/fixtures/artifacts.json'), 'utf8'));
const BIG_CODE = ART['core/EastSeaAccount'].bytecode;

/// Fresh, never-funded recipient i: keccak-derived address, so every transfer
/// creates a new state slot (the expensive thing under B5).
const freshAddr = (i) => '0x' + Buffer.from(keccak256(Buffer.from(`stress-account-${i}`)).slice(12)).toString('hex');

async function main() {
  const h0 = await height();
  const n0 = await nonceOf(devAddress(1));
  log(`height ${h0}, dev1 nonce ${n0} — ${TRANSFERS} transfers + ${DEPLOYS} deploys in waves of ${WAVE}`);

  // ---------------------------------------------------------------- submit
  const subs = []; // {kind, nonce, addr, wave, waveStartMs, spawnMs, code, out, err, hash, submittedMs}
  const samples = [{ t: Date.now(), h: h0 }];
  let nonce = n0;
  const sampler = setInterval(async () => {
    try { samples.push({ t: Date.now(), h: await height() }); } catch { /* node busy */ }
  }, 2000);

  const waves = [];
  const submitWave = async (items, waveIdx) => {
    const start = Date.now();
    const runs = await Promise.all(items.map(async (it) => {
      const args = it.kind === 'transfer'
        ? ['send', '--rpc', process.env.AETHER_RPC || 'http://127.0.0.1:8645', '--from-dev', '1', '--to', it.addr, '--value', '1', '--nonce', String(it.nonce)]
        : ['deploy', '--rpc', process.env.AETHER_RPC || 'http://127.0.0.1:8645', '--from-dev', '1', '--code', BIG_CODE, '--gas', '6000000', '--nonce', String(it.nonce)];
      const r = await cli(args, { timeoutMs: 60_000 });
      const hash = r.out.match(/tx (0x[0-9a-f]{64})/)?.[1] ?? null;
      return { ...it, code: r.code, out: r.out.trim().split('\n').slice(-2).join(' | '), err: r.err.trim().split('\n').slice(-2).join(' | '), hash, waveStartMs: start, submittedMs: Math.round(r.marks.submitted ?? r.ms) };
    }));
    waves.push({ waveIdx, start, end: Date.now(), ok: runs.filter((r) => r.hash).length, of: runs.length });
    subs.push(...runs);
    log(`wave ${waveIdx}: ${runs.filter((r) => r.hash).length}/${runs.length} submitted (h=${await height()})`);
  };

  let waveIdx = 0;
  const plan = [];
  for (let i = 0; i < TRANSFERS; i++) plan.push({ kind: 'transfer', addr: freshAddr(i), nonce: nonce++ });
  for (let i = 0; i < DEPLOYS; i++) plan.push({ kind: 'deploy', nonce: nonce++ });
  for (let o = 0; o < plan.length; o += WAVE) await submitWave(plan.slice(o, o + WAVE), waveIdx++);
  const submittedAll = Date.now();
  clearInterval(sampler);

  // ---------------------------------------------------------------- drain
  // Poll receipts until every submitted hash resolves or 5 minutes pass; the
  // first poll that sees a receipt approximates its inclusion time.
  const seen = new Map(); // hash -> {t}
  const deadline = Date.now() + 300_000;
  while (Date.now() < deadline) {
    const missing = subs.filter((s) => s.hash && !seen.has(s.hash));
    if (!missing.length) break;
    try {
      const batch = await Promise.all(missing.map(async (s) => ({ s, r: await rpc('aether_getReceipt', [s.hash]) })));
      for (const { s, r } of batch) if (r && r.receipt) seen.set(s.hash, { t: Date.now(), r });
    } catch { /* retry next round */ }
    samples.push({ t: Date.now(), h: await height().catch(() => null) });
    await sleep(3000);
  }
  const hFinal = await height();
  const nFinal = await nonceOf(devAddress(1));
  log(`drained: height ${h0}→${hFinal}, dev1 nonce ${n0}→${nFinal}, receipts ${seen.size}/${subs.filter((s) => s.hash).length}`);

  // ---------------------------------------------------------------- classify
  const receipts = [];
  for (const s of subs) {
    const hit = s.hash ? seen.get(s.hash) : undefined;
    const rec = hit?.r;
    receipts.push({
      kind: s.kind, nonce: Number(s.nonce), to: s.addr ?? null,
      outcome: !s.hash && s.code !== 0 ? 'client-refused'
        : !s.hash ? 'no-hash'
        : rec ? (rec.receipt.success === true ? 'included' : 'in-block-refused') : 'never-included',
      exit: s.code, hash: s.hash, height: rec ? Number(rec.height) : null,
      gas: rec ? Number(rec.receipt.gas_used ?? rec.receipt.gas ?? 0) : null,
      stateGas: rec ? Number(rec.receipt.state_gas ?? 0) : null,
      stateFee: rec?.receipt.state_fee ?? null,
      includedAfterMs: hit ? hit.t - s.waveStartMs : null,
      stderr: s.err || null, stdout: s.out || null,
    });
  }
  const by = (k) => receipts.reduce((m, r) => ((m[r.outcome] ??= []).push(r), m), {});
  const counts = Object.fromEntries(Object.entries(by()).map(([o, rs]) => [o, rs.length]));

  // Distinct wallet-facing error texts (deduped), for the readability check.
  const errors = {};
  for (const r of receipts) {
    if ((r.outcome === 'client-refused' || r.outcome === 'in-block-refused') && (r.stderr || r.stdout)) {
      const key = (r.stderr || r.stdout).replace(/0x[0-9a-f]+/g, '0x…').replace(/\d+/g, 'N');
      errors[key] ??= { raw: r.stderr || r.stdout, count: 0, kind: r.kind };
      errors[key].count++;
    }
  }

  // ---------------------------------------------------------------- verdict
  // Block production never stopped: no 10 s window without a height advance
  // while the burst was draining (empty blocks still finalize).
  let maxStallSec = 0;
  for (let i = 0; i < samples.length; i++) {
    const w = samples.filter((s) => s.t >= samples[i].t && s.t <= samples[i].t + 10_000);
    const adv = Math.max(...w.map((s) => s.h ?? 0)) - (samples[i].h ?? 0);
    maxStallSec = Math.max(maxStallSec, adv <= 0 ? 10 : 0);
  }
  const histogram = {};
  for (const r of receipts) if (r.height != null) histogram[r.height] = (histogram[r.height] || 0) + 1;
  const heightsAdvanced = hFinal > h0 && nFinal >= n0 && (nFinal - n0) >= subs.filter((s) => s.hash && seen.has(s.hash)).length;

  const verdict = {
    blocksNeverStopped: maxStallSec === 0,
    maxStallSec,
    heightBefore: h0, heightAfter: hFinal,
    nonceBefore: Number(n0), nonceAfter: Number(nFinal),
    submitted: subs.length, withHash: subs.filter((s) => s.hash).length,
    receipts: seen.size, counts,
    perHeightHistogram: Object.fromEntries(Object.entries(histogram).sort((a, b) => a[0] - b[0])),
    errorTexts: Object.values(errors),
    drainSeconds: Math.round((Date.now() - submittedAll) / 1000),
  };
  log(`verdict: ${JSON.stringify({ ...verdict, perHeightHistogram: `${Object.keys(histogram).length} heights`, errorTexts: verdict.errorTexts.length })}`);

  mkdirSync(dirname(OUT), { recursive: true });
  writeFileSync(OUT, JSON.stringify({
    verdict, waves, samples, heightHistogram: histogram, receipts, errors,
  }, null, 2));
  log(`stress → ${OUT}`);

  const bad = !verdict.blocksNeverStopped || counts['never-included'] > 0 || counts['no-hash'] > 0;
  if (bad) {
    console.log('STRESS FAIL:', JSON.stringify({ blocksNeverStopped: verdict.blocksNeverStopped, counts }));
    process.exitCode = 1;
  }
}

main().catch((e) => { console.error('stress failed:', e && e.stack || e); process.exit(1); });
