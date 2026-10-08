import { Brand } from './brand.js';
// What pages may ask the wallet, and how a transaction request is checked.

/** Reads go straight to the node; nothing here can move funds. */
export const READ_METHODS = new Set([
  'eth_blockNumber', 'eth_call', 'eth_estimateGas', 'eth_getBalance', 'eth_getCode',
  'eth_getLogs', 'eth_getStorageAt', 'eth_getTransactionCount', 'eth_gasPrice',
  'net_version', 'aether_status', 'aether_getReceipt', 'aether_getAccount', 'aether_accountHistory',
]);

export const ACCOUNT_METHODS = new Set(['eth_requestAccounts', 'aether_requestAccounts', 'eth_accounts', 'aether_accounts']);
export const SEND_METHODS = new Set(['eth_sendTransaction', 'aether_sendTransaction']);
export const TYPED_METHODS = new Set(['eth_signTypedData_v4']);
export const MAX_GAS = 10_000_000;

const ADDRESS = /^0x[0-9a-fA-F]{40}$/;
const HEX = /^0x([0-9a-fA-F]{2})*$/;

/**
 * Check a page's transaction request and normalize it to what the wasm builder
 * takes: {to, value_wei (decimal string), data, gas}. `value` and `gas` follow
 * EIP-1193 (hex quantities); decimal strings are accepted too.
 */
export function normalizeTx(tx) {
  if (!tx || typeof tx !== 'object') throw new Error('expected a transaction object');
  const to = tx.to == null || tx.to === '' ? '' : String(tx.to);
  if (to && !ADDRESS.test(to)) throw new Error('`to` is not an address');
  const data = tx.data ?? tx.input ?? '0x';
  if (typeof data !== 'string' || !HEX.test(data)) throw new Error('`data` must be 0x-prefixed hex bytes');
  if (!to && data === '0x') throw new Error('a contract creation needs init code in `data`');
  const value = tx.value == null ? 0n : quantity(tx.value, '`value`');
  let gas = tx.gas ?? tx.gasLimit;
  gas = gas == null ? 0 : Number(quantity(gas, '`gas`'));
  if (gas > MAX_GAS) throw new Error(`gas above the wallet cap (${MAX_GAS})`);
  return { to, value_wei: value.toString(), data: data.toLowerCase(), gas };
}

/** Exec gas for a plain value transfer to an address with code (a contract's
 * receive(), or an account delegated by EIP-7702): the wasm builder's default
 * 21,000 for `data: '0x'` cannot run any code, so such a send was included,
 * failed out of gas and still paid its fee (live run 2026-10-06). Mirrors
 * crates/execution `CODE_RECIPIENT_TRANSFER_GAS`. */
export const CODE_RECIPIENT_TRANSFER_GAS = 100_000;

/** A normalized tx with the transfer gas sized from the recipient's code
 * (`eth_getCode` answer). Only an unset gas on a plain value transfer changes;
 * a page's explicit gas, a call and a deployment are left as they are. */
export function withTransferGas(tx, recipientCode) {
  if (tx.gas || !tx.to || tx.data !== '0x') return tx;
  const code = typeof recipientCode === 'string' ? recipientCode : '0x';
  return code === '0x' || code === '' ? tx : { ...tx, gas: CODE_RECIPIENT_TRANSFER_GAS };
}

function quantity(v, what) {
  if (typeof v === 'string' && /^0x[0-9a-fA-F]+$/.test(v)) return BigInt(v);
  if (typeof v === 'string' && /^\d+$/.test(v)) return BigInt(v);
  if (typeof v === 'number' && Number.isSafeInteger(v) && v >= 0) return BigInt(v);
  throw new Error(`${what} must be a hex quantity`);
}

/** What a call does, from well-known 4-byte selectors (display only). `ticker`
 * is the native coin of the chain the call runs on (lib/brand.js coinTicker),
 * so the legacy 7780 testnet names its own coin, AETH. */
export function describeCall({ to, data }, { ticker = Brand.coinTicker } = {}) {
  if (!to) return `Deploy a contract (${(data.length - 2) / 2} bytes)`;
  if (data === '0x') return `Send ${ticker}`;
  const known = {
    '0xa9059cbb': 'Token transfer', '0x095ea7b3': 'Token approval (allows spending)', '0x23b872dd': 'Token transfer from',
    // Aether DEX router
    '0x38ed1739': 'Swap tokens', '0xac344b4d': `Swap ${ticker} for tokens`, '0x3f070ce1': `Swap tokens for ${ticker}`,
    '0xe8e33700': 'Add liquidity', '0xcf2df7c6': `Add liquidity with ${ticker}`, '0xbaa2abde': 'Remove liquidity',
    '0x0fb9ca68': `Remove liquidity to ${ticker}`, '0xd0e30db0': `Wrap ${ticker}`, '0x2e1a7d4d': `Unwrap ${ticker}`,
    '0x3ca6d100': 'Create a token', '0xc7ff321d': 'Create a token',
    // Aether launchpad
    '0x42a81515': 'Launch a token', '0xcce7ec13': 'Buy on the launch curve', '0x6a272462': 'Sell on the launch curve',
    '0x5cf66fe1': `Buy with ${ticker} (graduated pool)`, '0xff5b07d8': `Sell for ${ticker} (graduated pool)`,
  };
  return known[data.slice(0, 10)] || `Contract call ${data.slice(0, 10)}`;
}

/** Only https pages (and pages served from this computer) can connect, as with the app's links. */
export function originAllowed(origin) {
  try {
    const u = new URL(origin);
    return u.protocol === 'https:' || (u.protocol === 'http:' && ['localhost', '127.0.0.1'].includes(u.hostname));
  } catch {
    return false;
  }
}
