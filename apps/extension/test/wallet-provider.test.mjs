import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import vm from 'node:vm';

import { READ_METHODS, ACCOUNT_METHODS, SEND_METHODS } from '../src/lib/methods.js';

const here = path.dirname(fileURLToPath(import.meta.url));
const providerSrc = readFileSync(path.join(here, '../../wallet/Resources/provider.js'), 'utf8');
const inpageSrc = readFileSync(path.join(here, '../src/inpage.js'), 'utf8');

// The full set the wallet may answer: methods.js's three groups plus the
// three the extension's background answers by hand.
const SUPPORTED = new Set([
  ...READ_METHODS, ...ACCOUNT_METHODS, ...SEND_METHODS,
  'eth_chainId', 'wallet_disconnect', 'aether_disconnect',
]);

// Runs one of the scripts in a fake page. `replies` maps a method to what the
// native side would answer ({ result } or { error: { code, message } });
// `verify` does the same for the eastsea handler, keyed by `what`.
function run(src, replies = new Map(), verify = new Map()) {
  const announced = [];
  const bridgeCalls = [];
  let initialized = false;
  const listeners = {};
  const pageWindow = {
    addEventListener: (t, fn) => { (listeners[t] ??= new Set()).add(fn); },
    dispatchEvent: (e) => { for (const fn of listeners[e.type] || []) fn(e); return true; },
    postMessage: (m) => bridgeCalls.push(m), // the extension's inpage channel
    location: { origin: 'https://dapp.test' },
    webkit: {
      messageHandlers: {
        aether: { postMessage: (m) => { bridgeCalls.push(m); return Promise.resolve(replies.get(m.method) ?? { result: null }); } },
        eastsea: { postMessage: (m) => { bridgeCalls.push(m); return Promise.resolve(verify.get(m.what) ?? { result: null }); } },
      },
    },
  };
  pageWindow.addEventListener('eip6963:announceProvider', (e) => announced.push(e.detail));
  pageWindow.addEventListener('aether#initialized', () => { initialized = true; });
  const sandbox = {
    window: pageWindow, console, Date, Math, Promise, Set, Map, Array, Object, Error,
    crypto: { randomUUID: () => 'e2b6963a-0000-4000-8000-000000000000' },
    btoa: (s) => Buffer.from(s, 'binary').toString('base64'),
    Event: class { constructor(type) { this.type = type; } },
    CustomEvent: class { constructor(type, opts) { this.type = type; this.detail = opts && opts.detail; } },
  };
  vm.runInContext(src, vm.createContext(sandbox));
  return { sandbox, pageWindow, announced, bridgeCalls, initialized };
}

test('the injected provider answers exactly the extension method set', () => {
  const { sandbox } = run(providerSrc);
  const got = new Set(sandbox.window.aether.supportedMethods);
  assert.deepEqual([...got].sort(), [...SUPPORTED].sort());
});

test('surface parity with the extension inpage provider', () => {
  const wallet = run(providerSrc).sandbox.window.aether;
  const ext = run(inpageSrc).sandbox.window.aether;
  for (const p of [wallet, ext]) {
    assert.equal(p.isAether, true);
    assert.equal(p.chainId, '0x1e64');
    assert.equal(typeof p.request, 'function');
    assert.equal(typeof p.on, 'function');
    assert.equal(typeof p.removeListener, 'function');
    assert.equal(p.on('x', () => {}), p, 'on chains like the extension');
  }
  assert.equal(Object.isFrozen(wallet), true, 'frozen like the extension provider');
});

test('unsupported methods reject with 4200, same as the extension', async () => {
  const { sandbox } = run(providerSrc);
  for (const method of ['personal_sign', 'eth_signTypedData_v4', 'wallet_switchEthereumChain', 'eth_sign', 'eth_getTransactionReceipt']) {
    await assert.rejects(sandbox.window.aether.request({ method }), (e) => e.code === 4200, `${method} refused`);
  }
});

test('a non-string method rejects with -32600', async () => {
  const { sandbox } = run(providerSrc);
  await assert.rejects(sandbox.window.aether.request({ method: 42 }), (e) => e.code === -32600);
  await assert.rejects(sandbox.window.aether.request({}), (e) => e.code === -32600);
});

test('requests cross the bridge and results come back', async () => {
  const replies = new Map([['eth_chainId', { result: '0x1e64' }]]);
  const { sandbox, bridgeCalls } = run(providerSrc, replies);
  assert.equal(await sandbox.window.aether.request({ method: 'eth_chainId' }), '0x1e64');
  const call = bridgeCalls.find((m) => m.method === 'eth_chainId');
  assert.ok(call, 'the bridge saw the request');
  assert.match(call.id, /^aether-/, 'id prefix like the extension');
  // The params live in the vm's realm; compare them as plain JSON.
  assert.equal(JSON.stringify(call.params), '[]');
});

test('params are normalized to an array', async () => {
  const { sandbox, bridgeCalls } = run(providerSrc);
  await sandbox.window.aether.request({ method: 'eth_blockNumber', params: undefined });
  assert.equal(JSON.stringify(bridgeCalls.at(-1).params), '[]', 'undefined params become []');
  await sandbox.window.aether.request({ method: 'eth_call', params: { to: '0x1' } });
  assert.equal(JSON.stringify(bridgeCalls.at(-1).params), '[]', 'object params become [], as in the extension');
  await sandbox.window.aether.request({ method: 'eth_call', params: [{ to: '0x1' }, 'latest'] });
  assert.equal(JSON.stringify(bridgeCalls.at(-1).params), '[{"to":"0x1"},"latest"]', 'array params pass through');
});

test('bridge errors reject with their code kept', async () => {
  const replies = new Map([['eth_requestAccounts', { error: { code: 4001, message: 'denied' } }]]);
  const { sandbox } = run(providerSrc, replies);
  await assert.rejects(sandbox.window.aether.request({ method: 'eth_requestAccounts' }), (e) => e.code === 4001 && /denied/.test(e.message));
});

test('a missing bridge refuses everything (4100)', async () => {
  const { sandbox } = run(providerSrc);
  delete sandbox.window.webkit;
  await assert.rejects(sandbox.window.aether.request({ method: 'eth_chainId' }), (e) => e.code === 4100);
});

test('EIP-6963 announce matches the extension', () => {
  for (const src of [providerSrc, inpageSrc]) {
    const { announced } = run(src);
    assert.equal(announced.length, 1);
    const { info, provider } = announced[0];
    assert.equal(info.name, 'EastSea Wallet');
    assert.equal(info.rdns, 'com.pipln.aether');
    assert.match(info.uuid, /[0-9a-f-]{36}/);
    assert.match(info.icon, /^data:image\/svg\+xml;base64,/);
    assert.equal(provider.isAether, true);
  }
});

test('the extension inpage provider still round-trips a reply', async () => {
  const { sandbox, pageWindow, bridgeCalls } = run(inpageSrc);
  const p = sandbox.window.aether.request({ method: 'eth_chainId' });
  const sent = bridgeCalls.find((m) => m.tag === 'aether:to-content');
  assert.ok(sent, 'inpage still posts to the content script');
  pageWindow.dispatchEvent({ type: 'message', source: pageWindow, data: { tag: 'aether:to-page', id: sent.id, result: '0x1e64' } });
  assert.equal(await p, '0x1e64');
});

test('both scripts are idempotent on a double inject', () => {
  for (const src of [providerSrc, inpageSrc]) {
    const { sandbox } = run(src);
    const first = sandbox.window.aether;
    vm.runInContext(src, sandbox); // a second copy must not replace the provider
    assert.equal(sandbox.window.aether, first);
  }
});

// ---- window.eastsea.verify: the native verification surface ----

test('window.eastsea.verify posts {what, param} and resolves with the verdict', async () => {
  const verify = new Map([
    ['block', { result: { verified: true, height: 6, reason: '' } }],
    ['account', { result: { verified: false, reason: 'certificate: expired' } }],
  ]);
  const { sandbox, bridgeCalls } = run(providerSrc, new Map(), verify);
  const v = sandbox.window.eastsea.verify;
  for (const k of ['block', 'account', 'receipt']) assert.equal(typeof v[k], 'function', `${k} is a function`);
  assert.deepEqual(await v.block(6), { verified: true, height: 6, reason: '' });
  assert.deepEqual(await v.account('0x00000000000000000000000000000000000000aa'), { verified: false, reason: 'certificate: expired' });
  const call = bridgeCalls.find((m) => m.what === 'block');
  assert.ok(call, 'the eastsea bridge saw the ask');
  assert.match(call.id, /^eastsea-/, 'its own id space');
  assert.equal(call.param, 6);
});

test('window.eastsea.verify keeps the wallet\'s error codes and refuses without a handler', async () => {
  const verify = new Map([
    ['receipt', { error: { code: 4200, message: 'This page cannot use EastSea verification.' } }],
  ]);
  const { sandbox } = run(providerSrc, new Map(), verify);
  await assert.rejects(sandbox.window.eastsea.verify.receipt(`0x${'ab'.repeat(32)}`), (e) => e.code === 4200);
  delete sandbox.window.webkit.messageHandlers.eastsea;
  await assert.rejects(sandbox.window.eastsea.verify.block(6), (e) => e.code === 4100);
});

test('window.eastsea is frozen and idempotent, and never touches window.aether', () => {
  const { sandbox } = run(providerSrc);
  const firstEastsea = sandbox.window.eastsea;
  const firstAether = sandbox.window.aether;
  assert.equal(Object.isFrozen(firstEastsea), true);
  assert.equal(Object.isFrozen(firstEastsea.verify), true);
  vm.runInContext(providerSrc, sandbox); // a second copy must not replace either surface
  assert.equal(sandbox.window.eastsea, firstEastsea);
  assert.equal(sandbox.window.aether, firstAether);
});
