// The only boundary between an RPC response and the public globe model.
// Artwork and session jitter live locally; this model never contains positions.
export const CONTINENTS = Object.freeze([
  'africa', 'asia', 'europe', 'north_america', 'south_america',
  'oceania', 'antarctica', 'unknown',
]);

const CONTINENT_CODES = new Set(CONTINENTS);
const ROLES = ['validator', 'candidate', 'follower'];
const MAX_REGIONS = 1024;
const MAX_VERSIONS = 128;
const MAX_RECENT_BLOCKS = 8;
const JITTER_LIMIT = 0.035;
const BAD_DATA = 'Invalid presence data.';
const UNAVAILABLE = 'Live presence is unavailable.';

// ISO 3166-1 alpha-2, bundled so country validation makes no network request.
const COUNTRY_CODES = new Set((
  'AD AE AF AG AI AL AM AO AQ AR AS AT AU AW AX AZ BA BB BD BE BF BG BH BI BJ BL BM BN BO BQ BR BS BT BV BW BY BZ '
  + 'CA CC CD CF CG CH CI CK CL CM CN CO CR CU CV CW CX CY CZ DE DJ DK DM DO DZ EC EE EG EH ER ES ET FI FJ FK FM FO FR '
  + 'GA GB GD GE GF GG GH GI GL GM GN GP GQ GR GS GT GU GW GY HK HM HN HR HT HU ID IE IL IM IN IO IQ IR IS IT JE JM JO JP '
  + 'KE KG KH KI KM KN KP KR KW KY KZ LA LB LC LI LK LR LS LT LU LV LY MA MC MD ME MF MG MH MK ML MM MN MO MP MQ MR MS MT MU MV MW MX MY MZ '
  + 'NA NC NE NF NG NI NL NO NP NR NU NZ OM PA PE PF PG PH PK PL PM PN PR PS PT PW PY QA RE RO RS RU RW '
  + 'SA SB SC SD SE SG SH SI SJ SK SL SM SN SO SR SS ST SV SX SY SZ TC TD TF TG TH TJ TK TL TM TN TO TR TT TV TW TZ '
  + 'UA UG UM US UY UZ VA VC VE VG VI VN VU WF WS YE YT ZA ZM ZW'
).split(' '));

// Full release identifiers (including prerelease/build identifiers), at most
// 64 characters. A release label cannot carry an arbitrary server message.
const RELEASE = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-(?:0|[1-9]\d*|\d*[A-Za-z-][0-9A-Za-z-]*)(?:\.(?:0|[1-9]\d*|\d*[A-Za-z-][0-9A-Za-z-]*))*)?(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$/;

function invalid() { throw new Error(BAD_DATA); }

function record(value) {
  if (!value || typeof value !== 'object' || Array.isArray(value)) invalid();
  const prototype = Object.getPrototypeOf(value);
  if (prototype !== Object.prototype && prototype !== null) invalid();
  for (const key of ['__proto__', 'constructor', 'prototype']) {
    if (Object.hasOwn(value, key)) invalid();
  }
  return value;
}

function field(value, key) {
  const descriptor = Object.getOwnPropertyDescriptor(value, key);
  if (!descriptor) return undefined;
  if (!Object.hasOwn(descriptor, 'value')) invalid();
  return descriptor.value;
}

function collection(value, limit) {
  if (!Array.isArray(value) || Object.getPrototypeOf(value) !== Array.prototype || value.length > limit) invalid();
  for (const key of ['__proto__', 'constructor', 'prototype']) {
    if (Object.hasOwn(value, key)) invalid();
  }
  return value;
}

function count(value) {
  if (!Number.isSafeInteger(value) || value < 0) invalid();
  return value;
}

function add(a, b) { return count(a + b); }

function continent(value) {
  if (!CONTINENT_CODES.has(value)) invalid();
  return value;
}

function normalized(payload) {
  const source = record(payload);
  if (field(source, 'schema_version') !== 1 || field(source, 'scope') !== 'node') invalid();
  const total = count(field(source, 'total'));

  const roleSource = record(field(source, 'roles'));
  const roles = {};
  let roleTotal = 0;
  for (const role of ROLES) {
    roles[role] = count(field(roleSource, role));
    roleTotal = add(roleTotal, roles[role]);
  }
  if (roleTotal !== total) invalid();

  const versionSource = record(field(source, 'versions'));
  const releases = Object.keys(versionSource).sort();
  if (releases.length > MAX_VERSIONS) invalid();
  const versions = {};
  let versionTotal = 0;
  for (const release of releases) {
    if (release.length > 64 || !RELEASE.test(release)) invalid();
    versions[release] = count(field(versionSource, release));
    versionTotal = add(versionTotal, versions[release]);
  }
  if (versionTotal !== total) invalid();

  // Merge before testing k, so duplicate opted-in buckets cannot either evade
  // the threshold or cause an already anonymous country to be discarded.
  const buckets = new Map(CONTINENTS.map((code) => [code, { count: 0, countries: new Map() }]));
  const regionSource = collection(field(source, 'regions'), MAX_REGIONS);
  let regionTotal = 0;
  for (let i = 0; i < regionSource.length; i++) {
    const region = record(field(regionSource, String(i)));
    const code = continent(field(region, 'continent'));
    const size = count(field(region, 'count'));
    const country = field(region, 'country');
    if (country != null && !COUNTRY_CODES.has(country)) invalid();
    regionTotal = add(regionTotal, size);
    const bucket = buckets.get(code);
    if (country == null) bucket.count = add(bucket.count, size);
    else bucket.countries.set(country, add(bucket.countries.get(country) || 0, size));
  }
  if (regionTotal !== total) invalid();

  const regions = [];
  for (const code of CONTINENTS) {
    const bucket = buckets.get(code);
    const countries = [];
    for (const [country, size] of [...bucket.countries].sort(([a], [b]) => a.localeCompare(b))) {
      if (size < 3) bucket.count = add(bucket.count, size);
      else countries.push({ continent: code, country, count: size });
    }
    if (bucket.count) regions.push({ continent: code, count: bucket.count });
    regions.push(...countries);
  }

  const blockSource = field(source, 'recent_blocks');
  const recent_blocks = [];
  if (blockSource !== undefined) {
    const blocks = collection(blockSource, MAX_RECENT_BLOCKS);
    for (let i = 0; i < blocks.length; i++) {
      const block = record(field(blocks, String(i)));
      recent_blocks.push({
        height: count(field(block, 'height')),
        continent: continent(field(block, 'continent')),
      });
    }
  }

  return { schema_version: 1, scope: 'node', total, roles, versions, regions, recent_blocks };
}

/** Reject malformed responses; copy only the explicitly public aggregate data. */
export function normalizePresence(payload) {
  try { return normalized(payload); }
  catch { throw new Error(BAD_DATA); }
}

/** A stable, accessible list, including continents with no observed Macs. */
export function continentTotals(model) {
  const clean = normalizePresence(model);
  const totals = new Map(CONTINENTS.map((code) => [code, 0]));
  for (const region of clean.regions) {
    totals.set(region.continent, add(totals.get(region.continent), region.count));
  }
  return CONTINENTS.map((code) => ({ continent: code, count: totals.get(code) }));
}

function hash(text) {
  let value = 2166136261;
  for (let i = 0; i < text.length; i++) value = Math.imul(value ^ text.charCodeAt(i), 16777619);
  value ^= value >>> 16;
  value = Math.imul(value, 0x7feb352d);
  value ^= value >>> 15;
  value = Math.imul(value, 0x846ca68b);
  return (value ^ (value >>> 16)) >>> 0;
}

/** Local angular offsets; callers keep the page-session seed in memory only. */
export function sessionJitter(code, seed) {
  continent(code);
  if (!(typeof seed === 'string' || (typeof seed === 'number' && Number.isFinite(seed)))) invalid();
  if (typeof seed === 'string' && seed.length > 256) invalid();
  const input = `${code}\0${seed}`;
  const axis = (name) => (hash(`${input}\0${name}`) / 0xffffffff * 2 - 1) * JITTER_LIMIT;
  return Object.freeze([axis('yaw'), axis('pitch')]);
}

/** Presence is a read-only request without browser credentials or referrers. */
export async function requestPresence(endpoint, { fetch = globalThis.fetch, signal } = {}) {
  try {
    if (typeof endpoint !== 'string' || endpoint.length > 2048 || typeof fetch !== 'function') invalid();
    const url = new URL(endpoint);
    if (!['http:', 'https:'].includes(url.protocol) || url.username || url.password || url.hash) invalid();
    const response = await fetch(endpoint, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'aether_presence', params: [] }),
      credentials: 'omit',
      cache: 'no-store',
      referrerPolicy: 'no-referrer',
      redirect: 'error',
      signal,
    });
    if (!response || response.ok !== true) invalid();
    const envelope = record(await response.json());
    if (field(envelope, 'jsonrpc') !== '2.0' || field(envelope, 'id') !== 1
      || Object.hasOwn(envelope, 'error') || !Object.hasOwn(envelope, 'result')) invalid();
    return normalizePresence(field(envelope, 'result'));
  } catch {
    // Errors must never echo an RPC body, server message, URL or transport data.
    throw new Error(UNAVAILABLE);
  }
}
