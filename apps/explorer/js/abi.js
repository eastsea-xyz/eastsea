// The few Solidity ABI pieces the explorer reads: ERC-20 metadata answers,
// `Transfer`/`Approval` logs and revert reasons. Word parsing mirrors
// apps/extension/src/lib/tokens.js (test/abi.test.mjs).

const decoder = new TextDecoder();

/** Method selectors (`forge inspect <Contract> methodIdentifiers`). */
export const SEL = {
  allTokensLength: 'dbb80e42', allTokens: '634282af',
  allPairsLength: '574f2ba3', allPairs: '1e3dd18b', token0: '0dfe1681', token1: 'd21220a7',
  tokenCount: '9f181b5e', tokens: '4f64b2be', // launchpad (CurveLaunch)
  symbol: '95d89b41', name: '06fdde03', decimals: '313ce567', balanceOf: '70a08231',
  totalSupply: '18160ddd',
};

/** `keccak256("Transfer(address,address,uint256)")`. */
export const TRANSFER_TOPIC = '0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef';
/** `keccak256("Approval(address,address,uint256)")`. */
export const APPROVAL_TOPIC = '0x8c5be1e5ebec7d5bd14f71427d1e84f3dd0314c0f7b2291e5b200ac8c7c3b925';

export function call(sel, ...words) {
  return `0x${sel}${words.join('')}`;
}

/** An address as one 32-byte argument word (also an indexed address topic). */
export function wordAddress(a) {
  const hex = String(a).toLowerCase().replace(/^0x/, '');
  return hex.padStart(64, '0');
}

/** A small integer as one 32-byte argument word. */
export function wordUint(n) {
  return BigInt(n).toString(16).padStart(64, '0');
}

function words(data) {
  const hex = String(data).replace(/^0x/, '');
  if (!hex || hex.length % 64 !== 0 || !/^[0-9a-fA-F]+$/.test(hex)) throw new Error('bad answer from the node');
  return hex.match(/.{64}/g);
}

/** Word `i` as a BigInt (exact, any size). */
export function uintAt(data, i = 0) {
  const w = words(data);
  if (i >= w.length) throw new Error('bad answer from the node');
  return BigInt(`0x${w[i]}`);
}

/** Word `i` as a number; throws unless the word is a plain uint64. */
export function uint64At(data, i = 0) {
  const w = words(data);
  if (i >= w.length || !/^0{48}/.test(w[i])) throw new Error('bad answer from the node');
  return Number(BigInt(`0x${w[i]}`));
}

/** Word `i` as a lowercase 0x address. */
export function addressAt(data, i = 0) {
  const w = words(data);
  if (i >= w.length) throw new Error('bad answer from the node');
  return `0x${w[i].slice(24)}`;
}

/** A dynamic `string` return value, control characters stripped. */
export function stringAt(data) {
  const w = words(data);
  const off = uint64At(data, 0);
  if (off % 32 !== 0 || off / 32 >= w.length) throw new Error('bad answer from the node');
  const at = off / 32;
  const n = uint64At(data, at);
  if (n > 4096) throw new Error('bad answer from the node');
  const hex = w.slice(at + 1).join('').slice(0, n * 2);
  if (hex.length < n * 2) throw new Error('bad answer from the node');
  return decoder.decode(new Uint8Array((hex.match(/.{2}/g) || []).map((b) => parseInt(b, 16)))).replace(/[\u0000-\u001f\u007f]/g, '');
}

/** A 32-byte topic word as a lowercase 0x address. */
export function topicAddress(topic) {
  return `0x${String(topic).replace(/^0x/, '').slice(-40).toLowerCase()}`;
}

/** An ERC-20 `Transfer` log -> `{ from, to, value }`, or null if it is not one. */
export function decodeTransfer(ev) {
  if (!ev || String(ev.topics?.[0] || '').toLowerCase() !== TRANSFER_TOPIC || ev.topics.length !== 3) return null;
  try {
    return { from: topicAddress(ev.topics[1]), to: topicAddress(ev.topics[2]), value: uintAt(ev.data, 0) };
  } catch {
    return null;
  }
}

/** An ERC-20 `Approval` log -> `{ owner, spender, value }`, or null if it is not one. */
export function decodeApproval(ev) {
  if (!ev || String(ev.topics?.[0] || '').toLowerCase() !== APPROVAL_TOPIC || ev.topics.length !== 3) return null;
  try {
    return { owner: topicAddress(ev.topics[1]), spender: topicAddress(ev.topics[2]), value: uintAt(ev.data, 0) };
  } catch {
    return null;
  }
}

/** The human-readable reason from a node error like
 * "execution reverted: 0x08c379a0…" (`Error(string)`), or the message as is. */
export function revertReason(nodeError) {
  const s = String(nodeError || '');
  const at = s.indexOf('0x08c379a0');
  if (at < 0) return s;
  const hex = s.slice(at + 10).replace(/[^0-9a-fA-F]/g, '');
  const ws = hex.slice(0, hex.length - (hex.length % 64)).match(/.{64}/g) || [];
  if (ws.length < 3) return s;
  try {
    const off = BigInt(`0x${ws[0]}`);
    const len = Number(BigInt(`0x${ws[1]}`));
    if (off !== 32n || len <= 0 || len > (ws.length - 2) * 32) return s;
    const body = ws.slice(2).join('').slice(0, len * 2);
    const bytes = new Uint8Array((body.match(/.{2}/g) || []).map((b) => parseInt(b, 16)));
    const text = decoder.decode(bytes).replace(/[\r\n]/g, '');
    return text ? `"${text}"` : s;
  } catch {
    return s;
  }
}
