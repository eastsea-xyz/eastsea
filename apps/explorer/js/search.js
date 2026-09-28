// What a search string is, and where a 32-byte hash should go: the node knows
// receipts by hash and recent block summaries by height, so a hash is a
// transaction when a receipt answers and a block when a recent summary
// matches (test/search.test.mjs).

/**
 * A search string -> `{ kind: 'block', height }`, `{ kind: 'account', address }`,
 * `{ kind: 'hash', hash }` or null. Heights are decimal, addresses and hashes
 * are hex; the 0x prefix may be missing or any case, and everything comes
 * back lowercase-normalized.
 */
export function classifySearch(q) {
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
  return null;
}

/**
 * A route for the search: transaction, block or account page — or null when
 * nothing this node knows matches. A hash the node has no receipt for may
 * still be one of the newest 100 block hashes (summaries carry no receipts).
 */
export async function resolveSearch(q, node) {
  const c = classifySearch(q);
  if (!c) return null;
  if (c.kind === 'block') return { page: 'block', height: c.height };
  if (c.kind === 'account') return { page: 'account', address: c.address };
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
