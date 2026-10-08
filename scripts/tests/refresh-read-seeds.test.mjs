import test from 'node:test';
import assert from 'node:assert/strict';
import { generateKeyPairSync, sign } from 'node:crypto';
import { verifyPkarrPayload, refreshSeeds, zbase32 } from '../refresh-read-seeds.mjs';

function packet(timestamp = BigInt(Date.now()) * 1000n) {
  const { privateKey, publicKey } = generateKeyPairSync('ed25519');
  const node = publicKey.export({ type: 'spki', format: 'der' }).subarray(-32).toString('hex');
  const dns = Buffer.from([0, 0, 0x84, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
  const bytes = Buffer.alloc(8); bytes.writeBigUInt64BE(timestamp);
  const message = Buffer.concat([Buffer.from(`3:seqi${timestamp}e1:v${dns.length}:`), dns]);
  return { node, payload: Buffer.concat([sign(null, message, privateKey), bytes, dns]) };
}

test('seed refresh verifies the requested key and bounds packet age/size', () => {
  const { node, payload } = packet();
  assert.equal(verifyPkarrPayload(node, payload), true);
  assert.equal(verifyPkarrPayload('00'.repeat(32), payload), false);
  const forged = Buffer.from(payload); forged[0] ^= 1;
  assert.equal(verifyPkarrPayload(node, forged), false);
  assert.equal(verifyPkarrPayload(node, Buffer.alloc(1073)), false);
  const stale = packet(BigInt(Date.now() - 7_200_001) * 1000n);
  assert.equal(verifyPkarrPayload(stale.node, stale.payload), false);
});

test('release seeds fail over lookup relays without changing chain trust', async () => {
  const good = [packet(), packet(), packet()];
  const network = { chain_id: 7780, identity: 'pinned', validators: good.map(({ node }) => ({ node })) };
  const calls = [];
  const result = await refreshSeeds(network, { chain_id: 7780, peers: [] }, {
    relays: ['https://bad.example', 'https://lookup.example'],
    fetch: async (url) => {
      calls.push(url);
      if (url.startsWith('https://bad.example')) throw new Error('offline');
      const p = good.find(({ node }) => url.endsWith(`/${zbase32(Buffer.from(node, 'hex'))}`));
      return { ok: true, arrayBuffer: async () => p.payload };
    },
  });
  assert.equal(result.chain_id, network.chain_id);
  assert.deepEqual(result.peers.map(p => p.node).sort(), good.map(p => p.node).sort());
  assert.equal(result.peers.length, 3);
  assert.equal(calls.length, 6);
  assert.equal(result.identity, undefined);
  await assert.rejects(refreshSeeds(network, { chain_id: 1, peers: [] }), /chain/);
});
