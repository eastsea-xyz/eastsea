import test from 'node:test';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readFile } from 'node:fs/promises';
import { deriveAccountIcon, accountIconSVG, createAccountIcon } from '../js/accountIcon.js';
import { accountView, addrLink } from '../js/pages.js';

const fixture = JSON.parse(await readFile(new URL('../../../crates/client/tests/account-icon-vectors.json', import.meta.url), 'utf8'));

test('the served explorer module matches every shared canonical vector and SVG hash', () => {
  for (const vector of fixture.vectors) {
    const spec = deriveAccountIcon(vector.address);
    assert.deepEqual(spec, vector.features);
    assert.deepEqual(deriveAccountIcon(vector.address.toUpperCase()), spec);
    assert.deepEqual(deriveAccountIcon(vector.address.slice(2)), spec);
    assert.equal(createHash('sha256').update(accountIconSVG(spec)).digest('hex'), vector.svg64Sha256);
  }
  for (const address of ['', 'alice.eth', `${fixture.vectors[0].address}\n`, `${fixture.vectors[0].address}\r\n`, `${fixture.vectors[0].address}\u2028`]) assert.equal(deriveAccountIcon(address), null);
  assert.equal(deriveAccountIcon(fixture.vectors[0].address, 2), null);
});

test('the independently served module is byte-identical to extension and site', async () => {
  const actual = await readFile(new URL('../js/accountIcon.js', import.meta.url));
  for (const path of ['../../extension/src/lib/accountIcon.js', '../../../site/account-icon.js']) assert.deepEqual(await readFile(new URL(path, import.meta.url)), actual);
});

class El {
  constructor(tag, namespaceURI = null) { this.tagName = tag; this.namespaceURI = namespaceURI; this.children = []; this.attrs = {}; this.listeners = {}; }
  setAttribute(name, value) { this.attrs[name] = String(value); }
  get className() { return this.attrs.class || ''; }
  set className(value) { this.attrs.class = value; }
  get textContent() { return this.children.map((child) => child instanceof El ? child.textContent : String(child)).join(''); }
  append(...children) { this.children.push(...children.flat()); }
  replaceChildren(...children) { this.children = children.flat(); }
  addEventListener(name, callback) { (this.listeners[name] ||= []).push(callback); }
  set innerHTML(_) { throw new Error('Chain data must never be parsed as HTML'); }
}
const dom = { createElement: (tag) => new El(tag), createElementNS: (namespace, tag) => new El(tag, namespace) };
const all = (root) => root instanceof El ? [root, ...root.children.flatMap(all)] : [];

function withDOM() {
  const oldNode = Object.getOwnPropertyDescriptor(globalThis, 'Node'), oldDocument = Object.getOwnPropertyDescriptor(globalThis, 'document');
  Object.defineProperty(globalThis, 'Node', { configurable: true, writable: true, value: El });
  Object.defineProperty(globalThis, 'document', { configurable: true, writable: true, value: dom });
  return () => {
    if (oldNode) Object.defineProperty(globalThis, 'Node', oldNode); else delete globalThis.Node;
    if (oldDocument) Object.defineProperty(globalThis, 'document', oldDocument); else delete globalThis.document;
  };
}

test('actual explorer address links keep their routes and adjacent address text with decorative 16 px SVG', () => {
  const restore = withDOM();
  try {
    const address = fixture.vectors[7].address;
    const link = addrLink(address.toUpperCase());
    assert.equal(link.attrs.href, `#/account/${address}`);
    assert.equal(link.attrs.title, address.toUpperCase());
    assert.ok(link.textContent.includes('…'));
    const svg = link.children[0];
    assert.equal(svg.namespaceURI, 'http://www.w3.org/2000/svg');
    assert.equal(svg.attrs.width, '16');
    assert.equal(svg.attrs['aria-hidden'], 'true');
    const malicious = addrLink('<img src=x onerror=alert(1)>');
    assert.equal(malicious.children[0].children.length, 1);
    assert.equal(all(malicious).some((node) => node.tagName === 'img'), false);
  } finally { restore(); }
});

test('actual explorer account header shows a 64 px icon beside the full address and copy control', async () => {
  const restore = withDOM();
  try {
    const address = fixture.vectors[8].address;
    const requests = [];
    const ctx = {
      chainId: 7780,
      node: { url: 'http://offline.test', async call(method, params) {
        requests.push({ method, params });
        if (method === 'aether_getAccount') return { balance: '123', nonce: 4, code_size: 0, height: 10, state_root: 'a'.repeat(64) };
        if (method === 'aether_rewards' || method === 'eth_getLogs') return [];
        if (method === 'eth_blockNumber') return '0xa';
        throw new Error(`Unexpected method ${method}`);
      } },
      async read() { throw new Error('Not a token'); },
    };
    const page = await accountView(ctx, address.toUpperCase());
    const heading = all(page).find((node) => node.className === 'account-heading');
    assert.ok(heading);
    assert.equal(heading.textContent, `${address}copy`);
    const svg = heading.children[0];
    assert.equal(svg.attrs.width, '64');
    assert.equal(svg.attrs.height, '64');
    assert.equal(svg.attrs['aria-hidden'], 'true');
    assert.equal(svg.children[0].attrs.fill, fixture.palettes[fixture.vectors[8].features.palette]);
    assert.ok(all(heading).some((node) => node.tagName === 'button' && node.attrs['aria-label'] === 'Copy to clipboard'));
    assert.ok(requests.every(({ method }) => !method.includes('send')));
    assert.deepEqual(requests.find(({ method }) => method === 'aether_getAccount').params, [address]);
  } finally { restore(); }
});

test('missing or malformed explorer addresses produce the same unseeded neutral placeholder', () => {
  for (const address of [undefined, null, '', '0x1234', '<svg onload=alert(1)>']) {
    const svg = createAccountIcon(address, 32, dom);
    assert.equal(svg.children.length, 1);
    assert.equal(svg.children[0].tagName, 'rect');
    assert.equal(svg.children[0].attrs.fill, '#808890');
    assert.equal(svg.attrs['aria-hidden'], 'true');
  }
});
