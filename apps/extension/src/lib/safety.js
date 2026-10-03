// The send-flow checks of docs/research/token-spam-2026.md §6 (adopted
// 2026-09-28), mirroring the app's SendSafety
// (apps/wallet/Sources/TokenSend.swift). Storage rule: these checks read
// public chain data and settings on this device. Nothing new is written on
// chain — "addresses I sent to" comes from the activity this wallet already
// tracks, and the only things kept are the user's own choices (hidden or
// promoted tokens).

const ADDRESS = /^0x[0-9a-f]{40}$/i;
const decoder = new TextDecoder();

export function isValidAddress(s) {
  return typeof s === 'string' && ADDRESS.test(s.trim());
}

/**
 * Does `recipient` share its first and last 4 hex characters with a different
 * address the wallet sent to before (probable poisoning)? Is this the first
 * send there at all?
 */
export function addressRisk(recipient, history) {
  const to = String(recipient || '').trim().toLowerCase();
  const sent = new Set((history || []).map((a) => String(a).toLowerCase()));
  if (!ADDRESS.test(to)) return { poisoningMatch: null, firstSend: true };
  const body = to.slice(2);
  const prefix = body.slice(0, 4);
  const suffix = body.slice(-4);
  let match = null;
  for (const h of sent) {
    if (h === to) continue;
    const b = h.startsWith('0x') ? h.slice(2) : h;
    if (b.length === 40 && b.slice(0, 4) === prefix && b.slice(-4) === suffix) { match = h; break; }
  }
  return { poisoningMatch: match, firstSend: !sent.has(to) };
}

/** Ligatures NFKD leaves alone; the rest folds by decomposition. */
const LIGATURES = { 'æ': 'ae', 'ø': 'o', 'đ': 'd', 'ł': 'l', 'ß': 'ss' };

/** Lowercase ASCII alphanumerics only, so look-alike checks see through
 * "ÆTH", "aeth " and zero-width tricks. */
export function normalized(s) {
  const folded = String(s ?? '')
    .toLowerCase()
    .replace(/[æøđłß]/g, (c) => LIGATURES[c] || c)
    .normalize('NFKD')
    .replace(/[\u0300-\u036f]/g, '');
  return (folded.match(/[a-z0-9]/g) || []).join('');
}

export function editDistance(a, b) {
  const x = [...a], y = [...b];
  let prev = [...Array(y.length + 1).keys()];
  for (let i = 1; i <= x.length; i += 1) {
    const cur = [i];
    for (let j = 1; j <= y.length; j += 1) {
      cur[j] = Math.min(prev[j] + 1, cur[j - 1] + 1, prev[j - 1] + (x[i - 1] === y[j - 1] ? 0 : 1));
    }
    prev = cur;
  }
  return prev[y.length];
}

function resembles(a, b) {
  if (a === b) return true;
  // A name that contains an official symbol (wrapped/derived names such as
  // "WDBLN") is a look-alike; short symbols are left to the edit-distance rule.
  if (b.length >= 4 && a.includes(b)) return true;
  return Math.min(a.length, b.length) >= 3 && editDistance(a, b) <= 1;
}

/** Does this token's symbol or name equal or resemble an official one? */
export function looksLikeOfficial(token, official) {
  const s = normalized(token?.symbol);
  const n = normalized(token?.name);
  for (const o of official || []) {
    const os = normalized(o.symbol);
    const on = normalized(o.name);
    if (!os) continue;
    if (resembles(s, os) || (on && (resembles(s, on) || resembles(n, os) || resembles(n, on)))) return true;
  }
  return false;
}

/** The human-readable reason from a node error like
 * "execution reverted: 0x08c379a0…" (`Error(string)`), or the message as is. */
export function revertReason(nodeError) {
  const s = String(nodeError || '');
  const at = s.indexOf('0x08c379a0');
  if (at < 0) return s;
  const hex = s.slice(at + 10).replace(/[^0-9a-fA-F]/g, '');
  const words = hex.slice(0, hex.length - (hex.length % 64)).match(/.{64}/g) || [];
  if (words.length < 3) return s;
  try {
    const off = BigInt(`0x${words[0]}`);
    const len = Number(BigInt(`0x${words[1]}`));
    if (off !== 32n || len <= 0 || len > (words.length - 2) * 32) return s;
    const body = words.slice(2).join('').slice(0, len * 2);
    const bytes = new Uint8Array((body.match(/.{2}/g) || []).map((b) => parseInt(b, 16)));
    const text = decoder.decode(bytes).replace(/[\r\n]/g, '');
    return text ? `"${text}"` : s;
  } catch {
    return s;
  }
}

/** "0x8a9B…F41c" — 4 hex chars each side, as in the research doc. */
export function tokenShort(address) {
  const a = String(address || '').replace(/^0x/, '');
  return a.length > 8 ? `0x${a.slice(0, 4)}…${a.slice(-4)}` : String(address || '');
}

/** "NEB · 0x8a9B…F41c" — never a symbol alone. */
export function tokenLabel(token) {
  return `${token?.symbol ?? '?'} · ${tokenShort(token?.address)}`;
}

/**
 * Which holdings the main list shows (token-spam-2026.md §6.1): AETH and
 * tokens this wallet moved by its own signed action, official ones, and
 * whatever the user chose to show. Everything else someone sent in goes to
 * the collapsed Unverified section, out of any total. Only the user's
 * choices are stored, on this device; the derived sets come from the
 * wallet's own tracked transactions.
 */
export function splitHoldings(holdings, { touched = [], official = [], hidden = [], shown = [] } = {}) {
  const t = new Set(touched.map((a) => String(a).toLowerCase()));
  const o = new Set(official.map((a) => String(a).toLowerCase()));
  const h = new Set(hidden.map((a) => String(a).toLowerCase()));
  const s = new Set(shown.map((a) => String(a).toLowerCase()));
  const isMain = (address) => {
    const a = String(address).toLowerCase();
    if (h.has(a)) return false;
    return o.has(a) || t.has(a) || s.has(a);
  };
  const main = [], unverified = [];
  for (const x of holdings || []) (isMain(x.token.address) ? main : unverified).push(x);
  return { main, unverified };
}
