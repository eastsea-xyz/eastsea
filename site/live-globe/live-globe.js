import { CONTINENTS, continentTotals, normalizePresence, regionKey, requestPresence } from './data.js';
import { qualityMean, qualityDensity, QUALITY_BINS } from './quality.js';
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
    privacy: 'Continents follow the home relay. Country comes from the Mac’s region setting, on by default with a first-launch notice; it can be turned off in Settings. Country groups appear at 3 Macs or more.',
    artwork: 'Dot size = Macs connected; countries at 3 Macs, otherwise continents',
    qualityNew: 'New', qualitySteady: 'Long, steady operation',
    quality: 'Operation quality', spread: 'spread',
    validators: 'Validators', wallet: 'Wallet nodes',
    reserve: 'Reserve keys', standby: 'standby', seated: 'seated',
    visibility: { front: 'Front side of globe', back: 'Far side of globe · highlighted here', unknown: 'Continent unknown · list only', empty: '' },
    drag: 'Drag horizontally or use arrow keys to rotate.',
    map: 'Static map · reduced motion or WebGL unavailable',
    pause: 'Pause globe', resume: 'Resume globe',
    canvas: 'Globe of country and continent groups, colored by operation quality. Complete counts and quality spreads are in the list beside it.',
    list: 'Macs by country and continent',
    continents: ['Africa', 'Asia', 'Europe', 'North America', 'South America', 'Oceania', 'Antarctica', 'Region unknown'],
    country: 'Country from the Mac’s region setting',
  },
  ko: {
    caption: '이 노드가 보고 있는 Mac들',
    loading: '연결 현황을 불러오는 중…',
    live: '실시간 현황 · 10초마다 새로고침',
    fixture: '오늘 기준 실제 구성 (실시간 아님)',
    unavailable: '연결 수를 불러올 수 없습니다. 이 노드가 아직 현황을 제공하지 않을 수 있습니다. 10초마다 다시 확인합니다.',
    stale: '마지막으로 받은 현황 · 새로고침에 실패했습니다. 10초마다 다시 확인합니다.',
    empty: '지금 이 노드가 보고 있는 Mac은 없습니다.',
    privacy: '대륙은 홈 릴레이를 따릅니다. 국가는 Mac의 지역 설정에서 가져오며, 처음 실행할 때 안내하고 기본으로 켭니다. 설정에서 끌 수 있고, 같은 국가의 Mac이 3대 이상일 때만 표시합니다.',
    artwork: '점 크기 = 연결된 Mac 수, 3대 이상은 국가별 · 나머지는 대륙별',
    qualityNew: '새로 합류', qualitySteady: '오래·성실하게 운영',
    quality: '운영 품질', spread: '분포',
    validators: '검증자', wallet: '지갑 노드',
    reserve: '예비 키', standby: '대기', seated: '참여',
    visibility: { front: '지구본 앞면', back: '지구본 뒷면 · 목록에서 확인', unknown: '대륙 미상 · 목록에서만 표시', empty: '' },
    drag: '가로로 끌거나 방향키로 지구본을 돌려 보세요.',
    map: '정적인 지도 · 동작 줄이기 또는 WebGL 미지원',
    pause: '지구본 멈추기', resume: '지구본 다시 돌리기',
    canvas: '국가와 대륙별 연결 수를 운영 품질의 색으로 표시한 지구본. 모든 수치와 품질 분포는 옆 목록에서 확인할 수 있습니다.',
    list: '국가·대륙별 Mac 수',
    continents: ['아프리카', '아시아', '유럽', '북아메리카', '남아메리카', '오세아니아', '남극', '지역 미상'],
    country: 'Mac의 지역 설정에서 가져온 국가',
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
  const legend = element(doc, 'div', 'lg-art-legend');
  const gradient = element(doc, 'span', 'lg-quality-gradient');
  gradient.setAttribute('aria-hidden', 'true');
  const legendLabels = element(doc, 'p', 'lg-quality-labels');
  const newLabel = element(doc, 'span');
  const steadyLabel = element(doc, 'span');
  legendLabels.append(newLabel, steadyLabel);
  legend.append(gradient, legendLabels);
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
    const strip = qualityStrip();
    const countries = element(doc, 'dd', 'lg-countries');
    row.append(name, value, position, strip, countries);
    list.append(row);
    rows.set(code, { row, button, value, position, strip, countries });
    row.addEventListener('pointerenter', () => highlight(code));
    row.addEventListener('pointerleave', () => {
      if (highlighted === code && doc.activeElement !== button) highlight(null);
    });
    button.addEventListener('focus', () => highlight(code));
    button.addEventListener('blur', () => {
      if (highlighted === code) highlight(null);
    });
    row.addEventListener('click', event => {
      if (!event.target?.closest('.lg-country')) highlight(code);
    });
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

  function qualityStrip(tag = 'dd') {
    const strip = element(doc, tag, 'lg-quality-strip');
    const density = element(doc, 'span', 'lg-quality-density');
    const mean = element(doc, 'span', 'lg-quality-mean');
    strip.setAttribute('role', 'img');
    strip.append(density, mean);
    return strip;
  }

  function updateStrip(strip, region) {
    strip.hidden = !region?.count;
    if (!region?.count) return;
    const mean = qualityMean(region.quality, region.count);
    const bins = region.quality.histogram;
    const lower = bins.findIndex(value => value > 0) / QUALITY_BINS * 100;
    const upper = (bins.findLastIndex(value => value > 0) + 1) / QUALITY_BINS * 100;
    const density = qualityDensity(region.quality).map((value, i) => `rgba(0,0,0,${(.16 + .84 * value).toFixed(3)}) ${i * 100 / QUALITY_BINS}%`).join(',');
    strip.children[0].style.maskImage = `linear-gradient(to right,${density})`;
    strip.children[1].style.left = `${mean * 100}%`;
    strip.setAttribute('aria-label', `${COPY[language].quality} ${(mean * 100).toFixed(1)} / 100 · ${COPY[language].spread} ${lower}–${upper} / 100`);
  }

  function highlight(code, interaction = true) {
    highlighted = rows.has(code) && !rows.get(code).button.disabled ? code : null;
    for (const [continent, item] of rows) {
      const active = Boolean(continent === highlighted || (highlighted?.includes(':') && continent === highlighted.split(':')[0]));
      item.row.dataset.active = String(active);
      item.button.setAttribute('aria-pressed', String(active));
    }
    globe?.setHighlight(highlighted, { interaction });
  }

  function rowPosition(code) {
    const item = rows.get(code);
    if (!item) return;
    let position = visibility.get(code) || 'empty';
    if (!code.includes(':') && model) {
      const states = model.regions.filter(region => region.continent === code)
        .map(region => visibility.get(regionKey(region)) || 'empty');
      if (states.includes('front')) position = 'front';
      else if (states.includes('back')) position = 'back';
      else if (code === 'unknown' && states.length) position = 'unknown';
    }
    const copy = COPY[language];
    item.row.dataset.visibility = position;
    item.position.textContent = copy.visibility[position];
    item.position.hidden = position === 'empty';
    if (model) {
      const numbers = new Intl.NumberFormat(language);
      const amount = numbers.format(Number(item.row.dataset.count));
      const population = language === 'ko' ? `Mac ${amount}대` : `${amount} Macs`;
      item.button.setAttribute('aria-label', `${item.button.textContent}, ${population}, ${copy.quality} ${(Number(item.row.dataset.quality) * 100).toFixed(1)} / 100${position === 'empty' ? '' : `, ${copy.visibility[position]}`}`);
    } else item.button.removeAttribute('aria-label');
  }

  function updateVisibility(entries) {
    for (const entry of entries) {
      if (!Object.hasOwn(COPY.en.visibility, entry.visibility)) continue;
      visibility.set(entry.key, entry.visibility);
      rowPosition(entry.key);
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
    newLabel.textContent = copy.qualityNew;
    steadyLabel.textContent = copy.qualitySteady;
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
      const role = (label, item) => `${label} ${numbers.format(item.count)}`;
      const reserve = model.reserve_keys;
      roleSummary.textContent = `${role(copy.validators, model.roles.validator)} · ${role(copy.wallet, model.roles.wallet)} · ${copy.reserve} ${numbers.format(reserve.standby + reserve.seated)} (${copy.standby} ${numbers.format(reserve.standby)} / ${copy.seated} ${numbers.format(reserve.seated)})`;
    }
    const totals = new Map((model ? continentTotals(model) : []).map(item => [item.continent, item]));
    let countries;
    try { countries = new Intl.DisplayNames([language], { type: 'region' }); } catch { /* older browsers use ISO codes */ }
    const regionLabels = {};
    const countryKeys = new Set((model?.regions || []).filter(region => region.country).map(regionKey));
    for (const [key, item] of rows) if (key.includes(':') && !countryKeys.has(key)) {
      item.row.remove(); rows.delete(key); visibility.delete(key);
    }
    CONTINENTS.forEach((code, index) => {
      const row = rows.get(code);
      const total = totals.get(code);
      row.button.textContent = copy.continents[index];
      row.button.disabled = !total?.count;
      row.row.dataset.populated = String(Boolean(total?.count));
      row.row.dataset.count = total ? String(total.count) : '';
      row.row.dataset.quality = total ? String(qualityMean(total.quality, total.count)) : '';
      row.value.textContent = total ? numbers.format(total.count) : '—';
      updateStrip(row.strip, total);
      if (!total?.count) visibility.set(code, 'empty');
      else if (code === 'unknown') visibility.set(code, 'unknown');
      rowPosition(code);
      for (const region of model?.regions || []) {
        if (region.continent !== code || !region.country) continue;
        const key = regionKey(region);
        const label = countries?.of(region.country) || region.country;
        regionLabels[key] = label;
        let country = rows.get(key);
        if (!country) {
          const countryRow = element(doc, 'div', 'lg-country');
          countryRow.dataset.region = key;
          countryRow.dataset.continent = code;
          countryRow.dataset.country = region.country;
          const button = element(doc, 'button', 'lg-country-button');
          button.type = 'button';
          const value = element(doc, 'span', 'lg-region-value');
          const position = element(doc, 'span', 'lg-region-position');
          const strip = qualityStrip('span');
          countryRow.append(button, value, position, strip);
          country = { row: countryRow, button, value, position, strip };
          rows.set(key, country);
          row.countries.append(countryRow);
          countryRow.addEventListener('pointerenter', () => highlight(key));
          countryRow.addEventListener('pointerleave', () => { if (doc.activeElement !== button) highlight(null); });
          countryRow.addEventListener('click', event => { event.stopPropagation?.(); highlight(key); });
          button.addEventListener('focus', () => highlight(key));
          button.addEventListener('blur', () => highlight(null));
          button.addEventListener('keydown', event => { if (event.key === 'Escape') highlight(null); });
        }
        country.row.dataset.count = String(region.count);
        country.row.dataset.quality = String(qualityMean(region.quality, region.count));
        country.button.textContent = label;
        country.button.title = copy.country;
        country.value.textContent = numbers.format(region.count);
        updateStrip(country.strip, region);
        rowPosition(key);
      }
      row.countries.hidden = !row.countries.childElementCount;
    });
    highlight(highlighted, false);
    globe.setLabels({
      continents: Object.fromEntries(CONTINENTS.map((code, index) => [code, copy.continents[index]])),
      regions: regionLabels,
      quality: copy.quality,
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
