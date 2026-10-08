import { CONTINENTS, continentTotals, normalizePresence, requestPresence } from './data.js';
import { createGlobe } from './globe.js';

const COPY = {
  en: {
    caption: 'Macs this node can see',
    loading: 'Looking for a live snapshot…',
    live: 'Live snapshot · refreshes every 10 seconds',
    fixture: 'Today’s actual setup (not live)',
    unavailable: 'Live counts are unavailable. This node may not support presence yet. Retrying every 10 seconds.',
    stale: 'Last received snapshot · the latest refresh failed. Retrying every 10 seconds.',
    empty: 'This node currently sees no Macs.',
    privacy: 'Grouped by home relay continent, not a Mac’s location. Countries appear only by opt-in, with at least 3 Macs.',
    artwork: 'Dot size = Macs connected; placed per continent',
    founderLegend: 'Gold = founder-operated nodes',
    independentLegend: 'Ring = independent nodes',
    founder: 'founder', independent: 'independent',
    validators: 'Validators', wallet: 'Wallet nodes', founderRun: 'founder-run',
    reserve: 'Reserve keys', standby: 'standby', seated: 'seated',
    visibility: { front: 'Front side of globe', back: 'Far side of globe · highlighted here', unknown: 'Continent unknown · list only', empty: '' },
    drag: 'Drag horizontally or use arrow keys to rotate.',
    map: 'Static map · reduced motion or WebGL unavailable',
    pause: 'Pause globe', resume: 'Resume globe',
    canvas: 'Globe of continent totals. The complete counts are in the list beside it.',
    list: 'Macs by continent',
    continents: ['Africa', 'Asia', 'Europe', 'North America', 'South America', 'Oceania', 'Antarctica', 'Region unknown'],
    country: 'Country shared by opt-in',
  },
  ko: {
    caption: '이 노드가 보고 있는 Mac들',
    loading: '연결 현황을 불러오는 중…',
    live: '실시간 현황 · 10초마다 새로고침',
    fixture: '오늘 기준 실제 구성 (실시간 아님)',
    unavailable: '연결 수를 불러올 수 없습니다. 이 노드가 아직 현황을 제공하지 않을 수 있습니다. 10초마다 다시 확인합니다.',
    stale: '마지막으로 받은 현황 · 새로고침에 실패했습니다. 10초마다 다시 확인합니다.',
    empty: '지금 이 노드가 보고 있는 Mac은 없습니다.',
    privacy: 'Mac의 위치가 아닌 홈 릴레이의 대륙별 집계입니다. 국가는 직접 동의한 Mac이 3대 이상일 때만 표시합니다.',
    artwork: '점 크기 = 연결된 Mac 수, 위치는 대륙 단위',
    founderLegend: '금색 = 창업자 운영 노드',
    independentLegend: '바깥 고리 = 독립 운영 노드',
    founder: '창업자', independent: '독립 운영',
    validators: '검증자', wallet: '지갑 노드', founderRun: '창업자 운영',
    reserve: '예비 키', standby: '대기', seated: '참여',
    visibility: { front: '지구본 앞면', back: '지구본 뒷면 · 목록에서 확인', unknown: '대륙 미상 · 목록에서만 표시', empty: '' },
    drag: '가로로 끌거나 방향키로 지구본을 돌려 보세요.',
    map: '정적인 지도 · 동작 줄이기 또는 WebGL 미지원',
    pause: '지구본 멈추기', resume: '지구본 다시 돌리기',
    canvas: '대륙별 연결 수를 표시한 지구본. 모든 수치는 옆 목록에서 확인할 수 있습니다.',
    list: '대륙별 Mac 수',
    continents: ['아프리카', '아시아', '유럽', '북아메리카', '남아메리카', '오세아니아', '남극', '지역 미상'],
    country: '동의한 국가 정보',
  },
};

function element(doc, tag, className, text) {
  const el = doc.createElement(tag);
  if (className) el.className = className;
  if (text !== undefined) el.textContent = text;
  return el;
}

let pageSessionSeed;
function ephemeralSeed() {
  if (pageSessionSeed) return pageSessionSeed;
  const seed = new Uint32Array(2);
  globalThis.crypto.getRandomValues(seed);
  pageSessionSeed = `${seed[0]}:${seed[1]}`;
  return pageSessionSeed;
}

/** A shared, dependency-free view. No untrusted response is placed in HTML. */
export function mountLiveGlobe(root, {
  endpoint, fixture = false, lang = 'en', seed = ephemeralSeed(),
  fetch = (...args) => globalThis.fetch(...args),
} = {}) {
  const doc = root.ownerDocument;
  const win = doc.defaultView;
  let language = lang === 'ko' ? 'ko' : 'en';
  let model = null;
  let state = 'loading';
  let paused = false;
  let destroyed = false;
  let timer = null;
  let controller = null;
  let visible = !('IntersectionObserver' in win);
  let generation = 0;
  let highlighted = null;
  const motion = win.matchMedia('(prefers-reduced-motion: reduce)');

  root.classList.add('live-globe');
  const figure = element(doc, 'figure', 'lg-figure');
  const stage = element(doc, 'div', 'lg-stage');
  const canvas = element(doc, 'canvas', 'lg-canvas');
  canvas.tabIndex = 0;
  canvas.setAttribute('role', 'img');
  stage.append(canvas);
  const controls = element(doc, 'div', 'lg-controls');
  const pause = element(doc, 'button', 'lg-pause');
  pause.type = 'button';
  pause.setAttribute('aria-pressed', 'false');
  const interaction = element(doc, 'span', 'lg-interaction');
  controls.append(pause, interaction);
  const artCaption = element(doc, 'figcaption', 'lg-art-caption');
  const artwork = element(doc, 'p', 'lg-art-caption-line');
  const legend = element(doc, 'p', 'lg-art-legend');
  const founderLegend = element(doc, 'span', 'lg-legend-founders');
  const independentLegend = element(doc, 'span', 'lg-legend-independent');
  legend.append(founderLegend, doc.createTextNode(' · '), independentLegend);
  artCaption.append(artwork, legend);
  figure.append(stage, controls, artCaption);

  const summary = element(doc, 'div', 'lg-summary');
  const caption = element(doc, 'p', 'lg-caption');
  const count = element(doc, 'strong', 'lg-total', '—');
  const status = element(doc, 'p', 'lg-status');
  status.setAttribute('role', 'status');
  status.setAttribute('aria-live', 'polite');
  status.setAttribute('aria-atomic', 'true');
  const date = element(doc, 'time', 'lg-snapshot-date', '2026-10-08');
  date.dateTime = '2026-10-08';
  const roleSummary = element(doc, 'p', 'lg-role-summary');
  const list = element(doc, 'dl', 'lg-regions');
  const rows = new Map();
  const visibility = new Map(CONTINENTS.map(code => [code, 'empty']));
  for (const code of CONTINENTS) {
    const row = element(doc, 'div', 'lg-region');
    row.dataset.continent = code;
    row.dataset.populated = 'false';
    row.dataset.visibility = 'empty';
    const name = element(doc, 'dt', 'lg-region-name');
    const button = element(doc, 'button', 'lg-region-button');
    button.type = 'button';
    button.disabled = true;
    button.dataset.continent = code;
    button.setAttribute('aria-pressed', 'false');
    name.append(button);
    const value = element(doc, 'dd', 'lg-region-value', '—');
    const position = element(doc, 'dd', 'lg-region-position');
    const countries = element(doc, 'dd', 'lg-countries');
    row.append(name, value, position, countries);
    list.append(row);
    rows.set(code, { row, button, value, position, countries });
    row.addEventListener('pointerenter', () => highlight(code));
    row.addEventListener('pointerleave', () => {
      if (highlighted === code && doc.activeElement !== button) highlight(null);
    });
    button.addEventListener('focus', () => highlight(code));
    button.addEventListener('blur', () => {
      if (highlighted === code) highlight(null);
    });
    row.addEventListener('click', () => highlight(code));
    button.addEventListener('keydown', event => {
      if (event.key === 'Escape') {
        highlight(null);
        return;
      }
      if (!['ArrowDown', 'ArrowUp', 'ArrowRight', 'ArrowLeft', 'Home', 'End'].includes(event.key)) return;
      const available = [...rows.values()].filter(item => !item.button.disabled);
      const current = available.findIndex(item => item.button === button);
      const step = ['ArrowDown', 'ArrowRight'].includes(event.key) ? 1 : -1;
      const next = event.key === 'Home' ? 0 : event.key === 'End' ? available.length - 1
        : (current + step + available.length) % available.length;
      event.preventDefault();
      available[next]?.button.focus();
    });
  }
  const privacy = element(doc, 'p', 'lg-privacy');
  summary.append(caption, count, status, date, roleSummary, list, privacy);
  root.replaceChildren(figure, summary);
  let globe;
  globe = createGlobe(canvas, { seed, onSelect: highlight, onVisibility: updateVisibility });

  function highlight(code) {
    highlighted = rows.has(code) && !rows.get(code).button.disabled ? code : null;
    for (const [continent, item] of rows) {
      const active = continent === highlighted;
      item.row.dataset.active = String(active);
      item.button.setAttribute('aria-pressed', String(active));
    }
    globe?.setHighlight(highlighted);
  }

  function rowPosition(code) {
    const item = rows.get(code);
    const position = visibility.get(code);
    const copy = COPY[language];
    item.row.dataset.visibility = position;
    item.position.textContent = copy.visibility[position];
    item.position.hidden = position === 'empty';
    if (model) {
      const numbers = new Intl.NumberFormat(language);
      const amount = numbers.format(Number(item.row.dataset.count));
      const founder = numbers.format(Number(item.row.dataset.founderOperated));
      const population = language === 'ko' ? `Mac ${amount}대` : `${amount} Macs`;
      item.button.setAttribute('aria-label', `${item.button.textContent}, ${population}, ${copy.founder} ${founder}${position === 'empty' ? '' : `, ${copy.visibility[position]}`}`);
    } else item.button.removeAttribute('aria-label');
  }

  function updateVisibility(entries) {
    for (const entry of entries) {
      if (!rows.has(entry.continent) || !Object.hasOwn(COPY.en.visibility, entry.visibility)) continue;
      visibility.set(entry.continent, entry.visibility);
      rowPosition(entry.continent);
    }
  }

  function text() {
    const copy = COPY[language];
    const numbers = new Intl.NumberFormat(language);
    root.lang = language;
    caption.textContent = copy.caption;
    canvas.setAttribute('aria-label', copy.canvas);
    list.setAttribute('aria-label', copy.list);
    privacy.textContent = copy.privacy;
    artwork.textContent = copy.artwork;
    founderLegend.textContent = copy.founderLegend;
    independentLegend.textContent = copy.independentLegend;
    pause.textContent = paused ? copy.resume : copy.pause;
    pause.setAttribute('aria-pressed', String(paused));
    const isMap = motion.matches || canvas.dataset.renderer === 'map';
    pause.hidden = isMap;
    interaction.textContent = isMap ? copy.map : copy.drag;
    status.textContent = copy[state];
    status.dataset.state = state;
    date.hidden = state !== 'fixture';
    count.textContent = model ? numbers.format(model.total) : '—';
    roleSummary.hidden = !model;
    if (model) {
      const role = (label, item) => `${label} ${numbers.format(item.count)} (${copy.founderRun} ${numbers.format(item.founder_operated)})`;
      const reserve = model.reserve_keys;
      roleSummary.textContent = `${role(copy.validators, model.roles.validator)} · ${role(copy.wallet, model.roles.wallet)} · ${copy.reserve} ${numbers.format(reserve.standby + reserve.seated)} (${copy.standby} ${numbers.format(reserve.standby)} / ${copy.seated} ${numbers.format(reserve.seated)})`;
    }
    const totals = new Map((model ? continentTotals(model) : []).map(item => [item.continent, item]));
    let countries;
    try { countries = new Intl.DisplayNames([language], { type: 'region' }); } catch { /* older browsers use ISO codes */ }
    CONTINENTS.forEach((code, index) => {
      const row = rows.get(code);
      const total = totals.get(code);
      row.button.textContent = copy.continents[index];
      row.button.disabled = !total?.count;
      row.row.dataset.populated = String(Boolean(total?.count));
      row.row.dataset.count = total ? String(total.count) : '';
      row.row.dataset.founderOperated = total ? String(total.founder_operated) : '';
      row.value.textContent = total ? total.count
        ? `${numbers.format(total.count)} · ${copy.founder} ${numbers.format(total.founder_operated)}`
        : '0' : '—';
      if (!total?.count) visibility.set(code, 'empty');
      else if (code === 'unknown') visibility.set(code, 'unknown');
      rowPosition(code);
      row.countries.replaceChildren();
      for (const region of model?.regions || []) {
        if (region.continent !== code || !region.country) continue;
        const label = element(doc, 'span', 'lg-country', `${countries?.of(region.country) || region.country} · ${numbers.format(region.count)}`);
        label.title = copy.country;
        row.countries.append(label);
      }
      row.countries.hidden = !row.countries.childElementCount;
    });
    if (highlighted && rows.get(highlighted).button.disabled) highlight(null);
    globe.setLabels({
      continents: Object.fromEntries(CONTINENTS.map((code, index) => [code, copy.continents[index]])),
      founder: copy.founder,
      independent: copy.independent,
    });
  }

  function apply(snapshot) {
    model = normalizePresence(snapshot);
    state = fixture ? 'fixture' : model.total === 0 ? 'empty' : 'live';
    globe.update(model);
    text();
  }

  function stop() {
    generation++;
    win.clearTimeout(timer);
    timer = null;
    controller?.abort();
    controller = null;
  }

  function canPoll() { return !destroyed && !doc.hidden && visible; }

  async function poll() {
    if (!canPoll() || controller) return;
    const mine = generation;
    const started = win.performance.now();
    const current = new AbortController();
    controller = current;
    const timeout = win.setTimeout(() => current.abort(), 8000);
    try {
      let snapshot;
      if (fixture) {
        const response = await fetch(new URL('./fixture.json', import.meta.url), {
          signal: current.signal, credentials: 'omit', referrerPolicy: 'no-referrer',
        });
        if (!response.ok) throw new Error('Fixture unavailable');
        snapshot = await response.json();
      } else {
        snapshot = await requestPresence(endpoint, { fetch, signal: current.signal });
      }
      if (mine === generation && canPoll()) apply(snapshot);
    } catch {
      if (mine === generation && canPoll()) {
        state = model ? 'stale' : 'unavailable';
        text();
      }
    } finally {
      win.clearTimeout(timeout);
      if (controller === current) controller = null;
      if (mine === generation && canPoll() && !fixture) {
        timer = win.setTimeout(poll, Math.max(0, 10_000 - (win.performance.now() - started)));
      }
    }
  }

  function resume() {
    stop();
    if (canPoll() && (!fixture || !model)) void poll();
  }
  pause.addEventListener('click', () => {
    paused = !paused;
    globe.setPaused(paused);
    text();
  });
  const onMotion = () => text();
  motion.addEventListener('change', onMotion);
  const renderObserver = new win.MutationObserver(text);
  renderObserver.observe(canvas, { attributes: true, attributeFilter: ['data-renderer'] });
  doc.addEventListener('visibilitychange', resume);
  let intersection;
  if ('IntersectionObserver' in win) {
    intersection = new win.IntersectionObserver(entries => {
      const next = entries[0].isIntersecting;
      if (next !== visible) { visible = next; resume(); }
    });
    intersection.observe(root);
  }
  text();
  if (visible) void poll();

  return {
    setLanguage(next) { language = next === 'ko' ? 'ko' : 'en'; text(); },
    destroy() {
      destroyed = true;
      stop();
      doc.removeEventListener('visibilitychange', resume);
      motion.removeEventListener('change', onMotion);
      intersection?.disconnect();
      renderObserver.disconnect();
      globe.destroy();
      root.replaceChildren();
    },
  };
}
