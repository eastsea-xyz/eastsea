// Shared driver for the live contract run (scripts/contracts-live.sh).
//
// Every state change goes through the `aether` CLI (`send`/`deploy`/`call`),
// which is the same wallet code path the app and the extension use:
// crates/execution/src/tx.rs `sign_call_with` + `recommended_state_budget`,
// fee caps quoted from the node's `aether_status`, P-256 dev keys
// (crates/node/src/chain.rs `dev_seed`). This file never signs a transaction
// itself; the P-256 helpers below only produce in-contract signatures
// (vault spend, registry attestation) over digests the contracts publish.

import { spawn } from 'node:child_process';
import { keccak_256 as keccak256 } from '@noble/hashes/sha3';
import { sha256 } from '@noble/hashes/sha2';
import { p256 } from '@noble/curves/p256';
import { Interface, AbiCoder } from 'ethers';

export const RPC = process.env.AETHER_RPC || 'http://127.0.0.1:8645';
export const BIN = process.env.AETHER_BIN || 'aether';
export const CHAIN_ID = Number(process.env.AETHER_CHAIN_ID || 0);

let rpcId = 0;

export async function rpc(method, params = [], url = RPC) {
  const res = await fetch(url, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ jsonrpc: '2.0', id: ++rpcId, method, params }),
  });
  const body = await res.json();
  if (body.error) {
    const err = new Error(body.error.message || 'rpc error');
    err.code = body.error.code;
    throw err;
  }
  return body.result;
}

export async function height() {
  const st = await rpc('aether_status');
  return Number(st.height);
}

export async function status() {
  return rpc('aether_status');
}

// ---------------------------------------------------------------- dev keys

/// crates/node/src/chain.rs `dev_seed`: 0xae ‖ 30 zero bytes ‖ index.
export function devSeed(i) {
  const s = new Uint8Array(32);
  s[0] = 0xae;
  s[31] = i;
  return s;
}

/// The dev account's address, crates/crypto `address_of` for a P-256 key:
/// keccak256(scheme byte 1 ‖ SEC1 compressed point)[12..] — not the
/// Ethereum uncompressed-xy rule, which only secp256k1 keys use.
export function devAddress(i) {
  const pub = p256.getPublicKey(devSeed(i), true); // 33 bytes, 02/03‖x
  return '0x' + Buffer.from(keccak256(Buffer.concat([Buffer.from([1]), pub])).slice(12)).toString('hex');
}

/// A dev key's public halves, the way EastSeaVault/Account owners are stored.
export function devKeyXY(i) {
  const pub = p256.getPublicKey(devSeed(i), false);
  return { x: '0x' + Buffer.from(pub.slice(1, 33)).toString('hex'), y: '0x' + Buffer.from(pub.slice(33, 65)).toString('hex') };
}

/// Sign a contract-published digest (spendDigest, attestationDigest, …) with a
/// dev key. The contracts pass the digest straight to the P256VERIFY
/// precompile (contracts/src/EastSeaVault.sol `_verify`), so the digest IS the
/// prehash — no extra hashing here. Low-s normalized, like the chain's own
/// `P256Signer::sign` (crates/crypto/src/lib.rs).
export function devSignDigest(i, digestHex) {
  const digest = Buffer.from(digestHex.replace(/^0x/, ''), 'hex');
  const sig = p256.sign(digest, devSeed(i), { prehash: false, lowS: true });
  return {
    r: '0x' + sig.r.toString(16).padStart(64, '0'),
    s: '0x' + sig.s.toString(16).padStart(64, '0'),
  };
}

/// A dev account's next transaction nonce, from the node itself.
export async function nonceOf(addrOrDev, isDevIndex = false) {
  const addr = isDevIndex ? devAddress(addrOrDev) : addrOrDev;
  const n = await rpc('eth_getTransactionCount', [addr, 'latest']);
  return BigInt(n);
}

/// Where the next CREATE from `from` at `nonce` will land (EIP-684 RLP),
/// the same `owner.create(nonce)` the in-process fixtures use.
export function createAddress(from, nonce) {
  const buf = [Buffer.from(from.slice(2), 'hex')];
  const nn = BigInt(nonce);
  if (nn === 0n) buf.push(Buffer.from('80', 'hex'));
  else if (nn < 0x80n) buf.push(Buffer.from([Number(nn)]));
  else {
    const hex = nn.toString(16).padStart(2, '0');
    const bytes = Buffer.from(hex.length % 2 ? '0' + hex : hex, 'hex');
    buf.push(Buffer.from([0x80 + bytes.length]), bytes);
  }
  return '0x' + Buffer.from(keccak256(rlpList(buf))).slice(12).toString('hex');
}
function rlpList(items) {
  const enc = (b) => {
    if (b.length === 1 && b[0] < 0x80) return Buffer.from(b);
    if (b.length <= 55) return Buffer.concat([Buffer.from([0x80 + b.length]), b]);
    const len = Buffer.from(b.length.toString(16).padStart(2, '0'), 'hex');
    return Buffer.concat([Buffer.from([0x80 + 0x80 + len.length]), len, b]);
  };
  const payload = Buffer.concat(items.map(enc));
  if (payload.length <= 55) return Buffer.concat([Buffer.from([0xc0 + payload.length]), payload]);
  const len = Buffer.from(payload.length.toString(16).padStart(2, '0'), 'hex');
  return Buffer.concat([Buffer.from([0xc0 + 0x80 + len.length]), len, payload]);
}

/// EIP-191 personal_sign over a 32-byte hash, the way the in-process fixtures
/// sign multisig/DAO votes (`personal_signature`, secp256k1 + ecrecover).
export async function personalSign(wallet, hashHex) {
  return wallet.signMessage(Buffer.from(hashHex.slice(2), 'hex'));
}

// ---------------------------------------------------------------- the CLI

/// Run the aether CLI, timestamping stdout lines so a wallet-path transaction's
/// submit→finalized latency survives into the report.
export function cli(args, { timeoutMs = 120_000 } = {}) {
  return new Promise((resolve) => {
    const t0 = performance.now();
    const child = spawn(BIN, args, { stdio: ['ignore', 'pipe', 'pipe'] });
    let out = '';
    let err = '';
    const marks = {};
    const onLine = (buf, key) => {
      const text = buf.toString();
      if (key === 'out') {
        for (const line of text.split('\n')) {
          if (line.startsWith('tx 0x')) marks.submitted ??= performance.now() - t0;
          if (line.includes('finalized in block')) marks.finalized ??= performance.now() - t0;
        }
      }
      return text;
    };
    child.stdout.on('data', (d) => (out += onLine(d, 'out')));
    child.stderr.on('data', (d) => (err += onLine(d, 'err')));
    const timer = setTimeout(() => child.kill('SIGKILL'), timeoutMs);
    child.on('close', (code) => {
      clearTimeout(timer);
      resolve({ code, out, err, marks, ms: performance.now() - t0 });
    });
  });
}

/// One wallet-path transaction: run the CLI subcommand, then read the receipt
/// back over RPC. Returns a uniform record for the report.
export async function cliTx(sub, extra, opts) { return tx(sub, extra, opts); }
/// B5 admission refusal: the transaction needs more state units than the
/// rolling budget has left right now (the pre-fix node says "exceeds block gas
/// limit"; the fixed one says "state budget"). A wallet waits for the
/// per-height refill and resubmits; so does this driver, and records the wait.
const BUDGET_REFUSAL = /exceeds block gas limit|state budget/;
export const BUDGET_WAIT_MAX_MS = Number(process.env.BUDGET_WAIT_MAX_MS || 30 * 60_000);

async function tx(sub, extra, { label = '' } = {}) {
  const tStart = performance.now();
  let run = await cli([sub, '--rpc', RPC, ...extra]);
  let budgetRetries = 0;
  let firstRefusal = null;
  while (run.code !== 0 && BUDGET_REFUSAL.test(run.err) && performance.now() - tStart < BUDGET_WAIT_MAX_MS) {
    firstRefusal ??= run.err.trim().split('\n').pop();
    budgetRetries++;
    await sleep(15_000);
    run = await cli([sub, '--rpc', RPC, ...extra]);
  }
  const hashM = run.out.match(/tx (0x[0-9a-f]{64})/);
  const hash = hashM ? hashM[1] : null;
  const record = {
    label,
    ok: run.code === 0,
    exit: run.code,
    hash,
    cliMs: Math.round(run.ms),
    submittedMs: run.marks.submitted != null ? Math.round(run.marks.submitted) : null,
    finalizedMs: run.marks.finalized != null ? Math.round(run.marks.finalized) : null,
    stdout: run.out.trim().split('\n').slice(-3).join(' | '),
    stderr: run.err.trim().split('\n').slice(-3).join(' | '),
    budgetRetries,
    budgetWaitMs: budgetRetries ? Math.round(performance.now() - tStart - run.ms) : 0,
    budgetRefusal: firstRefusal,
  };
  if (hash) {
    try {
      const r = await rpc('aether_getReceipt', [hash]);
      if (r && r.receipt) {
        const rc = r.receipt;
        record.height = Number(r.height);
        record.success = rc.success;
        record.gas = Number(rc.gas_used ?? rc.gas);
        record.proveGas = Number(rc.prove_gas ?? 0);
        record.stateGas = Number(rc.state_gas ?? 0);
        record.stateFee = rc.state_fee ?? '0';
        record.contractAddress = rc.contract_address ?? null;
        record.logs = Number(rc.logs ?? 0);
        record.output = rc.output ?? null;
        record.events = rc.events ?? [];
      }
    } catch {
      /* a refused or never-included tx has no receipt; that is the finding */
    }
  }
  return record;
}

export async function send({ dev, to, value, nonce, wait = true, label = '' }) {
  const extra = ['--from-dev', String(dev), '--to', to, '--value', String(value)];
  if (nonce != null) extra.push('--nonce', String(nonce));
  if (wait) extra.push('--wait');
  return tx('send', extra, { label });
}

export async function deploy({ dev, code, gas, label = '' }) {
  const extra = ['--from-dev', String(dev), '--code', code.startsWith('0x') ? code : '0x' + code];
  if (gas) extra.push('--gas', String(gas));
  return tx('deploy', extra, { label });
}

export async function call({ dev, to, data, value, gas, wait = true, nonce, label = '' }) {
  const extra = ['--from-dev', String(dev), '--to', to, '--data', data || '0x'];
  if (value != null && value !== 0n && value !== '0') extra.push('--value', String(value));
  if (gas) extra.push('--gas', String(gas));
  if (nonce != null) extra.push('--nonce', String(nonce));
  if (wait) extra.push('--wait');
  return tx('call', extra, { label });
}

// ---------------------------------------------------------------- reads

export async function ethCall(to, data, { from, value } = {}) {
  const t = {};
  if (to) t.to = to;
  if (from) t.from = from;
  if (value) t.value = '0x' + BigInt(value).toString(16);
  t.data = data && data !== '0x' ? data : '0x';
  try {
    return await rpc('eth_call', [t, 'latest']);
  } catch (e) {
    const m = /execution reverted: (0x[0-9a-f]*)/i.exec(e.message || '');
    if (m) {
      const err = new Error('revert');
      err.revertData = m[1];
      throw err;
    }
    throw e;
  }
}

export async function getLogs(filter) {
  return rpc('eth_getLogs', [filter]);
}

export function iface(abi) {
  return new Interface(abi);
}

export function coder() {
  return AbiCoder.defaultAbiCoder();
}

/// Decode a revert payload (`Error(string)` or Panic) for the report.
export function decodeRevert(ifc, outputHex) {
  if (!outputHex || outputHex === '0x') return null;
  try {
    const desc = ifc.parseError(outputHex);
    return `${desc.name}(${desc.args.join(', ')})`;
  } catch {
    try {
      const [msg] = coder().decode(['string'], '0x' + outputHex.replace(/^0x/, '').slice(8));
      return `Error("${msg}")`;
    } catch {
      return outputHex.slice(0, 74);
    }
  }
}

export async function sleep(ms) {
  return new Promise((r) => setTimeout(r, ms));
}

export async function waitHeight(target, timeoutMs = 120_000) {
  const end = Date.now() + timeoutMs;
  for (;;) {
    const h = await height();
    if (h >= target) return h;
    if (Date.now() > end) throw new Error(`chain did not reach height ${target}`);
    await sleep(1000);
  }
}

export function nowSec() {
  return Math.floor(Date.now() / 1000);
}
