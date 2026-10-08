// What a search string is, and where a 32-byte hash should go: the node knows
// receipts by hash and recent block summaries by height, so a hash is a
// transaction when a receipt answers and a block when a recent summary
// matches (test/search.test.mjs).

import { browserInput } from './sea-url.mjs';

/**
 * A search string -> `{ kind: 'block', height }`, `{ kind: 'account', address }`,
 * `{ kind: 'hash', hash }` or null. Heights are decimal, addresses and hashes
 * are hex; the 0x prefix may be missing or any case, and everything comes
 * back lowercase-normalized.
 */
export function classifySearch(q, chainID = 1) {
  const s = String(q || '').trim();
  if (!s) return null;
  if (/^\d+$/.test(s)) {
    const height = Number(s);
    if (!Number.isSafeInteger(height)) return null;
    return { kind: 'block', height };
  }
  const bare = s.replace(/^0[xX]/, '').toLowerCase();
  if (/^[0-9a-f]{40}$/.test(bare)) return { kind: 'account', address: `0x${bare}` };
  if (/^[0-9a-f]{64}$/.test(bare)) return { kind: 'hash', hash: `0x${bare}` };
  if (/^(?:sea|eastsea|aether):/i.test(s) || /^[^/?#]+\.[^/?#]+/.test(s)) {
    const link = browserInput(s, chainID);
    if (link.kind === 'name' || link.kind === 'action') return { kind: link.kind, link };
  }
  return null;
}

/**
 * A route for the search: transaction, block or account page — or null when
 * nothing this node knows matches. A hash the node has no receipt for may
 * still be one of the newest 100 block hashes (summaries carry no receipts).
 */
export async function resolveSearch(q, node, chainID = 1) {
  const c = classifySearch(q, chainID);
  if (!c) return null;
  if (c.kind === 'block') return { page: 'block', height: c.height };
  if (c.kind === 'account') return { page: 'account', address: c.address };
  if (c.kind === 'name' || c.kind === 'action') return { page: 'name', link: c.link };
  try {
    const receipt = await node.call('aether_getReceipt', [c.hash]);
    if (receipt) return { page: 'tx', hash: c.hash };
  } catch { /* an unreadable node is the caller's problem to show */ }
  try {
    const blocks = await node.call('aether_recentBlocks', [100]) || [];
    const bare = c.hash.slice(2);
    if (blocks.some((b) => String(b.hash || '').toLowerCase() === bare || String(b.hash || '').toLowerCase() === c.hash)) {
      return { page: 'block', height: blocks.find((b) => String(b.hash || '').toLowerCase() === bare || String(b.hash || '').toLowerCase() === c.hash).height };
    }
  } catch { /* ditto */ }
  return null;
}
