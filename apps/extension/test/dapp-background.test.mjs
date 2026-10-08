import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import vm from 'node:vm';
import { Wallet } from '../src/lib/wallet.js';
import * as methods from '../src/lib/methods.js';
import { DappSigning } from '../src/lib/dappSigning.js';
import { t } from '../src/lib/i18n.js';
import { networkSettings } from '../src/lib/network.js';
import { pinStore } from '../src/lib/pinStore.js';
import { Brand, coinTicker, coinName } from '../src/lib/brand.js';
import { weiToAeth } from '../src/lib/units.js';

const OWN = '0x1111111111111111111111111111111111111111';
const CONTRACT = '0x2222222222222222222222222222222222222222';
const ORIGIN = 'https://dapp.test';
const tx = { from: OWN, to: CONTRACT, value: '0x1', gas: '0x186a0' };
const typed = { types: { EIP712Domain: [{ name: 'name', type: 'string' }, { name: 'chainId', type: 'uint256' }], Ask: [{ name: 'text', type: 'string' }] }, primaryType: 'Ask', domain: { name: 'Example', chainId: 7781 }, message: { text: 'hello' } };

async function worker() {
  const f = { signed: 0, sent: 0, revert: false };
  const local = new Map([['sites', { [ORIGIN]: { address: OWN } }]]), session = new Map();
  const storage = (map) => ({ get: async (key) => ({ [key]: structuredClone(map.get(key)) }), set: async (object) => { for (const [key, value] of Object.entries(object)) map.set(key, structuredClone(value)); }, remove: async (key) => map.delete(key) });
  const listen = { addListener: () => {} };
  const storageListeners = [];
  const chrome = { runtime: { id: 'fixture', getURL: (path) => `chrome-extension://fixture/${path}`, onConnect: listen, onMessage: listen },
    storage: { local: storage(local), session: storage(session), onChanged: { addListener: (fn) => storageListeners.push(fn) } }, windows: { create: (_opts, callback) => callback({ id: 1 }), get: async () => ({ id: 1 }), onRemoved: listen }, alarms: { create: () => {}, onAlarm: listen } };
  class Rpc {
    constructor() { f.rpc = this; this.chainId = 7781; this.generation = 0; this.urls = ['fixture']; }
    setChain(chain, urls) { this.chainId = chain; this.urls = urls; this.generation++; }
    setVerifier() { this.generation++; }
    async call(method) {
      if (method === 'aether_status') return { chain_id: this.chainId, base_fee: { exec: '0', prove: '0' } };
      if (method === 'eth_getCode') return '0x';
      if (method === 'eth_getBalance') return '0xde0b6b3a7640000';
      if (method === 'eth_getTransactionCount') return '0x0';
      if (method === 'eth_call' || method === 'eth_estimateGas') {
        if (f.revert) throw Object.assign(new Error('execution reverted: fixture denied'), { code: 3 });
        return method === 'eth_call' ? '0x' : '0x5208';
      }
      if (method === 'aether_simulateTransaction') return { success: !f.revert, gasUsed: '0x5208', output: '0x', failureReason: f.revert ? 'fixture denied' : null, nativeChanges: [], logs: [], tokenChanges: [] };
      if (method === 'aether_sendTransaction') { f.sent++; return { hash: '0xsent' }; }
      if (method === 'aether_getReceipt') return { receipt: { success: true, gas_used: 21000 }, height: 1 };
      throw new Error(`unexpected ${method}`);
    }
  }
  class Vault {
    info = async () => ({ address: OWN, publicKey: '04' + '11'.repeat(64) });
    unlocked = async () => true;
    sign = async () => { f.signed++; return new Uint8Array(64); };
  }
  const wasm = { accountAddress: () => OWN, prepareTx: () => JSON.stringify({ signing_message: '1234', envelope: {} }), attachSignature: () => '{}', publicKeyFromSecret: () => [], verifyAccount: () => '{}',
    accountSigningSupport: () => true, prepareTypedMessage: (_public, json, chain) => JSON.stringify({ chain_id: Number(chain), account: OWN, signing_message: '1234', digest_hex: '0x' + 'aa'.repeat(32), typed_data: JSON.parse(json) }), attachTypedSignature: () => '0x' + 'aa'.repeat(128) };
  const src = (await readFile(new URL('../src/background.js', import.meta.url), 'utf8')).replace(/^import .*;\n/gm, '');
  const context = vm.createContext({ ...methods, ...wasm, wasm, init: async () => {}, DEFAULT_LOCK_MINUTES: 30, DEFAULT_RPCS: ['fixture'], Rpc, RpcError: Error, Vault, Wallet, DappSigning, pinStore, networkSettings, Brand, coinTicker, coinName, weiToAeth, t,
    chrome, fetch: async () => ({ json: async () => ({ chain_id: 7781 }) }), crypto: globalThis.crypto, structuredClone, URL, Map, Set, Date, Promise, console, setTimeout });
  vm.runInContext(src + '\nglobalThis.fixtureHooks = { pageRequest, ui, approvals, configured, setSite };', context);
  f.hooks = context.fixtureHooks;
  await f.hooks.configured;
  f.waiting = async () => { await new Promise(setImmediate); return [...f.hooks.approvals.entries()][0]; };
  f.writeSites = (next) => {
    const oldValue = structuredClone(local.get('sites'));
    local.set('sites', structuredClone(next));
    for (const listener of storageListeners) listener({ sites: { oldValue, newValue: structuredClone(next) } }, 'local');
  };
  return f;
}

test('background approval refuses a transaction without a reviewed node preview', async () => {
  const f = await worker();
  const result = f.hooks.pageRequest(ORIGIN, 'eth_sendTransaction', [tx]);
  result.catch(() => {});
  const [id] = await f.waiting();
  await f.hooks.ui.quote({ id });
  await assert.rejects(f.hooks.ui.approve({ id }), /preview|미리보기|プレビュー|预览|預覽/);
  assert.equal(f.signed, 0);
  f.hooks.ui.reject({ id });
});

test('background binds the revert checkbox to the displayed preview before dApp signing', async () => {
  const f = await worker(); f.revert = true;
  const result = f.hooks.pageRequest(ORIGIN, 'eth_sendTransaction', [tx]); result.catch(() => {});
  const [id] = await f.waiting();
  assert.equal(typeof f.hooks.ui.preview, 'function', 'background must expose the preview');
  const preview = await f.hooks.ui.preview({ id });
  assert.equal(preview.simulation.success, false);
  await assert.rejects(f.hooks.ui.approve({ id, previewId: preview.previewId }), /Confirm|확인|確認|确认/);
  assert.equal(f.signed, 0);
  const fresh = await f.hooks.ui.preview({ id });
  const sent = await f.hooks.ui.approve({ id, previewId: fresh.previewId, confirmRevert: true });
  assert.equal(sent.hash, '0xsent');
  assert.equal(await result, '0xsent');
});

test('background routes eth_signTypedData_v4 to the readable typed approval and resolves a 128-byte signature', async () => {
  const f = await worker();
  let rejection;
  const result = f.hooks.pageRequest(ORIGIN, 'eth_signTypedData_v4', [OWN, JSON.stringify(typed)]).catch((e) => { rejection = e; });
  const waiting = await f.waiting();
  assert.ok(waiting, `typed request must wait for review, not reject with ${rejection?.code}`);
  const [id, request] = waiting;
  assert.equal(request.kind, 'typed');
  assert.equal(request.fields[0].value, 'hello');
  await f.hooks.ui.approve({ id, previewId: request.previewId });
  assert.match(await result, /^0x[0-9a-f]{256}$/);
  assert.equal(f.sent, 0);
});

for (const path of ['local setSite', 'storage permission events']) {
  test(`a consumed approval stays invalid after same-account revoke/regrant through ${path}`, async () => {
    const f = await worker();
    const requested = f.hooks.pageRequest(ORIGIN, 'eth_sendTransaction', [tx]); requested.catch(() => {});
    const [id] = await f.waiting();
    const preview = await f.hooks.ui.preview({ id });
    let resume;
    const held = new Promise((resolve) => { resume = resolve; });
    const call = f.rpc.call.bind(f.rpc);
    f.rpc.call = async (method, params) => { if (method === 'eth_getTransactionCount') await held; return call(method, params); };
    const approving = f.hooks.ui.approve({ id, previewId: preview.previewId }); approving.catch(() => {});
    await new Promise(setImmediate);
    assert.equal(f.hooks.approvals.has(id), false, 'approval is already consumed while nonce preparation waits');
    if (path === 'local setSite') {
      await f.hooks.ui.disconnect({ origin: ORIGIN });
      await f.hooks.setSite(ORIGIN, { address: OWN });
    } else {
      f.writeSites({});
      f.writeSites({ [ORIGIN]: { address: OWN } });
    }
    resume();
    await assert.rejects(approving, (e) => e.key === 'requestChanged');
    await assert.rejects(requested, (e) => e.key === 'requestChanged');
    assert.equal(f.signed, 0);
    assert.equal(f.sent, 0);
  });
}
