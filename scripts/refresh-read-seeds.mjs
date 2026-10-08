#!/usr/bin/env node
// Refresh release discovery hints through independent HTTP bridges to Mainline
// DHT. A locator signature never changes the pinned committee or chain identity.
import { createPublicKey, verify } from 'node:crypto';
import { readFile, writeFile } from 'node:fs/promises';
import { pathToFileURL } from 'node:url';

export const LOOKUP_RELAYS = ['https://pkarr.pubky.app', 'https://pkarr.pubky.org', 'https://relay.pkarr.org'];
const ALPHABET = 'ybndrfg8ejkmcpqxot1uwisza345h769';

export function zbase32(bytes) {
  let bits = 0, value = 0, encoded = '';
  for (const byte of bytes) {
    value = (value << 8) | byte; bits += 8;
    while (bits >= 5) { bits -= 5; encoded += ALPHABET[(value >>> bits) & 31]; }
  }
  if (bits) encoded += ALPHABET[(value << (5 - bits)) & 31];
  return encoded;
}

export function verifyPkarrPayload(node, bytes, now = Date.now()) {
  try {
    if (!/^[a-f\d]{64}$/i.test(node)) return false;
    const payload = Buffer.from(bytes);
    if (payload.length < 84 || payload.length > 1072) return false;
    const seq = payload.readBigUInt64BE(64);
    const nowMicros = BigInt(now) * 1000n;
    if (seq > nowMicros + 120_000_000n || seq + 7_200_000_000n < nowMicros) return false;
    const dns = payload.subarray(72);
    const signable = Buffer.concat([Buffer.from(`3:seqi${seq}e1:v${dns.length}:`), dns]);
    const key = createPublicKey({ format: 'der', type: 'spki', key: Buffer.concat([
      Buffer.from('302a300506032b6570032100', 'hex'), Buffer.from(node, 'hex'),
    ]) });
    return verify(null, signable, key, payload.subarray(0, 64));
  } catch { return false; }
}

async function lookup(node, relays, fetchFn, now) {
  for (const relay of relays) {
    try {
      const url = new URL(relay);
      if (url.protocol !== 'https:') throw new Error('lookup relay must use https');
      const response = await fetchFn(`${url.href.replace(/\/$/, '')}/${zbase32(Buffer.from(node, 'hex'))}`, {
        signal: AbortSignal.timeout(5_000), headers: { Accept: 'application/octet-stream' },
      });
      if (!response.ok) continue;
      if (Number(response.headers?.get?.('content-length') || 0) > 1072) continue;
      let bytes;
      if (response.body?.getReader) {
        const reader = response.body.getReader(); const chunks = []; let size = 0;
        try {
          for (;;) {
            const { value, done } = await reader.read(); if (done) break;
            size += value.length; if (size > 1072) break;
            chunks.push(Buffer.from(value));
          }
        } finally { await reader.cancel(); }
        if (size > 1072) continue;
        bytes = Buffer.concat(chunks);
      } else bytes = await response.arrayBuffer();
      if (verifyPkarrPayload(node, bytes, now())) return true;
    } catch { /* next independently replaceable lookup route */ }
  }
  return false;
}

export async function refreshSeeds(network, previous, {
  relays = LOOKUP_RELAYS, fetch: fetchFn = globalThis.fetch, now = Date.now,
} = {}) {
  if (!Number.isSafeInteger(network.chain_id) || previous.chain_id !== network.chain_id) throw new Error('seed chain differs from pinned network');
  const nodes = new Set();
  for (const peer of [...(network.validators || []), ...(previous.peers || [])]) {
    const node = typeof peer === 'string' ? peer : peer.node;
    if (/^[a-f\d]{64}$/i.test(node || '') && nodes.size < 64) nodes.add(node.toLowerCase());
  }
  // Bounded batches keep a release refresh from flooding public lookup bridges.
  const peers = [];
  const candidates = [...nodes];
  for (let i = 0; i < candidates.length; i += 4) {
    const batch = candidates.slice(i, i + 4);
    const found = await Promise.all(batch.map(node => lookup(node, relays.slice(0, 8), fetchFn, now)));
    found.forEach((ok, j) => { if (ok) peers.push({ node: batch[j] }); });
  }
  if (peers.length < 3) throw new Error(`only ${peers.length} signed public node locators resolved; keep the previous release seeds`);
  return { chain_id: network.chain_id, refreshed_at: new Date(now()).toISOString(), peers };
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const network = JSON.parse(await readFile('apps/explorer/network.json', 'utf8'));
  const previous = JSON.parse(await readFile('apps/explorer/public-read-peers.json', 'utf8'));
  const seeds = await refreshSeeds(network, previous);
  const content = `${JSON.stringify(seeds, null, 2)}\n`;
  await writeFile('apps/explorer/public-read-peers.json', content);
  await writeFile('apps/extension/public-read-peers.json', content);
  console.log(`refreshed ${seeds.peers.length} signed DHT node locators for chain ${seeds.chain_id}`);
}
