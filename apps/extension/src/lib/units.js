// Amounts: wei as BigInt inside, exact decimal AETH strings at the edges.

const DECIMALS = 18n;
const ONE = 10n ** DECIMALS;

/** "0x1a" | "26" | 26 | 26n -> 26n. Throws on anything else. */
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
  const i = w / ONE;
  const f = (w % ONE).toString().padStart(Number(DECIMALS), '0').replace(/0+$/, '');
  return f ? `${i}.${f}` : `${i}`;
}

/** "1.5" -> 1500000000000000000n. Rejects more than 18 decimals and junk. */
export function aethToWei(text) {
  const s = String(text).trim();
  const m = /^(\d*)(?:\.(\d*))?$/.exec(s);
  if (!s || !m || (m[1] === '' && !m[2])) throw new Error('enter an amount like 1.5');
  const frac = m[2] || '';
  if (frac.length > Number(DECIMALS)) throw new Error('at most 18 decimals');
  return BigInt(m[1] || '0') * ONE + BigInt(frac.padEnd(Number(DECIMALS), '0') || '0');
}

/** A short, readable amount for display ("1,234.5678"). */
export function formatAeth(wei, digits = 4) {
  const [i, f = ''] = weiToAeth(wei).split('.');
  const int = i.replace(/\B(?=(\d{3})+(?!\d))/g, ',');
  const frac = f.slice(0, digits).replace(/0+$/, '');
  return frac ? `${int}.${frac}` : int;
}

export function shortAddress(a) {
  return a && a.length > 12 ? `${a.slice(0, 6)}…${a.slice(-4)}` : a || '';
}
