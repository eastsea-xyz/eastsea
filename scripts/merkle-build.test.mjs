// merkle-build.mjs checked alone (no chain): the keccak vectors came from
// `cast keccak`, and the campaign checks mirror what the forge fixture test
// verifies on chain (contracts/test/MerkleDistributor.t.sol). Run with
// `node --test scripts/merkle-build.test.mjs`.

import test from 'node:test';
import assert from 'node:assert/strict';
import { keccak256, leafHash, buildTree, verifyProof, parseCsv, buildCampaign, render } from './merkle-build.mjs';

const h = (s) => keccak256(Buffer.from(s, 'hex')).toString('hex');

test('keccak256: the cast-verified vectors', () => {
  assert.equal(h(''), 'c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470');
  assert.equal(h('616263'), '4e03657aea45a94fc7d47ba826c8d667c0d1e6e33a64a036ec44f58fa12d6c45'); // "abc"
  assert.equal(h('deadbeef'), 'd4fd4e189132273036449fc9e11198c739161b4c0116a9a2dccdfa1c492006f1');
});

test('buildTree: sorted pairs, odd level repeats its last node', () => {
  const leaf = (i) => Buffer.from(String(i).repeat(64).slice(0, 64), 'hex');
  const pair = (a, b) => (a.compare(b) <= 0 ? keccak256(Buffer.concat([a, b])) : keccak256(Buffer.concat([b, a])));
  const { root, proofs } = buildTree([leaf(1), leaf(2), leaf(3)]);
  // level 0: (1,2) and (3,3); level 1: the two parents
  assert.equal(root.equals(pair(pair(leaf(1), leaf(2)), pair(leaf(3), leaf(3)))), true);
  assert.equal(proofs[2].length, 2);
  assert.equal(proofs[2][0].equals(leaf(3)), true); // its sibling is the repeated node
  assert.deepEqual(buildTree([leaf(1)]), { root: leaf(1), proofs: [[]] });
});

test('parseCsv: header and comments skipped, junk rejected', () => {
  assert.deepEqual(
    parseCsv('address,amount\n# note\n\n0x0000000000000000000000000000000000000001, 7 \n'),
    [{ address: '0x0000000000000000000000000000000000000001', amount: 7n }],
  );
  assert.throws(() => parseCsv('0xnope,1'), /not an address/);
  assert.throws(() => parseCsv('0x0000000000000000000000000000000000000001,0'), /positive/);
  assert.throws(() => parseCsv('0x0000000000000000000000000000000000000001,-5'), /amount/);
  assert.throws(() => parseCsv('0x0000000000000000000000000000000000000001'), /address,amount/);
  assert.throws(() => parseCsv('# only comments\n'), /no recipients/);
});

test('buildCampaign: sorted rows, stable indices, same root from any row order', () => {
  const rows = [
    { address: '0x0000000000000000000000000000000000000004', amount: 1500n },
    { address: '0x0000000000000000000000000000000000000002', amount: 250n },
    { address: '0x0000000000000000000000000000000000000004', amount: 500n },
  ];
  const out = buildCampaign(rows, '0x0000000000000000000000000000000000000009');
  assert.deepEqual(out.claims.map((c) => [c.index, c.address.slice(-4), c.amount]),
    [[0, '0002', '250'], [1, '0004', '500'], [2, '0004', '1500']]); // amount breaks the tie
  assert.equal(out.total, '2250');
  assert.equal(out.token, '0x0000000000000000000000000000000000000009');

  const shuffled = buildCampaign([rows[2], rows[0], rows[1]], '0x0000000000000000000000000000000000000009');
  assert.equal(render(shuffled), render(out)); // byte-identical output

  for (const c of out.claims) {
    assert.equal(verifyProof(Buffer.from(out.root.slice(2), 'hex'), c.index, c.address, c.amount,
      c.proof.map((p) => Buffer.from(p.slice(2), 'hex'))), true);
    assert.equal(verifyProof(Buffer.from(out.root.slice(2), 'hex'), c.index, c.address, c.amount + '0',
      c.proof.map((p) => Buffer.from(p.slice(2), 'hex'))), false); // a wrong amount must not verify
  }
});

test('leafHash: accepts numbers, BigInts and decimal strings alike', () => {
  assert.equal(leafHash(1, '0x0000000000000000000000000000000000000002', 5).toString('hex'),
    leafHash(1n, '0x0000000000000000000000000000000000000002', '5').toString('hex'));
});

test('render: valid JSON with one claim per line', () => {
  const out = buildCampaign([{ address: '0x0000000000000000000000000000000000000001', amount: 1n }]);
  const text = render(out);
  const lines = text.trim().split('\n');
  assert.equal(JSON.parse(text).root, out.root);
  assert.equal(lines.filter((l) => l.includes('"index"')).length, 1);
});
