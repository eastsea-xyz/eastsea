import test from 'node:test';
import assert from 'node:assert/strict';
import { DEFAULT_GATEWAY, loadGateway } from '../js/rpc.js';

test('a fresh browser has no company gateway default', () => {
  assert.equal(DEFAULT_GATEWAY, '');
  assert.equal(loadGateway(null), null);
  assert.equal(loadGateway({ getItem() { throw new Error('storage denied'); } }), null);
  assert.equal(loadGateway({ getItem: () => 'https://my-gateway.example/' }), 'https://my-gateway.example');
});
