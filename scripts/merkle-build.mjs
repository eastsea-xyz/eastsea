#!/usr/bin/env node
// Builds a Merkle claim tree for MerkleDistributor.sol (docs/design/17-token-tools.md)
// from a CSV of `address,amount` rows (amounts in base units, comments with #):
//
//   node scripts/merkle-build.mjs recipients.csv [--token 0xTOKEN] [--out merkle.json]
//
// The tree matches the contract exactly: leaves are keccak256(index, account,
// amount), every inner node hashes its pair sorted (smaller hash first), and an
// odd level repeats its last node. Rows are sorted by (address, amount) before
// indices are assigned, so the same CSV in any row order gives the same root.
// Output (stdout) is the JSON the wallet/relayers read: root, total, count and
// one claim per line. Every proof is re-verified against the root before the
// file is written. No dependencies: keccak is implemented below.

import { readFileSync, writeFileSync } from 'node:fs';
import { argv, exit } from 'node:process';
import { isAbsolute, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

// ---- keccak256 (Keccak-f[1600], the Ethereum pre-standard padding) ----

const LANE = 64n;
const MASK = (1n << LANE) - 1n;
const ROUND_CONSTANTS = [
  0x0000000000000001n, 0x0000000000008082n, 0x800000000000808an, 0x8000000080008000n,
  0x000000000000808bn, 0x0000000080000001n, 0x8000000080008081n, 0x8000000000008009n,
  0x000000000000008an, 0x0000000000000088n, 0x0000000080008009n, 0x000000008000000an,
  0x000000008000808bn, 0x800000000000008bn, 0x8000000000008089n, 0x8000000000008003n,
  0x8000000000008002n, 0x8000000000000080n, 0x000000000000800an, 0x800000008000000an,
  0x8000000080008081n, 0x8000000000008080n, 0x0000000080000001n, 0x8000000080008008n,
];
/// Rotation offsets by lane position [x][y].
const RHO = [
  [0, 36, 3, 41, 18],
  [1, 44, 10, 45, 2],
  [62, 6, 43, 15, 61],
  [28, 55, 25, 21, 56],
  [27, 20, 39, 8, 14],
];

const rotl = (v, n) => ((v << BigInt(n)) | (v >> (LANE - BigInt(n)))) & MASK;

function keccakF(s) {
  for (let round = 0; round < 24; round++) {
    const b = [[0n, 0n, 0n, 0n, 0n], [0n, 0n, 0n, 0n, 0n], [0n, 0n, 0n, 0n, 0n], [0n, 0n, 0n, 0n, 0n], [0n, 0n, 0n, 0n, 0n]];
    const c = [0n, 0n, 0n, 0n, 0n];
    const d = [0n, 0n, 0n, 0n, 0n];
    for (let x = 0; x < 5; x++) for (let y = 0; y < 5; y++) c[x] ^= s[x][y];
    for (let x = 0; x < 5; x++) d[x] = c[(x + 4) % 5] ^ rotl(c[(x + 1) % 5], 1);
    for (let x = 0; x < 5; x++) for (let y = 0; y < 5; y++) s[x][y] ^= d[x];
    for (let x = 0; x < 5; x++) for (let y = 0; y < 5; y++) b[y][(2 * x + 3 * y) % 5] = rotl(s[x][y], RHO[x][y]);
    for (let x = 0; x < 5; x++) for (let y = 0; y < 5; y++) s[x][y] = b[x][y] ^ (~b[(x + 1) % 5][y] & b[(x + 2) % 5][y]);
    s[0][0] ^= ROUND_CONSTANTS[round];
  }
}

export function keccak256(input) {
  const rate = 136; // bytes of state the message streams through (1088-bit rate)
  const s = [[0n, 0n, 0n, 0n, 0n], [0n, 0n, 0n, 0n, 0n], [0n, 0n, 0n, 0n, 0n], [0n, 0n, 0n, 0n, 0n], [0n, 0n, 0n, 0n, 0n]];
  const absorb = (buf) => {
    for (let i = 0; i < rate / 8; i++) s[i % 5][(i / 5) | 0] ^= buf.readBigUInt64LE(i * 8);
    keccakF(s);
  };
  const msg = Buffer.from(input);
  const padded = Buffer.concat([msg, Buffer.from([0x01])]); // keccak pad byte (0x01, not SHA-3's 0x06)
  let off = 0;
  while (padded.length - off >= rate) {
    absorb(padded.subarray(off, off + rate));
    off += rate;
  }
  const last = Buffer.alloc(rate);
  padded.copy(last, 0, off);
  last[rate - 1] |= 0x80;
  absorb(last);
  const out = Buffer.alloc(32);
  for (let i = 0; i < 4; i++) out.writeBigUInt64LE(s[i % 5][(i / 5) | 0], i * 8);
  return out;
}

// ---- the tree (mirrors MerkleDistributor.claim) ----

const hex32 = (buf) => `0x${buf.toString('hex')}`;

/** The contract's leaf: keccak256(abi.encodePacked(index, account, amount)).
 * `index`/`amount` may be numbers, BigInts or decimal strings. */
export function leafHash(index, address, amount) {
  const word = (v) => (typeof v === 'bigint' ? v : BigInt(v)).toString(16).padStart(64, '0');
  return keccak256(Buffer.concat([
    Buffer.from(word(index), 'hex'),
    Buffer.from(address.replace(/^0x/, ''), 'hex'),
    Buffer.from(word(amount), 'hex'),
  ]));
}

/** Sorted pair hash, as the contract's proof walk combines two nodes. */
const hashPair = (a, b) => (a.compare(b) <= 0 ? keccak256(Buffer.concat([a, b])) : keccak256(Buffer.concat([b, a])));

/** Root and per-leaf proofs; an odd level repeats its last node. */
export function buildTree(leaves) {
  if (leaves.length === 0) throw new Error('the tree is empty');
  if (leaves.length === 1) return { root: leaves[0], proofs: [[]] };
  const levels = [leaves];
  while (levels[levels.length - 1].length > 1) {
    const level = levels[levels.length - 1];
    const next = [];
    for (let i = 0; i < level.length; i += 2) next.push(hashPair(level[i], i + 1 < level.length ? level[i + 1] : level[i]));
    levels.push(next);
  }
  const proofs = leaves.map((_, index) => {
    const proof = [];
    let at = index;
    for (const level of levels.slice(0, -1)) {
      proof.push(level[at ^ 1] ?? level[at]); // odd level: the sibling is the node itself
      at >>= 1;
    }
    return proof;
  });
  return { root: levels[levels.length - 1][0], proofs };
}

/** The claim-side check `claim` runs: fold the proof with sorted pairs. */
export function verifyProof(root, index, address, amount, proof) {
  let node = leafHash(index, address, amount);
  for (const sibling of proof) node = hashPair(node, sibling);
  return node.equals(root);
}

// ---- CSV in, JSON out ----

/** Parse and validate the rows; `line` is only for error messages. */
export function parseCsv(text) {
  const rows = [];
  text.split(/\r?\n/).forEach((raw, i) => {
    const line = raw.trim();
    if (!line || line.startsWith('#')) return;
    if (/^address\s*,\s*amount$/i.test(line)) return; // optional header
    const cols = line.split(',').map((c) => c.trim());
    if (cols.length !== 2) throw new Error(`line ${i + 1}: expected "address,amount"`);
    if (!/^0x[0-9a-fA-F]{40}$/.test(cols[0])) throw new Error(`line ${i + 1}: not an address: ${cols[0]}`);
    if (!/^\d+$/.test(cols[1]) || BigInt(cols[1]) === 0n) throw new Error(`line ${i + 1}: the amount must be a positive whole number of base units`);
    rows.push({ address: cols[0].toLowerCase(), amount: BigInt(cols[1]) });
  });
  if (rows.length === 0) throw new Error('no recipients (only comments or a header)');
  return rows;
}

/** The whole pipeline: rows -> sorted entries -> tree -> verified claims JSON. */
export function buildCampaign(rows, token) {
  const entries = [...rows].sort((a, b) => (a.address < b.address ? -1 : a.address > b.address ? 1 : a.amount < b.amount ? -1 : a.amount > b.amount ? 1 : 0))
    .map((row, index) => ({ ...row, index }));
  const leaves = entries.map((e) => leafHash(e.index, e.address, e.amount));
  const { root, proofs } = buildTree(leaves);
  const claims = entries.map((e, i) => {
    const claim = { index: e.index, address: e.address, amount: e.amount.toString(), proof: proofs[i].map(hex32) };
    if (!verifyProof(root, claim.index, claim.address, claim.amount, claim.proof.map((p) => Buffer.from(p.slice(2), 'hex')))) {
      throw new Error(`internal: proof for index ${e.index} does not verify`);
    }
    return claim;
  });
  const out = { root: hex32(root), total: entries.reduce((t, e) => t + e.amount, 0n).toString(), count: claims.length, claims };
  if (token) out.token = token.toLowerCase();
  return out;
}

/** The CLI's output: every claim on its own line (fixture tests read it). */
export function render(out) {
  const head = [`  "root": ${JSON.stringify(out.root)}`, `  "total": ${JSON.stringify(out.total)}`, `  "count": ${out.count}`];
  if (out.token) head.unshift(`  "token": ${JSON.stringify(out.token)}`);
  return `{\n${head.join(',\n')},\n  "claims": [\n${out.claims.map((c) => `    ${JSON.stringify(c)}`).join(',\n')}\n  ]\n}\n`;
}

const die = (msg) => {
  console.error(`merkle-build: ${msg}`);
  exit(1);
};

// The parts below only run when this file is the script being executed, so the
// functions above stay importable (a future node test can exercise them).
if (import.meta.url === pathToFileURL(argv[1] || '').href) {
  const args = argv.slice(2);
  const flag = (name) => {
    const at = args.indexOf(name);
    if (at < 0) return undefined;
    const value = args[at + 1];
    if (!value) die(`${name} needs a value`);
    args.splice(at, 2);
    return value;
  };
  const out = flag('--out');
  const token = flag('--token');
  if (token && !/^0x[0-9a-fA-F]{40}$/.test(token)) die('--token is not an address');
  const [file] = args;
  if (!file || args.length !== 1) {
    console.error('usage: merkle-build.mjs <recipients.csv> [--token 0xTOKEN] [--out merkle.json]');
    exit(1);
  }

  try {
    const campaign = buildCampaign(parseCsv(readFileSync(isAbsolute(file) ? file : resolve(process.cwd(), file), 'utf8')), token);
    const json = render(campaign);
    if (out) writeFileSync(out, json); else process.stdout.write(json);
    console.error(`${campaign.count} entries, total ${campaign.total}, root ${campaign.root}${out ? ` -> ${out}` : ''}`);
  } catch (e) {
    die(e.message);
  }
}
