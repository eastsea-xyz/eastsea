import test from 'node:test';
import assert from 'node:assert/strict';
import { Brand, coinTicker, coinName } from '../src/lib/brand.js';
import { TERMS_VERSION, noticePoints } from '../src/lib/terms.js';
import { describeCall } from '../src/lib/methods.js';

test('the brand and the confirmed coin name are in one place', () => {
  assert.equal(Brand.project, 'EastSea');
  assert.equal(Brand.projectKo, '동해');
  assert.equal(Brand.coinName, 'Doubloon');
  assert.equal(Brand.coinTicker, 'DBLN');
  assert.equal(describeCall({ to: '0x1', data: '0x' }), `Send ${Brand.coinTicker}`);
});

test('the legacy 7780 testnet kept its own coin label', () => {
  assert.equal(coinTicker(7780), 'AETH');
  assert.equal(coinName(7780), 'Test AETH');
  assert.equal(coinTicker(7777), 'DBLN');
  assert.equal(coinName(7777), 'Doubloon');
  assert.equal(describeCall({ to: '0x1', data: '0x' }, { ticker: coinTicker(7780) }), 'Send AETH');
  assert.equal(describeCall({ to: '0x1', data: '0xd0e30db0' }, { ticker: coinTicker(7780) }), 'Wrap AETH');
});

test('renamed legal notice requires new consent and keeps its warning', () => {
  assert.equal(TERMS_VERSION, 4);
  const points = noticePoints(0);
  assert.match(points[0], /^EastSea is built for production\./);
  assert.match(points[0], /its DBLN does not carry over/);
  assert.match(points[0], /provided as is, without warranty/);
  assert.match(points[1], /nothing here promises a price, a return, a listing or a way to cash out/);
  // The legacy testnet's notice names its own coin.
  assert.match(noticePoints(7780)[0], /its AETH does not carry over/);
});
