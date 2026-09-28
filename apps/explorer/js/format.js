// Amounts, numbers and times. BigInt inside, exact decimals at the edges —
// the same rules as apps/extension/src/lib/units.js (test/format.test.mjs).

const AETH_DECIMALS = 18n;
const ONE_AETH = 10n ** AETH_DECIMALS;

/** "0x1a" | "26" | 26 | 26n -> 26n. Throws on anything else (no floats, no negatives). */
export function toBigInt(v) {
  if (typeof v === 'bigint') return v;
  if (typeof v === 'number' && Number.isSafeInteger(v) && v >= 0) return BigInt(v);
  if (typeof v === 'string' && /^0x[0-9a-fA-F]+$/.test(v)) return BigInt(v);
  if (typeof v === 'string' && /^\d+$/.test(v)) return BigInt(v);
  throw new Error(`not a non-negative integer: ${String(v).slice(0, 40)}`);
}

/** Wei -> exact decimal AETH ("1.5"), no trailing zeros. */
export function weiToAeth(wei) {
  const w = toBigInt(wei);
  const i = w / ONE_AETH;
  const f = (w % ONE_AETH).toString().padStart(Number(AETH_DECIMALS), '0').replace(/0+$/, '');
  return f ? `${i}.${f}` : `${i}`;
}

/** Wei -> short readable AETH ("1,234.5678"), at most `digits` fraction digits. */
export function formatAeth(wei, digits = 4) {
  const [i, f = ''] = weiToAeth(wei).split('.');
  const int = i.replace(/\B(?=(\d{3})+(?!\d))/g, ',');
  const frac = f.slice(0, digits).replace(/0+$/, '');
  return frac ? `${int}.${frac}` : int;
}

/** Token base units -> display text, at most 6 fraction digits with trailing
 * zeros dropped (as the wallet formats them). "<0.000001" when too small to show. */
export function formatTokenAmount(raw, decimals) {
  const s = typeof raw === 'bigint' ? raw.toString() : String(raw ?? '0');
  if (!(decimals > 0) || s === '0') return s;
  const padded = '0'.repeat(Math.max(0, decimals + 1 - s.length)) + s;
  const whole = padded.slice(0, -decimals).replace(/^0+/, '') || '0';
  const frac = padded.slice(-decimals).slice(0, 6).replace(/0+$/, '');
  if (!frac) return whole === '0' ? '<0.000001' : whole;
  return `${whole}.${frac}`;
}

/** 1234567 -> "1,234,567". */
export function formatInt(n) {
  return Number(n).toLocaleString('en-US');
}

/** "0xabcdef…" -> "0xabcd…cdef" (a link label; the full value stays in the title). */
export function shortHex(x, lead = 6, tail = 4) {
  const s = String(x ?? '');
  return s.length > lead + tail + 2 ? `${s.slice(0, lead)}…${s.slice(-tail)}` : s;
}

/** Milliseconds since epoch -> "3s ago" style, computed against `now`. */
export function timeAgo(ms, now = Date.now()) {
  const d = Math.max(0, now - ms);
  const s = Math.round(d / 1000);
  if (s < 5) return 'just now';
  if (s < 60) return `${s}s ago`;
  const m = Math.round(s / 60);
  if (m < 60) return `${m}m ago`;
  const hr = Math.round(m / 60);
  if (hr < 48) return `${hr}h ago`;
  return `${Math.round(hr / 24)}d ago`;
}

/** Block timestamp (ms) -> local "2026-09-29 14:22:03". */
export function localTime(ms) {
  const d = new Date(Number(ms));
  const p = (n, w = 2) => String(n).padStart(w, '0');
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}`;
}

/**
 * Transactions per second over a window of block summaries (any order): the
 * total tx count divided by the span between the oldest and newest timestamps.
 * `null` when the window is a single instant (one block, or a stampede).
 */
export function txRate(blocks) {
  const list = (blocks || []).filter((b) => Number.isFinite(b?.timestamp_ms));
  if (list.length < 2) return null;
  const times = list.map((b) => b.timestamp_ms);
  const span = Math.max(...times) - Math.min(...times);
  const txs = list.reduce((n, b) => n + (b.txs?.length || 0), 0);
  if (span <= 0) return null;
  return { txs, seconds: span / 1000, perSec: txs / (span / 1000) };
}

/** 1.234 -> "1.23" for rates; "—" for null. */
export function formatRate(x) {
  if (x == null) return '—';
  return String(Math.round(x * 100) / 100);
}
