import { CONTINENTS, GEOGRAPHIES, continentTotals, normalizePresence, presenceRegions, regionKey, requestPresence } from './data.js';
import { qualityMean, QUALITY_VERSION } from './quality.js';
import { createGlobe } from './globe.js';
import { SUBREGION_CODES, SUBREGION_NAMES } from './subregions.js';

const COPY = {
  en: {
    caption: ['', ' Macs connected now'], captionOne: ' Mac connected now',
    loading: 'Looking for a live snapshot…',
    live: 'Live snapshot · refreshes every 10 seconds',
    fixture: 'Today’s actual setup (not live)',
    unavailable: 'Live counts are unavailable. This node may not support presence yet. Retrying every 10 seconds.',
    stale: 'Last received snapshot · the latest refresh failed. Retrying every 10 seconds.',
    empty: 'This node currently sees no Macs.',
    privacy: 'Sub-regions follow UN M49. Country sharing requires an explicit choice. Small groups are merged or withheld; precise locations are never shown.',
    cohortCaption: ['', ' transport observations'],
    cohort: 'Unverified cohort observations · counts released in fixed 10-minute windows',
    cohortStale: 'Last cohort observation · the latest refresh failed.',
    withheld: 'Counts withheld for privacy · groups smaller than three are not published.',
    withheldCategory: 'Withheld or unreported', unknownRole: 'Unknown role', otherRole: 'Other roles',
    allRegions: 'All regions', legacy: 'legacy broad region',
    cohortList: 'Observations by UN M49 sub-region; older broad regions are labeled separately',
    cohortArtwork: 'Dot size = transport observations · unknown and folded regions stay in the list',
    cohortCanvas: 'Unverified regional cohort observations. Complete published counts are in the list. Quality is unavailable.',
    artwork: 'Dot size = connected Macs · continents include their countries',
    qualityNew: 'New', qualitySteady: 'Long, steady operation',
    quality: 'Operation quality',
    qualityUnavailable: 'Operation quality unavailable · this node does not provide measured quality evidence yet.',
    canvasUnavailable: 'Globe of country and continent groups. Complete counts are in the list beside it. Operation quality is unavailable.',
    validators: 'Validators', wallet: 'Wallet nodes',
    candidates: 'Candidate nodes', followers: 'Follower nodes',
    hostLive: 'Live snapshot · from your Mac’s node',
    hostFixture: 'Screenshot fixture · not live',
    hostUnavailable: 'Live counts are unavailable from this Mac’s node.',
    hostStale: 'Last received snapshot · your node’s latest refresh failed.',
    reserve: 'Reserve keys',
    drag: 'Drag horizontally or use arrow keys to rotate.',
    map: 'Static map · reduced motion or WebGL unavailable',
    pause: 'Pause globe', resume: 'Resume globe',
    canvas: 'Globe of country and continent totals, colored by operation quality. Continents include their countries. Complete counts are in the list.',
    list: 'Macs by country and continent',
    continents: ['Africa', 'Asia', 'Europe', 'North America', 'South America', 'Oceania', 'Antarctica', 'Region unknown'],
    country: 'Country from the Mac’s region setting',
  },
  ko: {
    caption: ['지금 연결된 맥 ', '대'],
    loading: '연결 현황을 불러오는 중…',
    live: '실시간 현황 · 10초마다 새로고침',
    fixture: '오늘 기준 실제 구성 (실시간 아님)',
    unavailable: '연결 수를 불러올 수 없습니다. 이 노드가 아직 현황을 제공하지 않을 수 있습니다. 10초마다 다시 확인합니다.',
    stale: '마지막으로 받은 현황 · 새로고침에 실패했습니다. 10초마다 다시 확인합니다.',
    empty: '지금 이 노드가 보고 있는 Mac은 없습니다.',
    privacy: '세부 지역은 UN M49 분류를 따릅니다. 국가는 명시적으로 선택한 경우에만 공유합니다. 작은 그룹은 합치거나 숨기며 정확한 위치는 표시하지 않습니다.',
    cohortCaption: ['노드 연결 관측 ', '건'],
    cohort: '검증되지 않은 관측 그룹 · 수치는 고정된 10분 구간마다 공개됩니다',
    cohortStale: '마지막 관측 그룹 · 최신 새로고침에 실패했습니다.',
    withheld: '개인정보 보호를 위해 수치를 숨겼습니다 · 3개 미만 그룹은 공개하지 않습니다.',
    withheldCategory: '비공개 또는 미보고', unknownRole: '역할 미상', otherRole: '기타 역할',
    allRegions: '모든 지역', legacy: '이전 방식의 넓은 지역',
    cohortList: 'UN M49 세부 지역별 관측 수; 이전 방식의 넓은 지역은 별도로 표시합니다',
    cohortArtwork: '점 크기 = 노드 연결 관측 수 · 미상·통합 지역은 목록에 표시합니다',
    cohortCanvas: '검증되지 않은 지역별 관측 그룹입니다. 공개된 모든 수치는 목록에서 확인합니다. 품질은 제공되지 않습니다.',
    artwork: '점 크기 = 연결된 맥 수 · 대륙에는 해당 나라가 포함됩니다',
    qualityNew: '새로 합류', qualitySteady: '오래·성실하게 운영',
    quality: '운영 품질',
    qualityUnavailable: '운영 품질 미상 · 이 노드가 아직 측정된 품질 근거를 제공하지 않습니다.',
    canvasUnavailable: '국가와 대륙별 연결 수를 표시한 지구본. 모든 수치는 옆 목록에서 확인할 수 있습니다. 운영 품질은 제공되지 않습니다.',
    validators: '검증자', wallet: '지갑 노드',
    candidates: '후보 노드', followers: '팔로어 노드',
    hostLive: '실시간 현황 · 이 Mac의 노드에서 제공',
    hostFixture: '스크린샷 예시 데이터 · 실시간 아님',
    hostUnavailable: '이 Mac의 노드에서 연결 수를 불러올 수 없습니다.',
    hostStale: '마지막으로 받은 현황 · 노드의 새로고침에 실패했습니다.',
    reserve: '예비 키',
    drag: '가로로 끌거나 방향키로 지구본을 돌려 보세요.',
    map: '정적인 지도 · 동작 줄이기 또는 WebGL 미지원',
    pause: '지구본 멈추기', resume: '지구본 다시 돌리기',
    canvas: '국가와 대륙별 연결 수를 운영 품질의 색으로 표시한 지구본. 대륙에는 해당 나라가 포함됩니다. 모든 수치는 목록에서 확인할 수 있습니다.',
    list: '국가·대륙별 Mac 수',
    continents: ['아프리카', '아시아', '유럽', '북아메리카', '남아메리카', '오세아니아', '남극', '지역 미상'],
    country: 'Mac의 지역 설정에서 가져온 국가',
  },
  ja: {
    caption: ['現在接続中のMac ', '台'],
    loading: '接続状況を読み込み中…',
    live: 'ライブ状況 · 10秒ごとに更新',
    fixture: '今日の実際の構成（ライブではありません）',
    unavailable: '接続数を取得できません。このノードはまだ接続状況に対応していない可能性があります。10秒ごとに再試行します。',
    stale: '最後に受信した状況 · 更新に失敗しました。10秒ごとに再試行します。',
    empty: 'このノードから見えるMacは現在ありません。',
    privacy: '地域区分はUN M49に従います。国の共有には明示的な選択が必要です。小さな集団は統合または非表示にし、正確な位置は表示しません。',
    cohortCaption: ['ノード接続の観測 ', '件'],
    cohort: '未検証の観測集団 · 数値は固定の10分区間ごとに公開されます',
    cohortStale: '最後の観測集団 · 最新の更新に失敗しました。',
    withheld: 'プライバシー保護のため数値を非表示 · 3件未満の集団は公開しません。',
    withheldCategory: '非公開または未報告', unknownRole: '役割不明', otherRole: 'その他の役割',
    allRegions: 'すべての地域', legacy: '従来の広域区分',
    cohortList: 'UN M49地域区分ごとの観測数。従来の広域区分は別に表示します',
    cohortArtwork: '点の大きさ = ノード接続の観測数 · 不明・統合地域は一覧に表示します',
    cohortCanvas: '未検証の地域別観測集団。公開された数値は一覧で確認できます。品質は利用できません。',
    artwork: '点の大きさ = 接続中のMacの数 · 大陸には各国を含みます',
    qualityNew: '新規参加', qualitySteady: '長期の安定した運用',
    quality: '運用品質',
    qualityUnavailable: '運用品質は不明 · このノードは測定された品質の根拠をまだ提供していません。',
    canvasUnavailable: '国と大陸ごとの接続数を表示する地球儀。すべての数値は隣の一覧にあります。運用品質は利用できません。',
    validators: '検証者', wallet: 'ウォレットノード',
    candidates: '候補ノード', followers: '追従ノード',
    hostLive: 'ライブ状況 · このMacのノードから取得',
    hostFixture: 'スクリーンショット用のサンプル · ライブではありません',
    hostUnavailable: 'このMacのノードから接続数を取得できません。',
    hostStale: '最後に受信した状況 · ノードの更新に失敗しました。',
    reserve: '予備キー',
    drag: '横にドラッグするか、矢印キーで回転できます。',
    map: '静止地図 · 視差効果を減らす設定またはWebGL非対応',
    pause: '地球儀を停止', resume: '地球儀を再開',
    canvas: '国と大陸ごとの接続数を運用品質の色で表示する地球儀。大陸には各国を含みます。すべての数値は一覧にあります。',
    list: '国と大陸ごとのMac数',
    continents: ['アフリカ', 'アジア', 'ヨーロッパ', '北アメリカ', '南アメリカ', 'オセアニア', '南極', '地域不明'],
    country: 'Macの地域設定から取得した国',
  },
  'zh-Hans': {
    caption: ['当前连接的Mac：', '台'],
    loading: '正在读取连接情况…',
    live: '实时快照 · 每10秒刷新',
    fixture: '今天的实际配置（非实时）',
    unavailable: '无法获取实时数量。此节点可能尚不支持连接情况。每10秒重试。',
    stale: '上次收到的快照 · 最新刷新失败。每10秒重试。',
    empty: '此节点目前看不到任何Mac。',
    privacy: '次区域采用UN M49分类。共享国家需要明确选择。小规模群体会被合并或隐藏，不显示精确位置。',
    cohortCaption: ['节点连接观测：', '条'],
    cohort: '未经验证的观测群体 · 数量按固定的10分钟区间发布',
    cohortStale: '上次观测群体 · 最新刷新失败。',
    withheld: '为保护隐私而隐藏数量 · 少于3条的群体不公开。',
    withheldCategory: '隐藏或未报告', unknownRole: '角色未知', otherRole: '其他角色',
    allRegions: '所有地区', legacy: '旧版大范围地区',
    cohortList: '按UN M49次区域划分的观测数量，旧版大范围地区单独标注',
    cohortArtwork: '点大小 = 节点连接观测数 · 未知和合并地区保留在列表中',
    cohortCanvas: '未经验证的区域观测群体。列表显示全部公开数量，运行质量不可用。',
    artwork: '点的大小 = 已连接的Mac数量 · 大洲包含各国家',
    qualityNew: '新加入', qualitySteady: '长期稳定运行',
    quality: '运行质量',
    qualityUnavailable: '运行质量不可用 · 此节点尚未提供测量的质量依据。',
    canvasUnavailable: '按国家和大洲显示连接数量的地球仪。完整数量见旁边的列表。运行质量不可用。',
    validators: '验证者', wallet: '钱包节点',
    candidates: '候选节点', followers: '跟随节点',
    hostLive: '实时快照 · 来自此Mac的节点',
    hostFixture: '截图示例数据 · 非实时',
    hostUnavailable: '无法从此Mac的节点获取实时数量。',
    hostStale: '上次收到的快照 · 节点的最新刷新失败。',
    reserve: '备用密钥',
    drag: '水平拖动或使用方向键旋转。',
    map: '静态地图 · 已启用减少动态效果或不支持WebGL',
    pause: '暂停地球仪', resume: '继续地球仪',
    canvas: '按运行质量着色的国家和大洲地球仪。大洲包含各国家。完整数量见列表。',
    list: '各国家和大洲的Mac数量',
    continents: ['非洲', '亚洲', '欧洲', '北美洲', '南美洲', '大洋洲', '南极洲', '地区未知'],
    country: '来自Mac地区设置的国家',
  },
  'zh-Hant': {
    caption: ['目前連線的 Mac：', '台'],
    loading: '正在讀取連線狀態…',
    live: '即時快照 · 每 10 秒重新整理',
    fixture: '今天的實際設定（非即時）',
    unavailable: '無法取得即時數量。此節點可能尚未支援連線狀態。每 10 秒重試。',
    stale: '上次收到的快照 · 最新重新整理失敗。每 10 秒重試。',
    empty: '此節點目前看不到任何 Mac。',
    privacy: '子區域依照 UN M49 分類。分享國家需要明確選擇。小型群組會合併或隱藏，不會顯示精確位置。',
    cohortCaption: ['節點連線觀測：', '筆'],
    cohort: '未經驗證的觀測群組 · 數量以固定的 10 分鐘區間公布',
    cohortStale: '上次觀測群組 · 最新重新整理失敗。',
    withheld: '為保護隱私而隱藏數量 · 少於 3 筆的群組不會公布。',
    withheldCategory: '隱藏或未回報', unknownRole: '角色不明', otherRole: '其他角色',
    allRegions: '所有地區', legacy: '舊版廣域分區',
    cohortList: '依 UN M49 子區域分類的觀測數量；舊版廣域分區另行標示',
    cohortArtwork: '點的大小 = 節點連線觀測數 · 不明及合併地區仍列於清單中',
    cohortCanvas: '未經驗證的區域觀測群組。清單顯示所有公開數量，運作品質無法取得。',
    artwork: '點的大小 = 已連線的 Mac 數量 · 大洲包含各國',
    qualityNew: '新加入', qualitySteady: '長期穩定運作',
    quality: '運作品質',
    qualityUnavailable: '無法取得運作品質 · 此節點尚未提供實測的品質依據。',
    canvasUnavailable: '依國家及大洲顯示連線數量的地球儀。完整數量請見旁邊的清單。運作品質無法取得。',
    validators: '驗證者', wallet: '錢包節點',
    candidates: '候選節點', followers: '跟隨節點',
    hostLive: '即時快照 · 來自這台 Mac 的節點',
    hostFixture: '截圖範例資料 · 非即時',
    hostUnavailable: '無法從這台 Mac 的節點取得即時數量。',
    hostStale: '上次收到的快照 · 節點的最新重新整理失敗。',
    reserve: '備用金鑰',
    drag: '水平拖曳或使用方向鍵旋轉。',
    map: '靜態地圖 · 已啟用減少動態效果或未支援 WebGL',
    pause: '暫停地球儀', resume: '繼續地球儀',
    canvas: '依運作品質著色的國家與大洲地球儀。大洲包含各國。完整數量請見清單。',
    list: '各國及大洲的 Mac 數量',
    continents: ['非洲', '亞洲', '歐洲', '北美洲', '南美洲', '大洋洲', '南極洲', '地區不明'],
    country: '來自 Mac 地區設定的國家',
  },
  es: {
    caption: ['', ' Macs conectados ahora'], captionOne: ' Mac conectado ahora',
    loading: 'Buscando una instantánea en directo…',
    live: 'Instantánea en directo · se actualiza cada 10 segundos',
    fixture: 'Configuración real de hoy (no está en directo)',
    unavailable: 'Los recuentos en directo no están disponibles. Puede que este nodo aún no admita presencia. Se reintenta cada 10 segundos.',
    stale: 'Última instantánea recibida · falló la última actualización. Se reintenta cada 10 segundos.',
    empty: 'Este nodo no ve ningún Mac en este momento.',
    privacy: 'Las subregiones siguen UN M49. Compartir el país requiere una elección explícita. Los grupos pequeños se combinan u ocultan; no se muestran ubicaciones precisas.',
    cohortCaption: ['', ' observaciones de conexiones'],
    cohort: 'Observaciones de grupos sin verificar · los recuentos se publican en intervalos fijos de 10 minutos',
    cohortStale: 'Última observación de grupo · falló la última actualización.',
    withheld: 'Recuentos ocultos por privacidad · no se publican grupos menores de tres.',
    withheldCategory: 'Oculto o no comunicado', unknownRole: 'Rol desconocido', otherRole: 'Otros roles',
    allRegions: 'Todas las regiones', legacy: 'región amplia anterior',
    cohortList: 'Observaciones por subregión UN M49; las regiones amplias anteriores se indican aparte',
    cohortArtwork: 'Tamaño del punto = observaciones de conexiones · las regiones desconocidas y agrupadas permanecen en la lista',
    cohortCanvas: 'Observaciones regionales sin verificar. La lista contiene todos los recuentos publicados. La calidad no está disponible.',
    artwork: 'Tamaño del punto = Macs conectados · los continentes incluyen sus países',
    qualityNew: 'Nuevo', qualitySteady: 'Funcionamiento prolongado y estable',
    quality: 'Calidad de funcionamiento',
    qualityUnavailable: 'Calidad no disponible · este nodo aún no proporciona datos de calidad medidos.',
    canvasUnavailable: 'Globo de grupos por país y continente. La lista contigua muestra todos los recuentos. La calidad no está disponible.',
    validators: 'Validadores', wallet: 'Nodos de cartera',
    candidates: 'Nodos candidatos', followers: 'Nodos seguidores',
    hostLive: 'Instantánea en directo · desde el nodo de tu Mac',
    hostFixture: 'Datos de ejemplo para capturas · no están en directo',
    hostUnavailable: 'Los recuentos en directo del nodo de este Mac no están disponibles.',
    hostStale: 'Última instantánea recibida · falló la última actualización del nodo.',
    reserve: 'Claves de reserva',
    drag: 'Arrastra horizontalmente o usa las flechas para girar.',
    map: 'Mapa estático · movimiento reducido o WebGL no disponible',
    pause: 'Pausar el globo', resume: 'Reanudar el globo',
    canvas: 'Globo de países y continentes, coloreados según la calidad de funcionamiento. Los continentes incluyen sus países. La lista muestra todos los recuentos.',
    list: 'Macs por país y continente',
    continents: ['África', 'Asia', 'Europa', 'América del Norte', 'América del Sur', 'Oceanía', 'Antártida', 'Región desconocida'],
    country: 'País de la región del Mac',
  },
};

function supportedLanguage(value) {
  return typeof value === 'string' && Object.hasOwn(COPY, value) ? value : 'en';
}

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
  host = false, paused: initiallyPaused = false, reducedMotion = false,
  fetch = (...args) => globalThis.fetch(...args),
} = {}) {
  const doc = root.ownerDocument;
  const win = doc.defaultView;
  let language = supportedLanguage(lang);
  let model = null;
  let regions = [];
  let state = 'loading';
  let paused = false;
  let hostPaused = Boolean(initiallyPaused);
  let hostReducedMotion = Boolean(reducedMotion);
  let evidenceAvailable = true;
  let hostEvidenceAvailable = true;
  let subregionsOnly = false;
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
  const qualityStatus = element(doc, 'p', 'lg-quality-status');
  legendLabels.append(newLabel, steadyLabel);
  legend.append(gradient, legendLabels);
  artCaption.append(artwork, legend, qualityStatus);
  figure.append(stage, controls, artCaption);

  const heading = element(doc, 'header', 'lg-heading');
  const caption = element(doc, 'h2', 'lg-caption');
  const headlineStart = element(doc, 'span');
  const count = element(doc, 'strong', 'lg-total', '—');
  const headlineEnd = element(doc, 'span');
  caption.append(headlineStart, count, headlineEnd);
  const summary = element(doc, 'div', 'lg-summary');
  const status = element(doc, 'p', 'lg-status');
  status.setAttribute('role', 'status');
  status.setAttribute('aria-live', 'polite');
  status.setAttribute('aria-atomic', 'true');
  const date = element(doc, 'time', 'lg-snapshot-date', '2026-10-08');
  date.dateTime = '2026-10-08';
  const roleSummary = element(doc, 'p', 'lg-role-summary');
  const list = element(doc, 'dl', 'lg-regions');
  const rows = new Map();
  const visibility = new Map(GEOGRAPHIES.map(code => [code, 'empty']));
  for (const code of GEOGRAPHIES) {
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
    const countries = element(doc, 'dd', 'lg-countries');
    row.append(name, value, countries);
    list.append(row);
    rows.set(code, { row, button, value, countries });
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
  heading.append(caption, roleSummary, status, date);
  summary.append(list, privacy);
  root.replaceChildren(heading, figure, summary);
  let globe;
  globe = createGlobe(canvas, {
    seed, onSelect: highlight, onVisibility: updateVisibility,
    paused: hostPaused, reducedMotion: hostReducedMotion,
  });

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
      const states = regions.filter(region => region.continent === code)
        .map(region => visibility.get(regionKey(region)) || 'empty');
      if (states.includes('front')) position = 'front';
      else if (states.includes('back')) position = 'back';
      else if (['unknown', 'world'].includes(code) && states.length) position = 'unknown';
    }
    const copy = COPY[language];
    item.row.dataset.visibility = position;
    if (model && item.row.dataset.count !== '') {
      const numbers = new Intl.NumberFormat(language);
      const amount = numbers.format(Number(item.row.dataset.count));
      const population = model.schema === 2 ? `${copy.cohortCaption[0]}${amount}${copy.cohortCaption[1]}`
        : language === 'ko' ? `Mac ${amount}대` : language === 'ja' ? `Mac ${amount}台`
        : language === 'zh-Hans' ? `${amount}台Mac` : language === 'zh-Hant' ? `${amount} 台 Mac` : `${amount} Macs`;
      const quality = evidenceAvailable ? `${copy.quality} ${(Number(item.row.dataset.quality) * 100).toFixed(1)} / 100` : copy.qualityUnavailable;
      item.button.setAttribute('aria-label', `${item.button.textContent}, ${population}, ${quality}`);
    } else if (model?.schema === 2 || state === 'withheld') {
      item.button.setAttribute('aria-label', `${item.button.textContent}, ${copy.withheldCategory}`);
    } else item.button.removeAttribute('aria-label');
  }

  function updateVisibility(entries) {
    for (const entry of entries) {
      if (!['front', 'back', 'unknown', 'empty'].includes(entry.visibility)) continue;
      visibility.set(entry.key, entry.visibility);
      rowPosition(entry.key);
      rowPosition(entry.continent);
    }
  }

  function text() {
    const copy = COPY[language];
    const numbers = new Intl.NumberFormat(language);
    const isCohort = model?.schema === 2 || state === 'withheld';
    evidenceAvailable = hostEvidenceAvailable && !isCohort;
    root.lang = language;
    root.dataset.host = String(host);
    root.dataset.reducedMotion = String(motion.matches || hostReducedMotion);
    root.dataset.evidenceAvailable = String(evidenceAvailable);
    headlineStart.textContent = isCohort ? copy.cohortCaption[0] : copy.caption[0];
    headlineEnd.textContent = isCohort ? copy.cohortCaption[1]
      : model?.total === 1 && copy.captionOne ? copy.captionOne : copy.caption[1];
    canvas.setAttribute('aria-label', isCohort || subregionsOnly ? copy.cohortCanvas : evidenceAvailable ? copy.canvas : copy.canvasUnavailable);
    list.setAttribute('aria-label', isCohort || subregionsOnly ? copy.cohortList : copy.list);
    privacy.textContent = copy.privacy;
    artwork.textContent = isCohort ? copy.cohortArtwork : copy.artwork;
    newLabel.textContent = copy.qualityNew;
    steadyLabel.textContent = copy.qualitySteady;
    legend.hidden = !evidenceAvailable;
    qualityStatus.hidden = evidenceAvailable;
    qualityStatus.textContent = copy.qualityUnavailable;
    pause.textContent = paused || hostPaused ? copy.resume : copy.pause;
    pause.setAttribute('aria-pressed', String(paused || hostPaused));
    pause.disabled = hostPaused;
    const isMap = motion.matches || hostReducedMotion || canvas.dataset.renderer === 'map';
    pause.hidden = isMap;
    interaction.textContent = isMap ? copy.map : copy.drag;
    status.textContent = isCohort && ['live', 'stale'].includes(state) ? copy[state === 'live' ? 'cohort' : 'cohortStale']
      : host && ['live', 'fixture', 'unavailable', 'stale'].includes(state)
      ? copy[`host${state[0].toUpperCase()}${state.slice(1)}`] : copy[state];
    status.dataset.state = state;
    date.hidden = state !== 'fixture' || host;
    count.textContent = model?.total != null ? numbers.format(model.total) : '—';
    roleSummary.textContent = '';
    if (model) {
      const role = (label, item) => `${label} ${numbers.format(item.count)}`;
      const observedRoles = model.schema === 2
        ? Object.fromEntries(Object.entries(model.by_role).map(([key, count]) => [key, { count }])) : model.roles;
      const roles = [
        [copy.validators, observedRoles.validator], [copy.wallet, observedRoles.wallet],
        [copy.candidates, observedRoles.candidate], [copy.followers, observedRoles.follower],
        [copy.unknownRole, observedRoles.unknown], [copy.otherRole, observedRoles.other],
      ];
      if (evidenceAvailable) roles.push([copy.reserve, { count: model.reserve_keys.standby + model.reserve_keys.seated }]);
      roleSummary.textContent = roles.filter(([, item]) => item?.count > 0).map(([label, item]) => role(label, item)).join(' · ');
    }
    roleSummary.hidden = !roleSummary.textContent;
    const totals = new Map((model ? continentTotals(model) : []).map(item => [item.continent, item]));
    let countries;
    try { countries = new Intl.DisplayNames([language], { type: 'region' }); } catch { /* older browsers use ISO codes */ }
    const geographyName = code => {
      if (SUBREGION_NAMES[code]) {
        if (language === 'en') return SUBREGION_NAMES[code];
        try { return countries?.of(code) || SUBREGION_NAMES[code]; } catch { return SUBREGION_NAMES[code]; }
      }
      if (code === 'world') return copy.allRegions;
      const label = copy.continents[CONTINENTS.indexOf(code)];
      return isCohort && code !== 'unknown' ? `${label} · ${copy.legacy}` : label;
    };
    const regionLabels = {};
    const countryKeys = new Set(regions.filter(region => region.country).map(regionKey));
    for (const [key, item] of rows) if (key.includes(':') && !countryKeys.has(key)) {
      item.row.remove(); rows.delete(key); visibility.delete(key);
    }
    const subregionsShown = isCohort || !model || regions.some(region => SUBREGION_NAMES[region.continent]);
    GEOGRAPHIES.forEach(code => {
      const row = rows.get(code);
      const total = totals.get(code);
      row.button.textContent = geographyName(code);
      row.row.hidden = subregionsOnly && !SUBREGION_NAMES[code] ? true
        : SUBREGION_CODES.includes(code) ? !subregionsShown
        : code === 'world' ? !total?.count
        : subregionsShown && code !== 'unknown' && !total?.count;
      row.button.disabled = !total?.count;
      row.row.dataset.populated = String(Boolean(total?.count));
      row.row.dataset.count = total?.count != null ? String(total.count) : '';
      row.row.dataset.quality = total?.quality && evidenceAvailable ? String(qualityMean(total.quality, total.count)) : '';
      row.value.textContent = total?.count != null ? numbers.format(total.count) : '—';
      row.value.title = isCohort && total?.count == null ? copy.withheldCategory : '';
      if (!total?.count) visibility.set(code, 'empty');
      else if (['unknown', 'world'].includes(code)) visibility.set(code, 'unknown');
      rowPosition(code);
      for (const region of regions) {
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
          countryRow.append(button, value);
          country = { row: countryRow, button, value };
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
        country.row.dataset.quality = evidenceAvailable ? String(qualityMean(region.quality, region.count)) : '';
        country.button.textContent = label;
        country.button.title = copy.country;
        country.value.textContent = numbers.format(region.count);
        rowPosition(key);
      }
      row.countries.hidden = !row.countries.childElementCount;
    });
    highlight(highlighted, false);
    globe.setLabels({
      continents: Object.fromEntries(GEOGRAPHIES.map(code => [code, geographyName(code)])),
      regions: regionLabels,
      quality: copy.quality,
      qualityAvailable: evidenceAvailable,
      qualityUnavailable: copy.qualityUnavailable,
    });
  }

  function apply(snapshot) {
    model = normalizePresence(snapshot);
    regions = presenceRegions(model).filter(region => !subregionsOnly
      || SUBREGION_NAMES[region.continent] && !region.country && region.count >= 3);
    state = model.total === null ? 'withheld' : fixture ? 'fixture' : model.total === 0 ? 'empty' : 'live';
    globe.update(model, { reset: model.total === null, subregionsOnly });
    text();
  }

  function stop() {
    generation++;
    win.clearTimeout(timer);
    timer = null;
    controller?.abort();
    controller = null;
  }

  // A native host owns its RPC lifecycle. Even fixture mode must not fetch.
  function canPoll() { return !host && !destroyed && !doc.hidden && visible; }

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
        state = model?.total != null ? 'stale' : 'unavailable';
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
    if (hostPaused) return;
    paused = !paused;
    globe.setPaused(paused || hostPaused);
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
  if (!host && visible) void poll();

  function clearSnapshot(nextState) {
    stop();
    model = null;
    regions = [];
    state = nextState;
    highlighted = null;
    visibility.clear();
    for (const code of GEOGRAPHIES) visibility.set(code, 'empty');
    // Internal geometry reset only: no zero network is presented to the user.
    globe.update({
      schema_version: 3, scope: 'node', quality_version: QUALITY_VERSION,
      total: 0, roles: {
        validator: { count: 0 }, wallet: { count: 0 },
        candidate: { count: 0 }, follower: { count: 0 },
      },
      versions: {}, reserve_keys: { standby: 0, seated: 0 },
      regions: [], recent_blocks: [],
    }, { reset: true });
  }

  return {
    setLanguage(next) { if (!destroyed) { language = supportedLanguage(next); text(); } },
    /** The native bridge sends only aggregate presence, never RPC URLs or records. */
    update(snapshot) {
      if (!host || destroyed) return false;
      try { apply(snapshot); return true; }
      catch { state = model?.total != null ? 'stale' : 'unavailable'; text(); return false; }
    },
    reset() {
      if (!host || destroyed) return false;
      clearSnapshot('loading');
      text();
      return true;
    },
    captureFrame() {
      return host && !destroyed ? globe.captureFrame() : false;
    },
    configure(next = {}) {
      if (!host || destroyed || !next || typeof next !== 'object' || Array.isArray(next)) return false;
      const values = {};
      for (const key of ['paused', 'reducedMotion', 'theme', 'lang', 'state', 'fixture', 'evidenceAvailable', 'subregionsOnly']) {
        const field = Object.getOwnPropertyDescriptor(next, key);
        if (!field) continue;
        if (!Object.hasOwn(field, 'value')) return false;
        values[key] = field.value;
      }
      for (const key of ['paused', 'reducedMotion', 'fixture', 'evidenceAvailable', 'subregionsOnly']) {
        if (Object.hasOwn(values, key) && typeof values[key] !== 'boolean') return false;
      }
      if (Object.hasOwn(values, 'theme') && !['light', 'dark'].includes(values.theme)) return false;
      if (Object.hasOwn(values, 'lang') && !Object.hasOwn(COPY, values.lang)) return false;
      if (Object.hasOwn(values, 'state') && !['loading', 'unavailable', 'stale', 'withheld'].includes(values.state)) return false;
      if (Object.hasOwn(values, 'paused')) hostPaused = values.paused;
      if (Object.hasOwn(values, 'reducedMotion')) hostReducedMotion = values.reducedMotion;
      if (Object.hasOwn(values, 'lang')) language = values.lang;
      if (Object.hasOwn(values, 'evidenceAvailable')) hostEvidenceAvailable = values.evidenceAvailable;
      if (Object.hasOwn(values, 'subregionsOnly') && subregionsOnly !== values.subregionsOnly) {
        subregionsOnly = values.subregionsOnly;
        if (model) {
          regions = presenceRegions(model).filter(region => !subregionsOnly
            || SUBREGION_NAMES[region.continent] && !region.country && region.count >= 3);
          globe.setSubregionsOnly(subregionsOnly);
        }
      }
      if (Object.hasOwn(values, 'fixture')) {
        fixture = values.fixture;
        if (model) state = model.total === null ? 'withheld' : fixture ? 'fixture' : model.total === 0 ? 'empty' : 'live';
      }
      if (Object.hasOwn(values, 'state')) {
        if (values.state === 'withheld') clearSnapshot('withheld');
        else state = values.state === 'stale' && model?.total == null ? 'unavailable' : values.state;
      }
      if (Object.hasOwn(values, 'theme')) {
        doc.documentElement.dataset.theme = values.theme;
        doc.documentElement.style.colorScheme = values.theme;
      }
      globe.setPaused(paused || hostPaused);
      globe.setReducedMotion(hostReducedMotion);
      globe.resize();
      text();
      return true;
    },
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
