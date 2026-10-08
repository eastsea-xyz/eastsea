// Canonical browser helper. Build packaging copies these exact bytes into
// the explorer and extension; their tests guard against parser drift.
export const reservedHosts = Object.freeze([
  'pay', 'call', 'connect', 'tx', 'app', 'follow', 'name', 'wallet', 'settings',
  'send', 'receive', 'sign', 'deploy', 'open',
]);
const actions = new Set(reservedHosts);

export function externalNameMessage(language = 'en') {
  const messages = {
    en: 'A web address such as .com is not an EastSea name. Open it with https://.',
    ko: '웹 주소(.com 등)는 동해 이름이 아니에요. https://로 여세요.',
    ja: 'ウェブアドレス（.com など）は EastSea の名前ではありません。https:// で開いてください。',
    'zh-Hans': '网站地址（如 .com）不是 EastSea 名称。请使用 https:// 打开。',
    es: 'Una dirección web como .com no es un nombre de EastSea. Ábrela con https://.',
  };
  const locale = String(language).startsWith('zh') ? 'zh-Hans' : String(language).split('-')[0];
  return messages[locale] || messages.en;
}

export class SeaURLParseError extends Error {
  constructor(code) {
    super(code === 'externalTLD' ? externalNameMessage() : code);
    this.name = 'SeaURLParseError';
    this.code = code;
  }
}
const fail = (code) => { throw new SeaURLParseError(code); };

function checkRaw(raw) {
  if (typeof raw !== 'string' || !raw || /[\x00-\x20\x7f\\]/.test(raw)) fail('invalidURL');
}

/** Split raw authority before URL/WHATWG parsing can lowercase a host,
 * decode an escape, discard userinfo or turn a backslash into a slash. */
function parts(raw) {
  const end = raw.search(/[/?#]/);
  const host = end < 0 ? raw : raw.slice(0, end);
  const tail = end < 0 ? '' : raw.slice(end);
  if (!host || /[@:\[\]\\]/.test(host)) fail('invalidURL');
  return { host, tail };
}

function nameLink(raw, chainID) {
  checkRaw(raw);
  const { host, tail } = parts(raw);
  if (tail.includes('#') || /%(?![0-9a-fA-F]{2})/.test(tail)) fail('invalidURL');
  const labels = host.split('.');
  if (host.length > 253 || labels.some((label) =>
    label.length < 1 || label.length > 63 || !/^[a-z0-9](?:[a-z0-9-]*[a-z0-9])?$/.test(label))) {
    fail('invalidName');
  }
  let name;
  let isLegacy = false;
  if (labels.length === 1) name = `${host}.sea`;
  else if (labels.at(-1) === 'sea') name = host;
  else if (labels.at(-1) === 'aeth') {
    if (Number(chainID) !== 7780) fail('legacyNameUnsupported');
    name = `${host.slice(0, -5)}.sea`;
    isLegacy = true;
  } else fail('externalTLD');
  if (name.length > 253) fail('invalidName');
  if (actions.has(name.split('.').at(-2))) fail('reservedName');
  const queryAt = tail.indexOf('?');
  const path = (queryAt < 0 ? tail : tail.slice(0, queryAt)) || '/';
  const query = queryAt < 0 ? null : tail.slice(queryAt + 1);
  return { kind: 'name', name, canonicalURL: `sea://${name}${path}${query === null ? '' : `?${query}`}`,
    path, query, isLegacy, registryName: isLegacy ? host : name };
}

/** Classify a custom link; action parameters are never decoded or changed.
 * Classification requests no account, payment, signature or network read. */
export function parseSeaURL(raw, chainID = 1) {
  checkRaw(raw);
  const match = raw.match(/^([a-zA-Z][a-zA-Z0-9+.-]*):/);
  if (!match) fail('invalidURL');
  const scheme = match[1].toLowerCase();
  if (!['sea', 'eastsea', 'aether'].includes(scheme)) fail('unsupportedScheme');
  const body = raw.slice(match[0].length);
  // Before the rename, URLComponents also accepted aether:pay?… and
  // eastsea:pay?… through its path fallback. Keep those action bytes too.
  if (!body.startsWith('//')) {
    const host = body.split(/[?#]/, 1)[0];
    if (scheme !== 'sea' && actions.has(host)) return { kind: 'action', host, raw };
    fail('invalidURL');
  }
  const authority = body.slice(2);
  const { host } = parts(authority);
  if (actions.has(host)) return { kind: 'action', host, raw };
  if (scheme === 'aether') fail('unsupportedScheme');
  return nameLink(authority, chainID);
}

/** Address-bar input. Bare action words stay reserved names; only an
 * explicit custom-scheme link can request an action's approval screen. */
export function browserInput(raw, chainID = 1) {
  if (typeof raw !== 'string') fail('invalidURL');
  const input = raw.trim();
  checkRaw(input);
  const scheme = input.match(/^([a-zA-Z][a-zA-Z0-9+.-]*):/);
  if (scheme) {
    if (/^https?:\/\//i.test(input)) {
      try { if (!new URL(input).hostname) fail('invalidURL'); }
      catch { fail('invalidURL'); }
      return { kind: 'web', url: input };
    }
    return parseSeaURL(input, chainID);
  }
  return nameLink(input, chainID);
}

/** Offer HTTPS only for an otherwise valid external DNS hostname. Invalid
 * authorities cannot be repaired into a different site's address. */
export function suggestedHTTPS(raw) {
  if (typeof raw !== 'string') return null;
  const input = raw.trim();
  try { browserInput(input); return null; }
  catch (error) { if (error.code !== 'externalTLD') return null; }
  const body = input.replace(/^(?:sea|eastsea):\/\//i, '');
  const { host, tail } = parts(body);
  return `https://${host}${tail.startsWith('/') ? tail : `/${tail}`}`;
}
