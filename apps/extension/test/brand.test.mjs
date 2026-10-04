import test from 'node:test';
import assert from 'node:assert/strict';
import { Brand } from '../src/lib/brand.js';
import { TERMS_VERSION, NOTICE_POINTS } from '../src/lib/terms.js';
import { describeCall } from '../src/lib/methods.js';

test('the brand and the confirmed coin name are in one place', () => {
  assert.equal(Brand.project, 'EastSea');
  assert.equal(Brand.projectKo, '동해');
  assert.equal(Brand.coinName, 'Doubloon');
  assert.equal(Brand.coinTicker, 'DBLN');
  assert.equal(describeCall({ to: '0x1', data: '0x' }), `Send ${Brand.coinTicker}`);
});

test('renamed legal notice requires new consent and keeps its warning', () => {
  assert.equal(TERMS_VERSION, 4);
  assert.match(NOTICE_POINTS[0], /^EastSea is built for production\./);
  assert.match(NOTICE_POINTS[0], /provided as is, without warranty/);
  assert.match(NOTICE_POINTS[1], /nothing here promises a price, a return, a listing or a way to cash out/);
});
