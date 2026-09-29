import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { networkSettings } from '../src/lib/network.js';

const bundled = JSON.parse(await readFile(new URL('../network.json', import.meta.url)));
const app = JSON.parse(await readFile(new URL('../../wallet/Resources/network.json', import.meta.url)));

test('the extension default tracks the app bundle', () => {
  assert.deepEqual(bundled, app);
  assert.equal(networkSettings(bundled).chainId, bundled.chain_id);
  assert.equal(networkSettings(bundled, { developmentNetwork: true }).chainId, bundled.chain_id);
  assert.equal(networkSettings({ ...bundled, chain_id: 9000 }).chainId, 9000);
});

test('local development uses only loopback and requires Developer mode', () => {
  const selected = networkSettings(bundled, { developerMode: true, developmentNetwork: true, developmentPort: 18546,
    rpcs: ['https://default.example'] });
  assert.deepEqual(selected, { chainId: 7777, urls: ['http://127.0.0.1:18546'], development: true, port: 18546 });
  assert.throws(() => networkSettings(bundled, { developerMode: true, developmentNetwork: true, developmentPort: 0 }), /port/);
});
