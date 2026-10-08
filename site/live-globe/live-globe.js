import { CONTINENTS, continentTotals, normalizePresence, requestPresence } from './data.js';
import { createGlobe } from './globe.js';

const COPY = {
  en: {
    caption: 'Macs this node can see',
    loading: 'Looking for a live snapshot…',
    live: 'Live snapshot · refreshes every 10 seconds',
    fixture: 'Illustrative snapshot · these are example counts',
    unavailable: 'Live counts are unavailable. This node may not support presence yet. Retrying every 10 seconds.',
    stale: 'Last received snapshot · the latest refresh failed. Retrying every 10 seconds.',
    empty: 'This node currently sees no Macs.',
    privacy: 'Grouped by home relay continent, not a Mac’s location. Countries appear only by opt-in, with at least 3 Macs.',
    artwork: 'Dots show continent totals. Arcs show successive validator regions, not peer connections.',
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
    fixture: '예시 화면 · 실제 연결 수가 아닙니다',
    unavailable: '연결 수를 불러올 수 없습니다. 이 노드가 아직 현황을 제공하지 않을 수 있습니다. 10초마다 다시 확인합니다.',
    stale: '마지막으로 받은 현황 · 새로고침에 실패했습니다. 10초마다 다시 확인합니다.',
    empty: '지금 이 노드가 보고 있는 Mac은 없습니다.',
    privacy: 'Mac의 위치가 아닌 홈 릴레이의 대륙별 집계입니다. 국가는 직접 동의한 Mac이 3대 이상일 때만 표시합니다.',
    artwork: '점은 대륙별 연결 수입니다. 곡선은 최근 블록의 검증자 지역 순서이며, Mac 사이의 연결 경로가 아닙니다.',
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
  figure.append(stage, controls, artCaption);

  const summary = element(doc, 'div', 'lg-summary');
  const caption = element(doc, 'p', 'lg-caption');
  const count = element(doc, 'strong', 'lg-total', '—');
  const status = element(doc, 'p', 'lg-status');
  status.setAttribute('role', 'status');
  status.setAttribute('aria-live', 'polite');
  status.setAttribute('aria-atomic', 'true');
  const list = element(doc, 'dl', 'lg-regions');
  const rows = new Map();
  for (const code of CONTINENTS) {
    const row = element(doc, 'div', 'lg-region');
    const name = element(doc, 'dt', 'lg-region-name');
    const value = element(doc, 'dd', 'lg-region-value', '—');
    const countries = element(doc, 'dd', 'lg-countries');
    row.append(name, value, countries);
    list.append(row);
    rows.set(code, { name, value, countries });
  }
  const privacy = element(doc, 'p', 'lg-privacy');
  summary.append(caption, count, status, list, privacy);
  root.replaceChildren(figure, summary);
  const globe = createGlobe(canvas, { seed });

  function text() {
    const copy = COPY[language];
    const numbers = new Intl.NumberFormat(language);
    root.lang = language;
    caption.textContent = copy.caption;
    canvas.setAttribute('aria-label', copy.canvas);
    list.setAttribute('aria-label', copy.list);
    privacy.textContent = copy.privacy;
    artCaption.textContent = copy.artwork;
    pause.textContent = paused ? copy.resume : copy.pause;
    pause.setAttribute('aria-pressed', String(paused));
    const isMap = motion.matches || canvas.dataset.renderer === 'map';
    pause.hidden = isMap;
    interaction.textContent = isMap ? copy.map : copy.drag;
    status.textContent = copy[state];
    status.dataset.state = state;
    count.textContent = model ? numbers.format(model.total) : '—';
    const totals = model ? continentTotals(model) : [];
    let countries;
    try { countries = new Intl.DisplayNames([language], { type: 'region' }); } catch { /* older browsers use ISO codes */ }
    CONTINENTS.forEach((code, index) => {
      const row = rows.get(code);
      row.name.textContent = copy.continents[index];
      row.value.textContent = model ? numbers.format(totals[index].count) : '—';
      row.countries.replaceChildren();
      for (const region of model?.regions || []) {
        if (region.continent !== code || !region.country) continue;
        const label = element(doc, 'span', 'lg-country', `${countries?.of(region.country) || region.country} · ${numbers.format(region.count)}`);
        label.title = copy.country;
        row.countries.append(label);
      }
      row.countries.hidden = !row.countries.childElementCount;
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
