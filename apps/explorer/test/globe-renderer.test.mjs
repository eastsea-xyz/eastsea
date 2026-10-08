// Exercise real renderer/component behavior with a deterministic browser host.
// This verifies projection and lifecycle logic; it does not replace raster QA.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { createGlobe } from '../live-globe/globe.js';
import { mountLiveGlobe } from '../live-globe/live-globe.js';

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
  const frames = new Map(), timers = new Map(), uploads = [];
  const intersections = [];
  const motion = Object.assign(new Events(), { matches: reduced });
  const context = new Proxy({}, { get: (target, key) => target[key] ?? (() => {}) });
  const gl = new Proxy({
    createShader: () => ({}), createProgram: () => ({}), createBuffer: () => ({}),
    getShaderParameter: () => true, getProgramParameter: () => true,
    getAttribLocation: (_program, name) => name, getUniformLocation: (_program, name) => name,
    getParameter: () => [1, 64], bufferData: (_target, data) => uploads.push(Array.from(data)),
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
  const root = new Element('div'), stage = new Element('div'), canvas = new Element('canvas');
  stage.append(canvas); root.append(stage);
  const descendants = element => [element, ...element.children.flatMap(descendants)];
  const find = (className, continent) => descendants(root).find(el => el.className?.split(' ').includes(className) && (!continent || el.dataset.continent === continent));
  function frame(milliseconds = 16) {
    now += milliseconds;
    for (const [key, timer] of [...timers]) if (timer.at <= now) { timers.delete(key); timer.callback(); }
    const pending = [...frames.values()]; frames.clear();
    for (const callback of pending) callback(now);
  }
  return { root, stage, canvas, doc, motion, frame, find, descendants, frames, timers, uploads, intersections };
}

test('largest-region opening centers its labeled marker, regardless of hemisphere', () => {
  for (const continent of ['asia', 'north_america', 'oceania', 'europe']) {
    const host = browserHost();
    const globe = createGlobe(host.canvas, { seed: 'test-session' });
    globe.setLabels({ continents: { [continent]: continent }, founder: 'Founder', independent: 'Independent' });
    globe.update({ ...today, regions: [{ continent, count: 4, founder_operated: 4 }] });
    const marker = host.find('lg-marker', continent);
    assert.equal(marker.hidden, false);
    assert.equal(marker.style.transform, 'translate3d(300.00px,300.00px,0)');
    assert.equal(host.find('lg-marker-label', undefined).tagName, 'span');
    assert.equal(marker.children[1].textContent, `${continent} 4 · Founder 4`);
    globe.destroy();
  }
});

test('every populated region has a front marker or an explicit list-only visibility state', () => {
  for (const reduced of [false, true]) {
    const host = browserHost({ reduced });
    let visibility;
    const globe = createGlobe(host.canvas, { seed: 'coverage', onVisibility: entries => { visibility = entries; } });
    const snapshot = { ...example, total: 27, versions: { '0.7.4': 27 }, regions: [
      ...example.regions, { continent: 'africa', count: 2, founder_operated: 0 },
      { continent: 'antarctica', count: 1, founder_operated: 0 },
    ] };
    globe.update(snapshot);
    for (let angle = 0; angle < 25; angle++) {
      for (const { continent, visibility: state } of visibility) {
        const marker = host.find('lg-marker', continent);
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

test('gold core area and independent ring follow explicit founder share; pulse diameter uses sqrt count', () => {
  const host = browserHost();
  const globe = createGlobe(host.canvas, { seed: 1 });
  globe.update(example);
  const asia = host.find('lg-marker', 'asia'), europe = host.find('lg-marker', 'europe');
  const core = asia.children[0].children[1], ring = asia.children[0].children[2];
  assert.ok(Math.abs(Number(core.getAttribute('r')) ** 2 / 225 - 4 / 9) < 1e-10);
  assert.ok(Math.abs(Number(ring.getAttribute('stroke-dasharray').split(' ')[0]) / 100 - 5 / 9) < 1e-10);
  assert.equal(europe.children[0].children[1].getAttribute('r'), '0');
  assert.equal(europe.children[0].children[2].getAttribute('stroke-dasharray'), '100 100');
  assert.equal(asia.style['--marker-size'], '40px');
  assert.ok(parseFloat(europe.style['--marker-size']) < 40);
  globe.destroy();
});

test('all-region mobile static-map labels remain inside the stage without overlapping', () => {
  const host = browserHost({ reduced: true, width: 350, height: 350, labelDimensions: { width: 150, height: 36 } });
  const globe = createGlobe(host.canvas, { seed: 'mobile-map' });
  const snapshot = { ...example, total: 27, versions: { '0.7.4': 27 }, regions: [
    ...example.regions, { continent: 'africa', count: 2, founder_operated: 0 },
    { continent: 'antarctica', count: 1, founder_operated: 0 },
  ] };
  globe.update(snapshot);
  const labels = host.descendants(host.root).filter(el => el.className === 'lg-marker' && !el.hidden).map(marker => {
    const [x, y] = marker.style.transform.match(/[\d.-]+(?=px)/g).map(Number);
    const label = marker.children[1];
    return { x: x - 22 + parseFloat(label.style.left), y: y - 22 + parseFloat(label.style.top), w: 150, h: 36, code: marker.dataset.continent };
  });
  assert.equal(labels.length, 7);
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
  assert.equal(host.find('lg-marker', 'asia').dataset.count, '4');
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
  assert.equal(host.find('lg-role-summary').textContent, '검증자 4 (창업자 운영 4) · 지갑 노드 3 (창업자 운영 2) · 예비 키 3 (대기 3 / 참여 0)');
  const pulse = host.find('lg-marker', 'asia'), row = host.find('lg-region', 'asia');
  pulse.dispatch('pointerenter'); assert.equal(row.dataset.active, 'true');
  pulse.dispatch('pointerleave'); assert.equal(row.dataset.active, 'false');
  row.dispatch('click'); assert.equal(pulse.dataset.active, 'true');
  assert.equal(row.dataset.count, '4');
  assert.equal(row.dataset.founderOperated, '4');
  assert.equal(host.find('lg-country').textContent, '대한민국 · 3');
  component.setLanguage('en');
  assert.equal(host.find('lg-status').textContent, 'Today’s actual setup (not live)');
  assert.equal(pulse.children[1].textContent, 'Asia 4 · founder 4');
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
