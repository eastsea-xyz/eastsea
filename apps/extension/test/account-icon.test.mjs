import test from 'node:test';
import assert from 'node:assert/strict';
import { createHash, webcrypto } from 'node:crypto';
import { readFile } from 'node:fs/promises';
import {
  ACCOUNT_ICON_VERSION, ACCOUNT_ICON_PALETTES, ACCOUNT_ICON_INK,
  deriveAccountIcon, accountIconSVG, createAccountIcon,
} from '../src/lib/accountIcon.js';

const vectors = JSON.parse(await readFile(new URL('../../../crates/client/tests/account-icon-vectors.json', import.meta.url), 'utf8'));
const hash = (value) => createHash('sha256').update(value).digest('hex');
const features = (seed) => ({ version: 1, palette: seed[0] & 7, layout: ((seed[1] << 8) | seed[2]) & 0x3fff, shape: (seed[0] >>> 3) & 3, rotation: (seed[0] >>> 5) & 3 });

test('all frozen shared vectors and canonical SVG hashes match', () => {
  assert.equal(ACCOUNT_ICON_VERSION, vectors.version);
  assert.deepEqual(ACCOUNT_ICON_PALETTES, vectors.palettes);
  assert.equal(ACCOUNT_ICON_INK, vectors.ink);
  for (const vector of vectors.vectors) {
    const spec = deriveAccountIcon(vector.address);
    assert.deepEqual(spec, vector.features, vector.address);
    assert.equal(hash(accountIconSVG(spec)), vector.svg64Sha256, vector.address);
    assert.equal(accountIconSVG(spec).endsWith('\n'), false);
  }
});

test('case and optional prefixes normalize to the same decoded address', () => {
  for (const { address, features: expected } of vectors.vectors) {
    assert.deepEqual(deriveAccountIcon(address.slice(2)), expected);
    assert.deepEqual(deriveAccountIcon(address.toUpperCase()), expected);
    assert.deepEqual(deriveAccountIcon(address.slice(2).toUpperCase()), expected);
    assert.deepEqual(deriveAccountIcon(address, 1), expected);
  }
});

test('invalid addresses and versions have no seeded icon', () => {
  const address = vectors.vectors[0].address;
  for (const invalid of [undefined, null, 0, {}, [], '', 'alice.eth', '0x', address.slice(0, -1), address + '0', ` ${address}`, `${address} `,
    `${address}\n`, `${address}\r\n`, `${address}\u2028`, `${address}\u2029`, `${address.slice(2)}\n`,
    `0x${'g'.repeat(40)}`, `0x${'f'.repeat(39)}ｆ`, `<svg onload=alert(1)>${'a'.repeat(20)}`]) {
    assert.equal(deriveAccountIcon(invalid), null, String(invalid));
  }
  for (const version of [0, 2, -1, '1', null, NaN, Infinity]) assert.equal(deriveAccountIcon(address, version), null);
  for (const invalid of [null, {}, { ...deriveAccountIcon(address), version: 2 }, { ...deriveAccountIcon(address), palette: '#fff" onload="alert(1)' },
    { ...deriveAccountIcon(address), layout: -1 }, { ...deriveAccountIcon(address), layout: 0x4000 }, { ...deriveAccountIcon(address), shape: 4 },
    { ...deriveAccountIcon(address), rotation: 1.5 }]) assert.equal(accountIconSVG(invalid), null);
  for (const size of [0, -1, NaN, Infinity, null, '64', '64" onload="alert(1)']) {
    assert.equal(accountIconSVG(deriveAccountIcon(address), size), null);
    assert.throws(() => createAccountIcon(address, size, dom), RangeError);
  }
});

test('local SHA-256 agrees independently with Node and Web Crypto for 256 addresses', async () => {
  for (let i = 0; i < 256; i++) {
    const bytes = createHash('sha256').update(`independent-account-icon-input-${i}`).digest().subarray(0, 20);
    const input = Buffer.concat([Buffer.from(vectors.domain, 'utf8'), bytes]);
    const seed = createHash('sha256').update(input).digest();
    const webSeed = Buffer.from(await webcrypto.subtle.digest('SHA-256', input));
    assert.deepEqual(webSeed, seed);
    assert.deepEqual(deriveAccountIcon(`0x${bytes.toString('hex')}`), features(seed), String(i));
  }
  for (const vector of vectors.vectors) {
    const seed = createHash('sha256').update(Buffer.from(vectors.domain)).update(Buffer.from(vector.address.slice(2), 'hex')).digest();
    assert.equal(seed.toString('hex'), vector.seedSha256);
  }
});

function luminance(color) {
  const linear = [1, 3, 5].map((i) => parseInt(color.slice(i, i + 2), 16) / 255).map((v) => v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4);
  return linear[0] * 0.2126 + linear[1] * 0.7152 + linear[2] * 0.0722;
}
const contrast = (a, b) => (Math.max(luminance(a), luminance(b)) + 0.05) / (Math.min(luminance(a), luminance(b)) + 0.05);

test('every fixed palette exceeds 3:1 against ink and both supported surfaces', () => {
  for (const color of ACCOUNT_ICON_PALETTES) {
    for (const surface of [ACCOUNT_ICON_INK, vectors.backgrounds.light, vectors.backgrounds.dark]) assert.ok(contrast(color, surface) >= 3, `${color} against ${surface}`);
  }
});

test('static explorer and site copies are byte-identical to the canonical module', async () => {
  const canonical = await readFile(new URL('../src/lib/accountIcon.js', import.meta.url));
  for (const path of ['../../explorer/js/accountIcon.js', '../../../site/account-icon.js']) assert.deepEqual(await readFile(new URL(path, import.meta.url)), canonical);
});

// The same dependency-free DOM approach as explorer/test/live.mjs, with SVG
// namespaces and events so these tests exercise the actual popup module.
class El {
  constructor(tag, namespaceURI = null) {
    this.tagName = tag; this.namespaceURI = namespaceURI; this.children = []; this.attrs = {}; this.listeners = {};
    this.hidden = false; this.disabled = false; this.checked = false; this.value = '';
    this.classList = { add: (...names) => { this.className = [this.className, ...names].join(' ').trim(); } };
  }
  setAttribute(name, value) { this.attrs[name] = String(value); if (name === 'hidden') this.hidden = true; }
  removeAttribute(name) { delete this.attrs[name]; }
  get className() { return this.attrs.class || ''; }
  set className(value) { this.attrs.class = value; }
  get textContent() { return this.children.map((child) => child instanceof El ? child.textContent : String(child)).join(''); }
  set textContent(value) { this.children = [String(value)]; }
  append(...children) { this.children.push(...children.flat()); }
  replaceChildren(...children) { this.children = children.flat(); }
  focus() {}
  addEventListener(name, callback) { (this.listeners[name] ||= []).push(callback); }
  async fire(name) { for (const listener of this.listeners[name] || []) await listener({ preventDefault() {} }); }
  set innerHTML(_) { throw new Error('HTML parsing is forbidden'); }
}
const dom = { createElement: (tag) => new El(tag), createElementNS: (namespace, tag) => new El(tag, namespace) };
const all = (node) => node instanceof El ? [node, ...node.children.flatMap(all)] : [];
const find = (root, predicate) => all(root).find(predicate);
const icons = (root) => all(root).filter((node) => node.tagName === 'svg');
const identity = (root, address) => all(root).find((node) => node.className.split(' ').includes('account-identity') && node.textContent === address);

function serializeIcon(svg) {
  const serialize = (node) => {
    const attrs = Object.entries(node.attrs).filter(([key]) => !['class', 'focusable'].includes(key));
    if (node.tagName === 'svg') attrs.unshift(['xmlns', node.namespaceURI]);
    const opening = `<${node.tagName} ${attrs.map(([key, value]) => `${key}="${value}"`).join(' ')}`;
    return node.children.length ? `${opening}>${node.children.map(serialize).join('')}</${node.tagName}>` : `${opening}/>`;
  };
  return serialize(svg);
}

test('safe SVG DOM construction matches every canonical SVG and keeps invalid input neutral', () => {
  for (const vector of vectors.vectors) {
    const svg = createAccountIcon(vector.address, 64, dom);
    assert.equal(hash(serializeIcon(svg)), vector.svg64Sha256);
    assert.equal(svg.attrs['aria-hidden'], 'true');
    assert.equal(svg.attrs.focusable, 'false');
    assert.ok(all(svg).every((node) => node.namespaceURI === 'http://www.w3.org/2000/svg'));
  }
  const placeholder = createAccountIcon('<svg onload="alert(1)">', 32, dom);
  assert.equal(placeholder.children.length, 1);
  assert.equal(placeholder.children[0].tagName, 'rect');
  assert.equal(placeholder.attrs.class, 'account-icon placeholder');
  assert.equal(serializeIcon(placeholder).includes('onload'), false);
  const empty = createAccountIcon(undefined, 32, dom);
  assert.equal(serializeIcon(empty), serializeIcon(placeholder));
});

test('the anchors stay occupied and empty, with geometry uniformly scaled at every size', () => {
  const spec = { version: 1, palette: 0, layout: 0, shape: 0, rotation: 0 };
  for (const size of [16, 32, 64]) {
    const svg = accountIconSVG(spec, size);
    assert.ok(svg.includes(`width="${size}" height="${size}" viewBox="0 0 64 64"`));
    assert.ok(svg.includes('<rect x="9" y="9" width="10" height="10"/>'));
    assert.equal(svg.includes('x="45" y="45"'), false);
  }
  assert.equal((accountIconSVG({ ...spec, layout: 0x3fff }).match(/<rect x=/g) || []).length, 15);
});

const sender = vectors.vectors[5].address;
const recipient = vectors.vectors[7].address;
const contract = vectors.vectors[6].address;
let popupImport = 0;

async function popup(statePatch, { approve = false, operations = {} } = {}) {
  const names = ['Node', 'document', 'chrome', 'location', 'navigator', 'window', 'setInterval', 'setTimeout'];
  const previous = Object.fromEntries(names.map((name) => [name, Object.getOwnPropertyDescriptor(globalThis, name)]));
  const app = new El('main'), copied = [], requests = [], timers = [];
  const state = { terms: 4, exists: true, unlocked: true, address: sender, defaultChainId: 7780, developmentNetwork: false, approvals: [], ...statePatch };
  const replies = { state, account: { balance: '1000000000000000000', height: 10, blockAt: Date.now() }, assets: { tokens: [] }, activity: [], ...operations };
  const globals = {
    Node: El, document: { ...dom, body: new El('body'), getElementById: () => app },
    chrome: { runtime: { async sendMessage(request) { requests.push(request); const reply = replies[request.op]; if (reply === undefined) throw new Error(`Unexpected operation ${request.op}`); return { ok: true, result: typeof reply === 'function' ? await reply(request.args) : reply }; } } },
    location: { search: approve ? '?approve=test' : '' }, navigator: { clipboard: { async writeText(text) { copied.push(text); } } },
    window: { close() {} }, setInterval() {}, setTimeout(fn) { timers.push(fn); },
  };
  for (const [name, value] of Object.entries(globals)) Object.defineProperty(globalThis, name, { configurable: true, writable: true, value });
  const settle = async () => { for (let i = 0; i < 4; i++) await new Promise((resolve) => setImmediate(resolve)); };
  const restore = () => { for (const name of names) if (previous[name]) Object.defineProperty(globalThis, name, previous[name]); else delete globalThis[name]; };
  try {
    await import(new URL(`../ui/popup.js?account-icon-test=${++popupImport}`, import.meta.url));
    await settle();
    return { app, copied, requests, timers, restore, settle };
  } catch (error) { restore(); throw error; }
}

test('actual popup Home, receive and unlock pair decorative icons with full account addresses', async () => {
  const home = await popup({});
  try {
    const account = identity(home.app, sender);
    assert.ok(account);
    assert.equal(icons(account)[0].attrs.width, '32');
    await account.fire('click');
    assert.deepEqual(home.copied, [sender]);
    assert.equal(icons(account).length, 1);
    home.timers[0]();
    assert.equal(account.textContent, sender);
    await find(home.app, (node) => node.tagName === 'button' && node.textContent.endsWith('Receive')).fire('click');
    assert.equal(all(home.app).filter((node) => node.className.includes('account-identity') && node.textContent === sender).length, 2);
  } finally { home.restore(); }
  const unlock = await popup({ unlocked: false });
  try { assert.ok(identity(unlock.app, sender)); assert.equal(icons(unlock.app).length, 1); } finally { unlock.restore(); }
});

test('actual connect and transaction approvals show the signing identities and preserve explicit approval', async () => {
  const connect = await popup({ approvals: [{ id: 'test', kind: 'connect', origin: 'https://example.com' }] }, { approve: true, operations: { approve: {} } });
  try {
    assert.ok(identity(connect.app, sender));
    assert.equal(connect.requests.some((request) => request.op === 'approve'), false);
    await find(connect.app, (node) => node.tagName === 'button' && node.textContent === 'Connect').fire('click');
    assert.deepEqual(connect.requests.find((request) => request.op === 'approve').args, { id: 'test' });
  } finally { connect.restore(); }
  for (const [selector, what] of [['095ea7b3', 'Token approval (allows spending)'], ['a9059cbb', 'Token transfer']]) {
    const approval = await popup({ approvals: [{ id: 'test', kind: 'transaction', origin: '<img src=x onerror=alert(1)>', what, value: '0', tx: { to: contract, data: `0x${selector}${recipient.slice(2).padStart(64, '0')}${'0'.repeat(63)}1` } }] }, { approve: true, operations: { quote: '1' } });
    try {
      for (const address of [sender, contract, recipient]) assert.ok(identity(approval.app, address), address);
      assert.equal(icons(approval.app).length, 3);
      assert.equal(all(approval.app).some((node) => node.tagName === 'img'), false);
      assert.equal(approval.requests.some((request) => request.op === 'approve'), false);
    } finally { approval.restore(); }
  }
  const validApprovalData = `0x095ea7b3${recipient.slice(2).padStart(64, '0')}${'0'.repeat(64)}`;
  for (const data of [`0x095ea7b3${'1'.repeat(24)}${recipient.slice(2)}${'0'.repeat(64)}`, `${validApprovalData}\n`, validApprovalData.slice(0, -2)]) {
    const malformed = await popup({ approvals: [{ id: 'test', kind: 'transaction', origin: 'https://example.com', what: 'Token approval (allows spending)', value: '0', tx: { to: contract, data } }] }, { approve: true, operations: { quote: '1' } });
    try { assert.equal(identity(malformed.app, recipient), undefined); assert.equal(icons(malformed.app).length, 2); } finally { malformed.restore(); }
  }
  const native = await popup({ approvals: [{ id: 'test', kind: 'transaction', origin: 'https://example.com', what: 'Send DBLN', value: '1', tx: { to: recipient, data: '0x' } }] }, { approve: true, operations: { quote: '1', approve: { hash: `0x${'a'.repeat(64)}` } } });
  try {
    assert.ok(identity(native.app, sender));
    assert.ok(identity(native.app, recipient));
    await find(native.app, (node) => node.tagName === 'button' && node.textContent === 'Approve').fire('click');
    assert.deepEqual(native.requests.find((request) => request.op === 'approve').args, { id: 'test' });
  } finally { native.restore(); }
});

test('actual linked account list preserves full addresses next to 32 px icons', async () => {
  const view = await popup({}, { operations: { activityPage: { items: [], cursors: {}, starts: {} }, linkedWallets: [recipient] } });
  try {
    await find(view.app, (node) => node.tagName === 'button' && node.textContent === 'Activity').fire('click');
    await view.settle();
    assert.ok(identity(view.app, recipient));
    assert.equal(icons(identity(view.app, recipient))[0].attrs.width, '32');
  } finally { view.restore(); }
});

test('recipient input updates only the preview; token confirmation and signing retain the frozen send intent', async () => {
  const token = { address: contract, decimals: 6, symbol: 'USDX', trusted: true, name: 'Test Dollar' };
  const view = await popup({}, { operations: { assets: { tokens: [{ token, balance: '2000000' }] }, sendCheck: { dry: { state: 'ok' } }, send: { hash: `0x${'a'.repeat(64)}` } } });
  try {
    await find(view.app, (node) => node.tagName === 'button' && node.textContent.endsWith('Send')).fire('click');
    await view.settle();
    const input = find(view.app, (node) => node.tagName === 'input' && node.attrs.placeholder === '0x… recipient');
    const amount = find(view.app, (node) => node.tagName === 'input' && node.attrs.inputmode === 'decimal');
    const preview = find(view.app, (node) => node.className === 'recipient-preview');
    assert.equal(preview.hidden, true);
    input.value = recipient;
    await input.fire('input');
    assert.ok(identity(preview, recipient));
    input.value = 'alice.eth';
    await input.fire('input');
    assert.equal(preview.hidden, true);
    assert.equal(icons(preview).length, 0);
    input.value = recipient;
    await input.fire('input');
    const select = find(view.app, (node) => node.tagName === 'select');
    select.value = contract;
    await select.fire('change');
    amount.value = '1.25';
    await amount.fire('input');
    const form = find(view.app, (node) => node.tagName === 'form' && node.textContent.startsWith('Send'));
    await form.fire('submit');
    const confirmation = find(view.app, (node) => node.className === 'card' && node.textContent.startsWith('Confirm the send'));
    assert.ok(identity(confirmation, recipient));
    assert.ok(identity(confirmation, sender));
    input.value = vectors.vectors[8].address;
    amount.value = '99';
    await input.fire('input');
    await amount.fire('input');
    assert.ok(identity(confirmation, recipient));
    await find(confirmation, (node) => node.tagName === 'button' && node.textContent === 'Confirm').fire('click');
    const signed = view.requests.find((request) => request.op === 'send');
    assert.equal(signed.args.to, recipient);
    assert.equal(signed.args.token.baseUnits, '1250000');
    assert.equal(signed.args.token.address, contract);
  } finally { view.restore(); }
});
