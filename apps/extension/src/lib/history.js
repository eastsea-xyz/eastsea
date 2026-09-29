import { Brand } from './brand.js';
// Node-sourced activity is display data. AETH balances in the Mac app are
// still checked against a finality certificate and a state proof.

import { isValidAddress, tokenShort } from './safety.js';

export function linkedAddress(address, own, existing = []) {
  const value = String(address || '').trim().toLowerCase();
  if (!isValidAddress(value)) throw new Error('Enter a 0x-prefixed 20-byte address.');
  if (value === String(own).toLowerCase() || existing.some((a) => a.toLowerCase() === value)) throw new Error('That wallet is already shown.');
  return value;
}

function units(raw, decimals = 18) {
  const s = String(raw || '0').replace(/^0+(?=\d)/, '');
  if (!/^\d+$/.test(s)) return '?';
  if (!decimals) return s;
  const padded = s.padStart(decimals + 1, '0');
  const fraction = padded.slice(-decimals).replace(/0+$/, '').slice(0, 6);
  return `${padded.slice(0, -decimals)}${fraction ? `.${fraction}` : ''}`;
}

function tokenName(address, catalog = {}) {
  const meta = catalog[String(address).toLowerCase()];
  const name = meta?.symbol || 'Token';
  const label = `${name} · ${tokenShort(address)}`;
  return meta?.origin === 'launchpad' ? `${label} (Launchpad · unverified)` : meta ? label : `${label} (unverified)`;
}

export function describeHistory(row, { sources = {}, catalog = {} } = {}) {
  const me = row.address.toLowerCase();
  const from = (row.from || '').toLowerCase();
  const to = (row.to || '').toLowerCase();
  const incoming = (row.tokens || []).filter((t) => t.to.toLowerCase() === me && t.from.toLowerCase() !== me);
  const outgoing = (row.tokens || []).filter((t) => t.from.toLowerCase() === me && t.to.toLowerCase() !== me);
  const tokenText = (t) => {
    const meta = catalog[t.token.toLowerCase()];
    return `${meta ? units(t.amount, meta.decimals) : `${t.amount} base units`} ${tokenName(t.token, catalog)}`;
  };
  const amount = units(row.value_wei);
  const contract = sources.router && to === sources.router.toLowerCase() ? 'router'
    : sources.launchpad && to === sources.launchpad.toLowerCase() ? 'launchpad'
      : sources.tokenFactory && to === sources.tokenFactory.toLowerCase() ? 'factory' : '';
  let title;
  if (row.kind === 'node_reward') title = `Node reward ${amount} ${Brand.coinTicker}`;
  else if (row.kind === 'proof_reward') title = `Proof reward ${amount} ${Brand.coinTicker}`;
  else if (row.kind === 'registration') title = 'Registered a voting node';
  else if (!row.success) title = `Failed call ${tokenShort(row.to || row.from || row.address)} (${row.method || 'transfer'})`;
  else if (row.kind === 'deploy') title = `Deployed contract ${tokenShort(row.contract_address || row.address)}`;
  else if (contract === 'router' && row.pair_swaps?.length && ['0x38ed1739', '0xac344b4d', '0x3f070ce1'].includes(row.method)) {
    const spent = outgoing[0] ? tokenText(outgoing[0]) : `${amount} ${Brand.coinTicker}`;
    const nativeIn = sources.waeth && row.native_payout_source?.toLowerCase() === sources.waeth.toLowerCase()
      ? row.native_received_wei : null;
    const got = incoming.at(-1) ? tokenText(incoming.at(-1)) : (nativeIn ? `${units(nativeIn)} ${Brand.coinTicker}` : Brand.coinTicker);
    title = `Swapped ${spent} → ${got}`;
  } else if (contract === 'router' && ['0xe8e33700', '0xcf2df7c6'].includes(row.method)) title = 'Added liquidity';
  else if (contract === 'router' && ['0xbaa2abde', '0x0fb9ca68'].includes(row.method)) title = 'Removed liquidity';
  else if (contract === 'launchpad' && row.method === '0x42a81515') title = 'Launched a token';
  else if (contract === 'launchpad' && row.method === '0xcce7ec13') title = `Bought on launchpad · ${incoming[0] ? tokenText(incoming[0]) : `${amount} ${Brand.coinTicker}`}`;
  else if (contract === 'launchpad' && row.method === '0x6a272462') title = `Sold on launchpad · ${outgoing[0] ? tokenText(outgoing[0]) : 'token'}`;
  else if (contract === 'factory' && ['0x3ca6d100', '0xc7ff321d'].includes(row.method)) title = 'Created a token';
  else if (row.method === '0x095ea7b3') title = `${row.approval_amount === '0' ? 'Revoked' : 'Approved'} token ${tokenName(row.to, catalog)}${row.approval_spender ? ` for ${tokenShort(row.approval_spender)}` : ''}`;
  else if (row.kind === 'native_transfer' && from !== me) title = `Received ${amount} ${Brand.coinTicker} from ${tokenShort(row.from)}`;
  else if (row.kind === 'native_transfer') title = `Sent ${amount} ${Brand.coinTicker} to ${tokenShort(row.to)}`;
  else if (incoming.length && from !== me) title = `Received ${tokenText(incoming[0])} from ${tokenShort(incoming[0].from)}`;
  else if (outgoing.length && row.method === '0xa9059cbb') title = `Sent ${tokenText(outgoing[0])} to ${tokenShort(outgoing[0].to)}`;
  else title = `Contract call ${tokenShort(row.to || row.address)} (method ${row.method || '0x'})`;
  if (title.startsWith('Contract call') && (incoming.length || outgoing.length || row.value_wei !== '0')) {
    const deltas = [...outgoing.map((t) => `−${tokenText(t)}`), ...incoming.map((t) => `+${tokenText(t)}`)];
    if (row.value_wei !== '0') deltas.unshift(`−${amount} ${Brand.coinTicker}`);
    title += ` · ${deltas.join(', ')}`;
  }
  return { hash: row.tx_hash, title, at: row.timestamp_ms, state: row.success ? 'done' : 'failed',
    origin: `From the node · ${tokenShort(row.address)}`, owner: row.address, height: row.height,
    value: row.value_wei, source: 'node',
    to: from === me ? (row.kind === 'native_transfer' ? row.to : row.kind === 'erc20_transfer' ? outgoing[0]?.to : undefined) : undefined,
    token: from === me && (row.kind === 'erc20_transfer' || row.method === '0x095ea7b3') ? row.to : undefined };
}

export function mergeHistory(local = [], chain = []) {
  const byHash = new Map();
  for (const item of local) byHash.set(String(item.hash).toLowerCase(), item);
  for (const item of chain) {
    const key = String(item.hash).toLowerCase();
    const previous = byHash.get(key);
    if (previous?.source === 'node') continue; // primary address wins when linked wallets share a tx
    byHash.set(key, previous ? { ...previous, ...item, title: item.title, source: 'node' } : item);
  }
  return [...byHash.values()].sort((a, b) => (b.at || 0) - (a.at || 0));
}
