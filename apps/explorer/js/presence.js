// Live presence is an observation from one node, separate from finalized
// chain data. A missing or malformed answer is unavailable, never a zero.

export const PRESENCE_ROLES = ['validator', 'candidate', 'follower'];
export const PRESENCE_REGIONS = [
  ['asia', 'Asia'],
  ['europe', 'Europe'],
  ['north_america', 'North America'],
  ['south_america', 'South America'],
  ['africa', 'Africa'],
  ['oceania', 'Oceania'],
  ['unknown', 'Unknown'],
];

function counts(value) {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null;
  const entries = Object.entries(value);
  return entries.every(([key, count]) => key.length > 0 && Number.isSafeInteger(count) && count >= 0)
    ? Object.fromEntries(entries) : null;
}

function complete(counts, keys, total) {
  return counts && keys.every((key) => Object.hasOwn(counts, key))
    && Object.values(counts).reduce((sum, count) => sum + count, 0) === total;
}

/** Accept versioned counts only while the observation is fresh. `now` is
 * Unix seconds; a small future skew is allowed between the node and browser. */
export function parsePresence(answer, now = Math.floor(Date.now() / 1000)) {
  if (!answer || answer.schema !== 1 || answer.available !== true
    || !Number.isSafeInteger(now) || now < 0
    || !Number.isSafeInteger(answer.total) || answer.total < 0
    || !Number.isSafeInteger(answer.observed_at) || answer.observed_at < 0
    || answer.ttl_seconds !== 180
    || now - answer.observed_at >= answer.ttl_seconds
    || answer.observed_at - now > 60) return null;
  const byRole = counts(answer.by_role);
  const byVersion = counts(answer.by_version);
  const byRegion = counts(answer.by_region);
  if (!complete(byRole, PRESENCE_ROLES, answer.total)
    || !complete(byVersion, [], answer.total)
    || !complete(byRegion, PRESENCE_REGIONS.map(([key]) => key), answer.total)) return null;
  return { total: answer.total, byRole, byVersion, byRegion, ttlSeconds: answer.ttl_seconds };
}

/** Older nodes and gateways may not serve this read yet. Keep the rest of
 * the home page working when that source cannot report presence. */
export async function readPresence(node) {
  try {
    return parsePresence(await node.call('aether_presence', []));
  } catch {
    return null;
  }
}
