// Exercise real renderer/component behavior with a deterministic browser host.
// This verifies projection and lifecycle logic; it does not replace raster QA.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { createGlobe } from '../live-globe/globe.js';
import { mountLiveGlobe } from '../live-globe/live-globe.js';
import { summarizeQuality, qualityMean, qualityColor } from '../live-globe/quality.js';
import { installWalletHost } from '../../wallet/Resources/LiveGlobe/wallet-host.js';

const today = JSON.parse(await readFile(new URL('../live-globe/fixture.json', import.meta.url)));
const example = JSON.parse(await readFile(new URL('./fixtures/presence-example.json', import.meta.url)));

class Events {
  handlers = new Map();
  addEventListener(type, callback) {
    if (!this.handlers.has(type)) this.handlers.set(type, new Set());
    this.handlers.get(type).add(callback);
  }
  removeEventListener(type, callback) { this.handlers.get(type)?.delete(callback); }
  dispatch(type, extras = {}) {
    const event = { preventDefault() {}, ...extras };
    for (const callback of this.handlers.get(type) || []) callback(event);
  }
}

function browserHost({ reduced = false, webgl = true, width = 600, height = 600, labelDimensions, intersection = false } = {}) {
  let now = 0, id = 0;
  const frames = new Map(), timers = new Map(), uploads = [], renders = [], blends = [];
  const intersections = [];
  const motion = Object.assign(new Events(), { matches: reduced });
  const context = new Proxy({ clearRect: () => renders.push('map') }, { get: (target, key) => target[key] ?? (() => {}) });
  const gl = new Proxy({
    createShader: () => ({}), createProgram: () => ({}), createBuffer: () => ({}),
    getShaderParameter: () => true, getProgramParameter: () => true,
    getAttribLocation: (_program, name) => name, getUniformLocation: (_program, name) => name,
    getParameter: () => [1, 64], bufferData: (_target, data) => uploads.push(Array.from(data)),
    clear: () => renders.push('webgl'),
    blendFunc: (...factors) => blends.push({ method: 'blendFunc', factors }),
    blendFuncSeparate: (...factors) => blends.push({ method: 'blendFuncSeparate', factors }),
  }, { get: (target, key) => target[key] ?? (key === key.toUpperCase() ? key : () => {}) });
  const win = Object.assign(new Events(), {
    devicePixelRatio: 1, performance: { now: () => now },
    matchMedia: () => motion,
    getComputedStyle: () => ({ getPropertyValue: () => '' }),
    requestAnimationFrame: callback => { const next = ++id; frames.set(next, callback); return next; },
    cancelAnimationFrame: frame => frames.delete(frame),
    setTimeout: (callback, delay = 0) => { const next = ++id; timers.set(next, { callback, at: now + delay }); return next; },
    clearTimeout: timer => timers.delete(timer),
    MutationObserver: class { observe() {} disconnect() {} },
  });
  if (intersection) win.IntersectionObserver = class {
    constructor(callback) { this.callback = callback; intersections.push(this); }
    observe(element) { this.target = element; }
    disconnect() { this.disconnected = true; }
    setVisible(isIntersecting) { if (!this.disconnected) this.callback([{ isIntersecting }]); }
  };
  class Element extends Events {
    constructor(tag) {
      super(); this.tagName = tag; this.children = []; this.attributes = new Map(); this.dataset = {};
      this.hidden = false; this.textContent = ''; this.ownerDocument = doc; this.capture = new Set();
      this.style = { display: '', touchAction: '', setProperty(name, value) { this[name] = value; }, getPropertyValue(name) { return this[name] || ''; } };
      this.classList = { add: name => { this.className = [this.className, name].filter(Boolean).join(' '); } };
    }
    append(...elements) { for (const el of elements) { el.parentElement = this; this.children.push(el); } }
    after(el) {
      const siblings = this.parentElement.children;
      siblings.splice(siblings.indexOf(this) + 1, 0, el); el.parentElement = this.parentElement;
    }
    remove() {
      if (this.parentElement) this.parentElement.children = this.parentElement.children.filter(el => el !== this);
    }
    replaceChildren(...elements) { this.children = []; this.append(...elements); }
    get childElementCount() { return this.children.length; }
    get offsetWidth() { return this.className === 'lg-marker-label' ? labelDimensions?.width : undefined; }
    get offsetHeight() { return this.className === 'lg-marker-label' ? labelDimensions?.height : undefined; }
    setAttribute(name, value) { this.attributes.set(name, String(value)); }
    getAttribute(name) { return this.attributes.get(name) ?? null; }
    removeAttribute(name) { this.attributes.delete(name); }
    getBoundingClientRect() { return { width, height }; }
    getContext(kind) { return kind === '2d' ? context : webgl ? gl : null; }
    setPointerCapture(pointer) { this.capture.add(pointer); }
    hasPointerCapture(pointer) { return this.capture.has(pointer); }
    releasePointerCapture(pointer) { this.capture.delete(pointer); }
    focus() {
      doc.activeElement?.dispatch('blur'); doc.activeElement = this; this.dispatch('focus');
    }
  }
  const doc = Object.assign(new Events(), { defaultView: win, hidden: false, activeElement: null });
  doc.createElement = tag => new Element(tag);
  doc.createElementNS = (_namespace, tag) => new Element(tag);
  doc.createTextNode = text => Object.assign(new Element('#text'), { textContent: text });
  doc.documentElement = new Element('html');
  doc.body = new Element('body');
  const root = new Element('div'), stage = new Element('div'), canvas = new Element('canvas');
  stage.append(canvas); root.append(stage);
  const descendants = element => [element, ...element.children.flatMap(descendants)];
  const find = (className, continent) => descendants(root).find(el => el.className?.split(' ').includes(className) && (!continent || (continent.includes(':') ? el.dataset.region === continent : el.dataset.continent === continent)));
  function frame(milliseconds = 16) {
    now += milliseconds;
    for (const [key, timer] of [...timers]) if (timer.at <= now) { timers.delete(key); timer.callback(); }
    const pending = [...frames.values()]; frames.clear();
    for (const callback of pending) callback(now);
  }
  return { root, stage, canvas, doc, motion, frame, find, descendants, frames, timers, uploads, renders, blends, intersections };
}

test('largest-region opening centers its labeled marker, regardless of hemisphere', () => {
  for (const continent of ['asia', 'north_america', 'oceania', 'europe']) {
    const host = browserHost();
    const globe = createGlobe(host.canvas, { seed: 'test-session' });
    globe.setLabels({ continents: { [continent]: continent }, quality: 'Quality' });
    globe.update({ ...today, regions: [{ continent, count: 4, quality: summarizeQuality([.1, .2, .3, .4]) }] });
    const marker = host.find('lg-marker', continent);
    assert.equal(marker.hidden, false);
    assert.equal(marker.style.transform, 'translate3d(300.00px,300.00px,0)');
    assert.equal(host.find('lg-marker-label', undefined).tagName, 'span');
    assert.equal(marker.children[1].textContent, `${continent} 4`);
    globe.destroy();
  }
});

test('WebGL stores source-over alpha rather than squared alpha for premultiplied canvas composition', () => {
  const host = browserHost();
  const globe = createGlobe(host.canvas, { seed: 'alpha' });
  globe.update(today);
  assert.ok(host.blends.length > 0);
  for (const blend of host.blends) {
    assert.equal(blend.method, 'blendFuncSeparate');
    assert.deepEqual(blend.factors, ['SRC_ALPHA', 'ONE_MINUS_SRC_ALPHA', 'ONE', 'ONE_MINUS_SRC_ALPHA']);
  }
  globe.destroy();
});

test('every populated region has a front marker or an explicit list-only visibility state', () => {
  for (const reduced of [false, true]) {
    const host = browserHost({ reduced });
    let visibility;
    const globe = createGlobe(host.canvas, { seed: 'coverage', onVisibility: entries => { visibility = entries; } });
    const snapshot = { ...example, total: 27, versions: { '0.7.4': 27 }, regions: [
      ...example.regions, { continent: 'africa', count: 2, quality: summarizeQuality([.3, .4]) },
      { continent: 'antarctica', count: 1, quality: summarizeQuality([.2]) },
    ] };
    globe.update(snapshot);
    for (let angle = 0; angle < 25; angle++) {
      for (const { key, continent, visibility: state } of visibility) {
        const marker = host.find('lg-marker', key);
        if (state === 'front') assert.equal(marker?.hidden, false, continent);
        else {
          assert.ok(['back', 'unknown'].includes(state), continent);
          assert.ok(!marker || marker.hidden, continent);
        }
      }
      if (!reduced) host.canvas.dispatch('keydown', { key: 'ArrowRight' });
    }
    globe.destroy();
  }
});

test('pulse mean color/intensity varies continuously, with identical cores and sqrt population size', () => {
  const host = browserHost();
  const globe = createGlobe(host.canvas, { seed: 1 });
  const regions = [
    { continent: 'asia', count: 9, quality: summarizeQuality(Array(9).fill(.413)) },
    { continent: 'europe', count: 6, quality: summarizeQuality(Array(6).fill(.417)) },
  ];
  globe.update({ ...today, total: 15, roles: { validator: { count: 4 }, wallet: { count: 3 }, candidate: { count: 0 }, follower: { count: 0 } }, versions: { '0.7.4': 15 }, regions });
  const asia = host.find('lg-marker', 'asia'), europe = host.find('lg-marker', 'europe');
  assert.equal(asia.children[0].children[1].getAttribute('r'), '15');
  assert.equal(europe.children[0].children[1].getAttribute('r'), '15');
  assert.equal(asia.children[0].children[2].getAttribute('stroke-dasharray'), null);
  assert.equal(asia.style['--marker-color'], qualityColor(.413));
  assert.equal(europe.style['--marker-color'], qualityColor(.417));
  assert.notEqual(asia.style['--marker-intensity'], europe.style['--marker-intensity']);
  assert.equal(asia.style['--marker-size'], '40px');
  assert.ok(parseFloat(europe.style['--marker-size']) < 40);
  globe.destroy();
});

test('all-region mobile static-map labels remain inside the stage without overlapping', () => {
  const host = browserHost({ reduced: true, width: 350, height: 350, labelDimensions: { width: 150, height: 36 } });
  const globe = createGlobe(host.canvas, { seed: 'mobile-map' });
  const snapshot = { ...example, total: 27, versions: { '0.7.4': 27 }, regions: [
    ...example.regions, { continent: 'africa', count: 2, quality: summarizeQuality([.3, .4]) },
    { continent: 'antarctica', count: 1, quality: summarizeQuality([.2]) },
  ] };
  globe.update(snapshot);
  const labels = host.descendants(host.root).filter(el => el.className === 'lg-marker' && !el.hidden).map(marker => {
    const [x, y] = marker.style.transform.match(/[\d.-]+(?=px)/g).map(Number);
    const label = marker.children[1];
    return { x: x - 22 + parseFloat(label.style.left), y: y - 22 + parseFloat(label.style.top), w: 150, h: 36, code: marker.dataset.continent };
  });
  assert.equal(labels.length, 9);
  for (const rect of labels) {
    assert.ok(rect.x >= 3.99 && rect.x + rect.w <= 346.01, rect.code);
    assert.ok(rect.y >= 3.99 && rect.y + rect.h <= 346.01, rect.code);
    for (const other of labels) {
      if (rect === other) continue;
      assert.ok(rect.x + rect.w <= other.x || other.x + other.w <= rect.x || rect.y + rect.h <= other.y || other.y + other.h <= rect.y, `${rect.code} overlaps ${other.code}`);
    }
  }
  globe.destroy();
});

test('drag, keyboard, hovered or selected pulse all resume rotation after ten idle seconds', () => {
  for (const interaction of ['keyboard', 'pointer', 'selection']) {
    const host = browserHost();
    const globe = createGlobe(host.canvas, { seed: 1 });
    globe.update(today); host.frame();
    if (interaction === 'keyboard') host.canvas.dispatch('keydown', { key: 'Home' });
    if (interaction === 'pointer') {
      host.canvas.dispatch('pointerdown', { button: 0, pointerId: 1, clientX: 200, clientY: 200 });
      host.canvas.dispatch('pointermove', { pointerId: 1, clientX: 225, clientY: 200 });
      host.canvas.dispatch('pointerup', { pointerId: 1 });
    }
    if (interaction === 'selection') globe.setHighlight('asia');
    const marker = host.find('lg-marker', 'asia'), held = marker.style.transform;
    host.frame(9_900);
    assert.equal(marker.style.transform, held, interaction);
    host.frame(200);
    assert.notEqual(marker.style.transform, held, interaction);
    globe.destroy();
  }
});

test('manual pause, reduced motion, offscreen lifecycle and destroy schedule no animation', () => {
  const host = browserHost();
  const globe = createGlobe(host.canvas, { seed: 1 }); globe.update(today);
  globe.setPaused(true);
  const marker = host.find('lg-marker', 'asia'), held = marker.style.transform;
  host.frame(12_000);
  assert.equal(host.frames.size, 0);
  assert.equal(marker.style.transform, held);
  assert.equal(host.find('lg-markers').dataset.animated, 'false');
  globe.setPaused(false); assert.equal(host.frames.size, 1);
  host.doc.hidden = true; host.doc.dispatch('visibilitychange');
  assert.equal(host.frames.size, 0);
  assert.equal(host.find('lg-markers').dataset.animated, 'false');
  host.doc.hidden = false; host.doc.dispatch('visibilitychange');
  assert.equal(host.frames.size, 1);
  host.motion.matches = true; host.motion.dispatch('change');
  assert.equal(host.frames.size, 0);
  assert.equal(host.canvas.dataset.renderer, 'map');
  globe.destroy();
  assert.equal(host.find('lg-markers'), undefined);
  assert.equal(host.frames.size, 0);
});

test('WebGL context loss shows the static map and recovers with labels/counts intact', () => {
  const host = browserHost();
  const globe = createGlobe(host.canvas, { seed: 1 }); globe.update(today);
  host.canvas.dispatch('webglcontextlost');
  assert.equal(host.canvas.dataset.renderer, 'map');
  assert.equal(host.frames.size, 0);
  assert.equal(host.find('lg-marker', 'asia').hidden, false);
  host.canvas.dispatch('webglcontextrestored');
  assert.equal(host.canvas.dataset.renderer, 'webgl');
  assert.equal(host.frames.size, 1);
  assert.equal(host.find('lg-marker', 'asia').dataset.count, '1');
  assert.equal(host.find('lg-marker', 'asia:KR').dataset.count, '3');
  globe.destroy();
});

test('component links pulse hover and full-row taps, and discloses today’s exact snapshot', async () => {
  const host = browserHost();
  const component = mountLiveGlobe(host.root, {
    fixture: true, lang: 'ko', seed: 'integration',
    fetch: async () => ({ ok: true, json: async () => today }),
  });
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(host.find('lg-status').textContent, '오늘 기준 실제 구성 (실시간 아님)');
  assert.equal(host.find('lg-role-summary').textContent, '검증자 4 · 지갑 노드 3 · 예비 키 3 (대기 3 / 참여 0)');
  const pulse = host.find('lg-marker', 'asia'), row = host.find('lg-region', 'asia');
  pulse.dispatch('pointerenter'); assert.equal(row.dataset.active, 'true');
  pulse.dispatch('pointerleave'); assert.equal(row.dataset.active, 'false');
  row.dispatch('click'); assert.equal(pulse.dataset.active, 'true');
  assert.equal(row.dataset.count, '4');
  assert.equal(Number(row.dataset.quality), 162053 / 4 / 1_000_000);
  assert.equal(host.find('lg-country').children[0].textContent, '대한민국');
  assert.equal(host.find('lg-country').children[1].textContent, '3');
  component.setLanguage('en');
  assert.equal(host.find('lg-status').textContent, 'Today’s actual setup (not live)');
  assert.equal(pulse.children[1].textContent, 'Asia 1');
  component.destroy(); assert.equal(host.frames.size, 0); assert.equal(host.timers.size, 0);
});

test('off-globe list entry remains labeled while its pulse is hidden, including unknown', async () => {
  const host = browserHost();
  const component = mountLiveGlobe(host.root, { fixture: true, seed: 'integration', fetch: async () => ({ ok: true, json: async () => example }) });
  await new Promise(resolve => setImmediate(resolve));
  const north = host.find('lg-region', 'north_america'), unknown = host.find('lg-region', 'unknown');
  assert.equal(north.dataset.visibility, 'back');
  assert.equal(host.find('lg-marker', 'north_america').hidden, true);
  assert.ok(north.children[2].textContent.includes('Far side'));
  north.dispatch('click'); assert.equal(north.dataset.active, 'true');
  assert.equal(host.find('lg-marker', 'north_america').hidden, true);
  assert.equal(unknown.dataset.visibility, 'unknown');
  assert.ok(unknown.children[2].textContent.includes('list only'));
  assert.equal(host.find('lg-marker', 'unknown'), undefined);
  component.destroy();
});

test('offscreen observer stops polling, GPU frames and CSS pulses; visibility requests fresh data', async () => {
  const host = browserHost({ intersection: true });
  let calls = 0;
  const component = mountLiveGlobe(host.root, {
    endpoint: 'https://rpc.example.invalid', seed: 1,
    fetch: async () => { calls++; return { ok: true, json: async () => ({ jsonrpc: '2.0', id: 1, result: today }) }; },
  });
  assert.equal(calls, 0);
  for (const observer of host.intersections) observer.setVisible(true);
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(calls, 1); assert.equal(host.frames.size, 1);
  for (const observer of host.intersections) observer.setVisible(false);
  assert.equal(host.frames.size, 0); assert.equal(host.timers.size, 0);
  assert.equal(host.find('lg-markers').dataset.animated, 'false');
  host.frame(12_000); assert.equal(calls, 1);
  for (const observer of host.intersections) observer.setVisible(true);
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(calls, 2); assert.equal(host.frames.size, 1);
  component.destroy();
  assert.equal(host.frames.size, 0); assert.equal(host.timers.size, 0);
  assert.ok(host.intersections.every(observer => observer.disconnected));
});


test('country k=3 pulses are centroid-based and disjoint; smaller countries fold into continent pulses', () => {
  const host = browserHost({ reduced: true });
  const globe = createGlobe(host.canvas, { seed: 'countries' });
  globe.update(today);
  const country = host.find('lg-marker', 'asia:KR');
  const continent = host.find('lg-marker', 'asia');
  assert.equal(country.dataset.count, '3');
  assert.equal(continent.dataset.count, '1');
  assert.equal(country.hidden, false);
  assert.notEqual(country.style.transform, continent.style.transform);
  assert.equal(Number(country.dataset.quality), qualityMean(today.regions[0].quality, 3));
  globe.update({ ...today, total: 2, roles: { validator: { count: 2 }, wallet: { count: 0 }, candidate: { count: 0 }, follower: { count: 0 } }, versions: { '0.7.4': 2 }, regions: [{ ...today.regions[0], count: 2, quality: summarizeQuality([.1, .2]) }] });
  assert.equal(host.find('lg-marker', 'asia:KR'), undefined, 'a disappearing country leaves no hidden country DOM identifier');
  assert.equal(host.find('lg-marker', 'asia').dataset.count, '2');
  globe.destroy();
});

test('component has one two-end gradient and smooth region strips with no founder/tier fields', async () => {
  const host = browserHost({ reduced: true });
  const component = mountLiveGlobe(host.root, { fixture: true, seed: 'gradient', fetch: async () => ({ ok: true, json: async () => today }) });
  await new Promise(resolve => setImmediate(resolve));
  const legend = host.find('lg-quality-labels');
  assert.deepEqual(legend.children.map(el => el.textContent), ['New', 'Long, steady operation']);
  assert.equal(host.descendants(host.root).filter(el => el.className === 'lg-quality-gradient').length, 1);
  const strip = host.find('lg-region', 'asia').children[3];
  assert.ok(strip.children[0].style.maskImage.includes('linear-gradient'));
  assert.ok(strip.getAttribute('aria-label').includes('spread'));
  const country = host.find('lg-country', 'asia:KR');
  const pulse = host.find('lg-marker', 'asia:KR');
  pulse.dispatch('pointerenter'); assert.equal(country.dataset.active, 'true');
  country.dispatch('click'); assert.equal(pulse.dataset.active, 'true');
  for (const el of host.descendants(host.root)) {
    assert.ok(!/founder|창업자|tier/i.test(JSON.stringify({ text: el.textContent, dataset: el.dataset, attributes: [...el.attributes] })));
  }
  component.setLanguage('ko');
  assert.deepEqual(legend.children.map(el => el.textContent), ['새로 합류', '오래·성실하게 운영']);
  component.destroy();
});

test('refreshing the component preserves selection without postponing idle rotation', async () => {
  const host = browserHost();
  const component = mountLiveGlobe(host.root, { fixture: true, seed: 'refresh', fetch: async () => ({ ok: true, json: async () => today }) });
  await new Promise(resolve => setImmediate(resolve));
  host.frame();
  const row = host.find('lg-region', 'asia');
  row.dispatch('click');
  host.frame(9_900);
  component.setLanguage('ko');
  const marker = host.find('lg-marker', 'asia');
  const held = marker.style.transform;
  host.frame(200);
  assert.notEqual(marker.style.transform, held);
  assert.equal(row.dataset.active, 'true');
  component.destroy();
});

test('country keyboard focus and button identity survive language changes and live refreshes', async () => {
  const host = browserHost();
  let requests = 0;
  const component = mountLiveGlobe(host.root, {
    endpoint: 'https://rpc.example.invalid', seed: 'focus',
    fetch: async () => { requests++; return { ok: true, json: async () => ({ jsonrpc: '2.0', id: 1, result: today }) }; },
  });
  await new Promise(resolve => setImmediate(resolve));
  const country = host.find('lg-country', 'asia:KR');
  const button = country.children[0];
  button.focus();
  component.setLanguage('ko');
  host.frame(10_000);
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(requests, 2);
  assert.equal(host.find('lg-country', 'asia:KR'), country);
  assert.equal(country.children[0], button);
  assert.equal(host.doc.activeElement, button);
  assert.ok(host.descendants(host.root).includes(button));
  component.destroy();
});

test('native host never fetches or polls, including fixtures, visibility changes and endpoint input', async () => {
  for (const fixture of [false, true]) {
    const host = browserHost({ intersection: true });
    let calls = 0;
    const component = mountLiveGlobe(host.root, {
      host: true, fixture, endpoint: 'https://rpc.example.invalid', seed: 'offline',
      fetch: () => { calls++; throw new Error('Native host must never fetch'); },
    });
    assert.equal(host.find('lg-status').dataset.state, 'loading');
    assert.equal(component.update(today), true);
    for (const observer of host.intersections) observer.setVisible(true);
    host.frame(30_000);
    host.doc.hidden = true; host.doc.dispatch('visibilitychange');
    for (const observer of host.intersections) observer.setVisible(false);
    host.doc.hidden = false; host.doc.dispatch('visibilitychange');
    for (const observer of host.intersections) observer.setVisible(true);
    component.configure({ state: 'stale', fixture: false });
    host.frame(30_000);
    await new Promise(resolve => setImmediate(resolve));
    assert.equal(calls, 0);
    assert.equal(host.timers.size, 0);
    component.destroy();
    assert.equal(host.frames.size, 0);
  }
});

test('native states distinguish unavailable from a retained stale snapshot and recover on valid update', () => {
  const host = browserHost();
  const component = mountLiveGlobe(host.root, { host: true, seed: 'states' });
  assert.equal(host.find('lg-total').textContent, '—');
  component.configure({ state: 'stale' });
  assert.equal(host.find('lg-status').dataset.state, 'unavailable');
  assert.equal(component.update({ total: -1, secret: 'never show this RPC error' }), false);
  assert.equal(host.find('lg-status').textContent, 'Live counts are unavailable from this Mac’s node.');
  assert.equal(component.update(today), true);
  assert.equal(host.find('lg-status').dataset.state, 'live');
  assert.equal(host.find('lg-total').textContent, '4');
  assert.ok(host.find('lg-role-summary').textContent.includes('Candidate nodes 0 · Follower nodes 0'));
  component.configure({ state: 'stale' });
  assert.equal(host.find('lg-status').dataset.state, 'stale');
  assert.equal(host.find('lg-total').textContent, '4');
  assert.equal(component.update({ ...today, total: 99 }), false);
  assert.equal(host.find('lg-total').textContent, '4');
  assert.equal(component.update(today), true);
  assert.equal(host.find('lg-status').dataset.state, 'live');
  assert.ok(!host.find('lg-status').textContent.includes('10 seconds'));
  component.destroy();
  assert.equal(component.update(today), false);
  assert.equal(component.configure({ paused: true }), false);
});

test('native reset removes the previous network snapshot and selection before accepting a fresh network', () => {
  const host = browserHost();
  let calls = 0;
  const component = mountLiveGlobe(host.root, {
    host: true, seed: 'network-switch', fetch: () => { calls++; },
  });
  component.update(example);
  host.find('lg-country', 'asia:KR').dispatch('click');
  assert.equal(host.find('lg-country', 'asia:KR').dataset.active, 'true');
  assert.equal(host.find('lg-total').textContent, '24');
  host.doc.hidden = true; host.doc.dispatch('visibilitychange');
  assert.equal(component.reset(), true);
  assert.equal(host.find('lg-status').dataset.state, 'loading');
  assert.equal(host.find('lg-total').textContent, '—');
  assert.equal(host.find('lg-role-summary').hidden, true);
  assert.equal(host.find('lg-role-summary').textContent, '');
  assert.equal(host.find('lg-country', 'asia:KR'), undefined);
  assert.equal(host.find('lg-country', 'europe:DE'), undefined);
  assert.equal(host.find('lg-marker', 'asia:KR'), undefined);
  for (const element of host.descendants(host.root)) {
    if (element.className === 'lg-region') {
      assert.equal(element.dataset.count, '');
      assert.equal(element.dataset.quality, '');
      assert.equal(element.dataset.populated, 'false');
      assert.equal(element.dataset.active, 'false');
      assert.equal(element.dataset.visibility, 'empty');
      assert.equal(element.children[1].textContent, '—');
      assert.equal(element.children[2].hidden, true);
      const strip = element.children[3];
      assert.equal(strip.hidden, true);
      assert.equal(strip.getAttribute('aria-label'), null);
      assert.equal(strip.children[0].style.maskImage, '');
      assert.equal(strip.children[1].style.left, '');
    }
    if (element.className === 'lg-marker') {
      assert.equal(element.hidden, true);
      assert.equal(element.dataset.count, '0');
      assert.equal(element.dataset.active, 'false');
    }
  }
  component.configure({ state: 'unavailable' });
  assert.equal(host.find('lg-total').textContent, '—');
  host.doc.hidden = false; host.doc.dispatch('visibilitychange');
  component.update({ ...today, regions: [{ continent: 'north_america', count: 4, quality: summarizeQuality([.1, .2, .3, .4]) }] });
  assert.equal(host.find('lg-status').dataset.state, 'live');
  assert.equal(host.find('lg-total').textContent, '4');
  assert.equal(host.find('lg-marker', 'north_america').hidden, false);
  assert.equal(host.find('lg-marker', 'north_america').style.transform, 'translate3d(300.00px,300.00px,0)', 'a new network gets a new opening region');
  assert.equal(host.find('lg-region', 'asia').dataset.populated, 'false');
  assert.equal(calls, 0);
  assert.equal(host.timers.size, 0);
  component.destroy();
  assert.equal(component.reset(), false);
});

test('native capture draws one still frame while hidden and offscreen without restarting animation', () => {
  for (const options of [{}, { reduced: true }, { webgl: false }]) {
    const host = browserHost({ ...options, intersection: true });
    let calls = 0;
    const component = mountLiveGlobe(host.root, {
      host: true, seed: 'capture', fetch: () => { calls++; },
    });
    component.configure({ paused: true });
    for (const observer of host.intersections) observer.setVisible(false);
    host.doc.hidden = true; host.doc.dispatch('visibilitychange');
    assert.equal(component.update(example), true);
    assert.equal(host.frames.size, 0);
    host.renders.length = 0;
    assert.equal(component.captureFrame(), true);
    assert.deepEqual(host.renders, [options.reduced || options.webgl === false ? 'map' : 'webgl']);
    assert.equal(host.find('lg-total').textContent, '24');
    assert.equal(host.find('lg-region', 'asia').dataset.count, '9');
    assert.equal(host.find('lg-pause').getAttribute('aria-pressed'), 'true');
    assert.equal(host.find('lg-markers').dataset.animated, 'false');
    assert.equal(host.doc.hidden, true);
    assert.equal(host.frames.size, 0);
    assert.equal(host.timers.size, 0);
    host.frame(30_000);
    assert.equal(host.renders.length, 1, 'capture never schedules another rendering frame');
    assert.equal(calls, 0);
    component.destroy();
    assert.equal(component.captureFrame(), false);
  }
});

test('native aggregate updates strip identities and fold countries below k=3 before rendering', () => {
  const host = browserHost();
  const component = mountLiveGlobe(host.root, { host: true, seed: 'boundary' });
  const payload = {
    ...today, total: 2, versions: { '0.7.4': 2 },
    roles: { validator: { count: 2 }, wallet: { count: 1 }, candidate: { count: 0 }, follower: { count: 0 } },
    regions: [{ continent: 'asia', country: 'KR', count: 2, quality: summarizeQuality([.1, .2]), address: '<private-address>' }],
    peer_ids: ['<private-peer>'], rpc_url: 'https://private-node.invalid',
  };
  assert.equal(component.update(payload), true);
  assert.equal(host.find('lg-marker', 'asia:KR'), undefined);
  assert.equal(host.find('lg-country', 'asia:KR'), undefined);
  assert.equal(host.find('lg-region', 'asia').dataset.count, '2');
  const content = JSON.stringify(host.descendants(host.root).map(el => ({
    text: el.textContent, dataset: el.dataset, attributes: [...el.attributes],
  })));
  assert.ok(!/private-address|private-peer|private-node|"country":"KR"/.test(content));
  let invoked = false;
  const bad = { ...today };
  Object.defineProperty(bad, 'roles', { get() { invoked = true; return today.roles; } });
  assert.equal(component.update(bad), false);
  assert.equal(invoked, false, 'the bridge does not execute response getters');
  assert.equal(host.find('lg-total').textContent, '2');
  component.destroy();
});

test('native pause and Reduce Motion settings stop frames and pulses, preserving system preferences', () => {
  const host = browserHost();
  const component = mountLiveGlobe(host.root, { host: true, seed: 'settings' });
  component.update(today);
  assert.equal(host.frames.size, 1);
  assert.equal(component.configure({ paused: true }), true);
  assert.equal(host.frames.size, 0);
  assert.equal(host.find('lg-markers').dataset.animated, 'false');
  host.find('lg-pause').dispatch('click');
  assert.equal(host.frames.size, 0, 'a user control cannot override the host pause');
  component.configure({ paused: false, reducedMotion: true });
  assert.equal(host.frames.size, 0);
  assert.equal(host.root.dataset.reducedMotion, 'true');
  assert.equal(host.find('lg-canvas').dataset.renderer, 'map');
  component.configure({ reducedMotion: false });
  assert.equal(host.frames.size, 1);
  host.motion.matches = true; host.motion.dispatch('change');
  component.configure({ reducedMotion: false });
  assert.equal(host.frames.size, 0, 'native settings cannot override the system Reduce Motion preference');
  host.motion.matches = false; host.motion.dispatch('change');
  assert.equal(host.frames.size, 1);
  component.configure({ theme: 'dark' });
  assert.equal(host.doc.documentElement.dataset.theme, 'dark');
  component.configure({ theme: 'light' });
  assert.equal(host.doc.documentElement.dataset.theme, 'light');
  assert.equal(component.configure({ paused: 'false', theme: 'remote-theme' }), false);
  assert.equal(host.frames.size, 1);
  component.destroy();
});

test('native missing quality evidence remains neutral and explicitly unavailable in all five languages', () => {
  const host = browserHost();
  const component = mountLiveGlobe(host.root, { host: true, seed: 'evidence' });
  component.update(today);
  component.configure({ evidenceAvailable: false });
  assert.ok(!host.find('lg-role-summary').textContent.includes('Reserve keys'));
  for (const lang of ['en', 'ko', 'ja', 'zh-Hans', 'es']) {
    assert.equal(component.configure({ lang }), true);
    assert.equal(host.root.lang, lang);
    assert.equal(host.find('lg-art-legend').hidden, true);
    assert.equal(host.find('lg-quality-status').hidden, false);
    assert.ok(host.find('lg-quality-status').textContent.length > 20);
    assert.equal(host.find('lg-region', 'asia').children[3].hidden, true);
    assert.ok(!host.find('lg-marker', 'asia').getAttribute('aria-label').includes('/ 100'));
    assert.ok(!host.find('lg-region-button', 'asia').getAttribute('aria-label').includes('/ 100'));
    assert.equal(host.find('lg-marker', 'asia').style['--marker-color'], '#94A4B5');
    assert.equal(host.find('lg-total').textContent, '4');
  }
  component.configure({ evidenceAvailable: true, lang: 'en' });
  assert.ok(host.find('lg-role-summary').textContent.includes('Reserve keys 3 (standby 3 / seated 0)'));
  assert.equal(host.find('lg-art-legend').hidden, false);
  assert.equal(host.find('lg-quality-status').hidden, true);
  assert.equal(host.find('lg-region', 'asia').children[3].hidden, false);
  assert.ok(host.find('lg-marker', 'asia').getAttribute('aria-label').includes('/ 100'));
  component.configure({ fixture: true });
  assert.equal(host.find('lg-status').dataset.state, 'fixture');
  assert.equal(host.find('lg-status').textContent, 'Screenshot fixture · not live');
  assert.equal(host.find('lg-snapshot-date').hidden, true);
  component.destroy();
});

test('wallet entry offers a ready inbound-only API with measured height and localized title', () => {
  const host = browserHost({ reduced: true });
  const api = installWalletHost(host.root);
  assert.deepEqual(Object.keys(api).sort(), ['captureFrame', 'configure', 'height', 'ready', 'reset', 'update']);
  assert.equal(api.ready, true);
  assert.equal(host.root.dataset.ready, 'true');
  assert.equal(Object.isFrozen(api), true);
  assert.equal(api.height(), 600);
  assert.equal(api.configure({ lang: 'ko', theme: 'dark', fixture: true }), true);
  assert.equal(host.doc.title, '네트워크 · EastSea');
  assert.equal(host.doc.documentElement.lang, 'ko');
  assert.equal(api.update(today), true);
  assert.equal(host.find('lg-status').dataset.state, 'fixture');
  assert.equal(api.captureFrame(), true);
  assert.equal(api.reset(), true);
  assert.equal(host.find('lg-status').dataset.state, 'loading');
  assert.equal(host.find('lg-total').textContent, '—');
  assert.equal(host.timers.size, 0);
  assert.equal(host.frames.size, 0);
});
