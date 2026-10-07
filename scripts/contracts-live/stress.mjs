#!/usr/bin/env node
// Stress phase for scripts/contracts-live.sh: a burst of 200 transfers to
// fresh accounts plus 20 large (EastSeaAccount, ~3.4 M gas) deploys submitted
// in a short window through the wallet path (the aether CLI, explicit nonces,
// no --wait so submissions overlap). Verifies the three things that must hold
// under B5 state-budget pressure:
//   1. block production never stops (height keeps advancing while the burst drains),
//   2. refused/queued transactions are classified (included, in-block refusal,
//      client-side refusal, pending or dropped WITH the node's reason) instead
//      of disappearing — a hash the node can say nothing about is a silent
//      loss and fails the run (contracts-live bug #5),
//   3. the wallet-facing error text a user would read is captured verbatim.
// Writes tmp/live/stress.json.

import { writeFileSync, mkdirSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { keccak_256 as keccak256 } from '@noble/hashes/sha3';
import { rpc, height, cli, devAddress, nonceOf, sleep } from './lib.mjs';

const HERE = dirname(fileURLToPath(import.meta.url));
const OUT = process.env.OUT || resolve(HERE, '../../tmp/live/stress.json');

const log = (...a) => console.log(new Date().toISOString().slice(11, 19), ...a);

const TRANSFERS = Number(process.env.STRESS_TRANSFERS || 200);
const DEPLOYS = Number(process.env.STRESS_DEPLOYS || 20);
const WAVE = Number(process.env.STRESS_WAVE || 20);
const DEPLOYERS = Number(process.env.STRESS_DEPLOYERS || 5);
// The node keeps at most 64 pending transactions per sender (chain.rs
// MAX_PER_SENDER); one sender would measure that cap, not B5. The transfers
// are spread over dev1..dev4 (≤ 50 each), so every one of them reaches the
// state-budget check.
const SENDERS = Number(process.env.STRESS_SENDERS || 4);
const RPC_URL = process.env.AETHER_RPC || 'http://127.0.0.1:8645';
// Optional (B5 review round 2, finding 2): a follower of the same chain. A
// remote wallet's reads go to followers first, and a follower never sees the
// validators' pending transactions. The run asks it about every hash the
// validator still holds (pending) or dropped, and counts what a follower-first
// read would have shown, next to the wallet's routing — the admitting
// validator first, then the follower, then a validator — which must never
// come back empty for a hash the validator knows.
const FOLLOWER_RPC = process.env.AETHER_FOLLOWER_RPC || null;
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
      const args = ['send', '--rpc', RPC_URL, '--from-dev', String(it.dev), '--to', it.addr, '--value', '1', '--nonce', String(it.nonce)];
      const r = await cli(args, { timeoutMs: 60_000 });
      const hash = r.out.match(/tx (0x[0-9a-f]{64})/)?.[1] ?? null;
      return { ...it, code: r.code, out: r.out.trim().split('\n').slice(-2).join(' | '), err: r.err.trim().split('\n').slice(-2).join(' | '), hash, waveStartMs: start, submittedMs: Math.round(r.marks.submitted ?? r.ms) };
    }));
    waves.push({ waveIdx, start, end: Date.now(), ok: runs.filter((r) => r.hash).length, of: runs.length });
    subs.push(...runs);
    log(`wave ${waveIdx}: ${runs.filter((r) => r.hash).length}/${runs.length} submitted (h=${await height()})`);
  };

  // `aether deploy` has no --nonce (it reads the account nonce and waits for
  // inclusion, like the wallet's deploy sheet), so the 20 large deploys come
  // from DEPLOYERS separate dev accounts, each sending its share back to back,
  // all accounts at once, while dev1's transfer waves are in flight.
  const deployers = Array.from({ length: DEPLOYERS }, (_, k) => 6 + k);
  for (const d of deployers) {
    const r = await cli(['send', '--rpc', RPC_URL, '--from-dev', '1', '--to', devAddress(d), '--value', String(10n ** 22n), '--nonce', String(nonce++), '--wait']);
    if (r.code !== 0) throw new Error(`funding deployer dev${d}: ${r.err.trim()}`);
  }
  log(`deployers dev${deployers[0]}..dev${deployers.at(-1)} funded (h=${await height()})`);

  let waveIdx = 0;
  const plan = [];
  const senderNonce = { 1: nonce };
  for (let d = 2; d <= SENDERS; d++) {
    const r = await cli(['send', '--rpc', RPC_URL, '--from-dev', '1', '--to', devAddress(d), '--value', String(10n ** 21n), '--nonce', String(senderNonce[1]++), '--wait']);
    if (r.code !== 0) throw new Error(`funding sender dev${d}: ${r.err.trim()}`);
    senderNonce[d] = await nonceOf(devAddress(d));
  }
  for (let i = 0; i < TRANSFERS; i++) {
    const dev = 1 + (i % SENDERS);
    plan.push({ kind: 'transfer', dev, addr: freshAddr(i), nonce: senderNonce[dev]++ });
  }
  const deployRuns = (async () => {
    const per = Math.ceil(DEPLOYS / deployers.length);
    const out = await Promise.all(deployers.map(async (d, k) => {
      const mine = [];
      for (let j = 0; j < per && k * per + j < DEPLOYS; j++) {
        const start = Date.now();
        const r = await cli(['deploy', '--rpc', RPC_URL, '--from-dev', String(d), '--code', BIG_CODE, '--gas', '6000000'], { timeoutMs: 120_000 });
        const hash = r.out.match(/tx (0x[0-9a-f]{64})/)?.[1] ?? null;
        mine.push({ kind: 'deploy', dev: d, nonce: j, code: r.code, out: r.out.trim().split('\n').slice(-2).join(' | '), err: r.err.trim().split('\n').slice(-2).join(' | '), hash, waveStartMs: start, submittedMs: Math.round(r.marks.submitted ?? r.ms) });
      }
      return mine;
    }));
    return out.flat();
  })();
  for (let o = 0; o < plan.length; o += WAVE) await submitWave(plan.slice(o, o + WAVE), waveIdx++);
  subs.push(...(await deployRuns));
  log(`deploys: ${subs.filter((s) => s.kind === 'deploy' && s.hash).length}/${DEPLOYS} got a tx hash`);
  const submittedAll = Date.now();
  clearInterval(sampler);
  const remote = { followerRpc: FOLLOWER_RPC, samples: [] };
  // The wallet's routing (crates/ffi `receipt_answer`) on plain JSON-RPC:
  // the admitting validator first, then the follower, then the validator.
  const routed = async (hash) => {
    const v = await rpc('aether_getReceipt', [hash]).catch(() => null);
    if (v) return v;
    const f = FOLLOWER_RPC ? await rpc('aether_getReceipt', [hash], FOLLOWER_RPC).catch(() => null) : null;
    return f ?? (await rpc('aether_getReceipt', [hash]).catch(() => null));
  };
  const sampleRemote = async (label) => {
    if (!FOLLOWER_RPC) return;
    const tally = { label, asked: 0, validatorKnows: 0, followerNull: 0, followerReceipt: 0, routedNull: 0 };
    for (const s of subs.filter((x) => x.hash)) {
      const v = await rpc('aether_getReceipt', [s.hash]).catch(() => null);
      const f = await rpc('aether_getReceipt', [s.hash], FOLLOWER_RPC).catch(() => undefined);
      if (f === undefined) continue; // follower unreachable this round
      tally.asked++;
      if (f && f.receipt) tally.followerReceipt++;
      if (v && !v.receipt && (v.pending || v.status === 'dropped')) {
        tally.validatorKnows++;
        if (f === null) tally.followerNull++;
        if (!(await routed(s.hash))) tally.routedNull++;
      }
    }
    remote.samples.push(tally);
    log(`remote ${label}: ${JSON.stringify(tally)}`);
  };
  await sampleRemote('after-submit');

  // ---------------------------------------------------------------- drain
  // Poll until every submitted hash is included or dropped (the node keeps a
  // reason for each drop), or the window ends; the first poll that sees a
  // receipt approximates its inclusion time. The default window is the
  // 10-minute mempool TTL plus a minute, so every under-priced tx is seen
  // leaving with its reason rather than vanishing just after the window.
  const seen = new Map(); // hash -> {t, r}: included
  const dropped = new Map(); // hash -> {t, r}: left the pool, with a reason
  const last = new Map(); // hash -> the node's latest answer
  const deadline = Date.now() + Number(process.env.STRESS_DRAIN_MS || 660_000);
  while (Date.now() < deadline) {
    const missing = subs.filter((s) => s.hash && !seen.has(s.hash) && !dropped.has(s.hash));
    if (!missing.length) break;
    try {
      const batch = await Promise.all(missing.map(async (s) => ({ s, r: await rpc('aether_getReceipt', [s.hash]) })));
      for (const { s, r } of batch) {
        last.set(s.hash, r);
        if (r && r.receipt) seen.set(s.hash, { t: Date.now(), r });
        else if (r && r.status === 'dropped') dropped.set(s.hash, { t: Date.now(), r });
      }
    } catch { /* retry next round */ }
    samples.push({ t: Date.now(), h: await height().catch(() => null) });
    await sleep(3000);
  }
  await sampleRemote('after-drain');
  const hFinal = await height();
  const pendingAfterDrain = (await rpc('aether_status').catch(() => ({}))).mempool ?? null;
  const nFinal = await nonceOf(devAddress(1)); // dev1 only (the other senders are in the receipts)
  log(`drained: height ${h0}→${hFinal}, dev1 nonce ${n0}→${nFinal}, receipts ${seen.size}/${subs.filter((s) => s.hash).length}`);

  // ---------------------------------------------------------------- classify
  // One last look at what is still unresolved, so its state is current.
  for (const s of subs) {
    if (!s.hash || seen.has(s.hash) || dropped.has(s.hash)) continue;
    const r = await rpc('aether_getReceipt', [s.hash]).catch(() => last.get(s.hash) ?? null);
    last.set(s.hash, r);
    if (r && r.receipt) seen.set(s.hash, { t: Date.now(), r });
    else if (r && r.status === 'dropped') dropped.set(s.hash, { t: Date.now(), r });
  }
  // Bug #5's verdict: every hash ends included, dropped with a reason, or
  // pending with what it waits for. Null — the node knows nothing — is silent.
  const fate = (s) => {
    if (!s.hash) return s.code !== 0 ? 'client-refused' : 'no-hash';
    const rec = seen.get(s.hash)?.r;
    if (rec) return rec.receipt.success === true ? 'included' : 'in-block-refused';
    const d = dropped.get(s.hash)?.r;
    if (d) return `dropped:${d.reason?.kind ?? 'no-reason'}`;
    const p = last.get(s.hash);
    if (p && (p.status === 'pending' || p.pending)) return `pending:${p.waiting?.kind ?? 'its-turn'}`;
    return 'silent';
  };
  const receipts = [];
  for (const s of subs) {
    const hit = s.hash ? seen.get(s.hash) : undefined;
    const rec = hit?.r;
    const why = s.hash ? (dropped.get(s.hash)?.r?.reason ?? last.get(s.hash)?.waiting ?? null) : null;
    receipts.push({
      kind: s.kind, dev: s.dev ?? 1, nonce: Number(s.nonce), to: s.addr ?? null,
      outcome: fate(s), reason: why,
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
  // Keep watching 20 s past the drain, then measure the longest time the
  // finalized height stood still across the whole window (1 s blocks).
  for (let i = 0; i < 10; i++) { samples.push({ t: Date.now(), h: await height().catch(() => null) }); await sleep(2000); }
  samples.sort((a, b) => a.t - b.t);
  let maxStallMs = 0;
  let lastH = null;
  let lastChange = samples[0].t;
  for (const sm of samples) {
    if (sm.h == null) continue;
    if (lastH === null || sm.h > lastH) { lastH = sm.h; lastChange = sm.t; }
    maxStallMs = Math.max(maxStallMs, sm.t - lastChange);
  }
  const maxStallSec = Math.round(maxStallMs / 100) / 10;
  const histogram = {};
  for (const r of receipts) if (r.height != null) histogram[r.height] = (histogram[r.height] || 0) + 1;
  const silent = receipts.filter((r) => r.outcome === 'silent' || r.outcome === 'no-hash' || r.outcome === 'dropped:no-reason').length;
  const refusedWithoutText = receipts.filter((r) => r.outcome === 'client-refused' && !(r.stderr || r.stdout)).length;
  const heightsAdvanced = hFinal > h0 && nFinal >= n0 && (nFinal - n0) >= subs.filter((s) => s.hash && seen.has(s.hash)).length;

  const verdict = {
    blocksNeverStopped: maxStallSec < 10,
    maxStallSec,
    heightBefore: h0, heightAfter: hFinal, mempoolAfterDrain: pendingAfterDrain,
    nonceBefore: Number(n0), nonceAfter: Number(nFinal),
    submitted: subs.length, withHash: subs.filter((s) => s.hash).length,
    receipts: seen.size, dropped: dropped.size, counts, silentLosses: silent, refusedWithoutText,
    remote,
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

  // Queued-but-not-yet-included (B5 refill) and dropped-with-a-reason are
  // designed outcomes; a stalled chain, a tx the node lost without a word
  // (bug #5), or a refusal without text is a failure.
  // The wallet's routing must answer for every hash the validator knows.
  const routedSilent = remote.samples.reduce((n, t) => n + t.routedNull, 0);
  const bad = !verdict.blocksNeverStopped || silent > 0 || refusedWithoutText > 0 || routedSilent > 0;
  if (bad) {
    console.log('STRESS FAIL:', JSON.stringify({ blocksNeverStopped: verdict.blocksNeverStopped, silent, refusedWithoutText, routedSilent, counts }));
    process.exitCode = 1;
  }
}

main().catch((e) => { console.error('stress failed:', e && e.stack || e); process.exit(1); });
