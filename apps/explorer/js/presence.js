// Public presence is a frozen, thresholded cohort observation, independent
// of finalized chain data. A suppressed small count must not become a zero.

export const PRESENCE_ROLES = ['validator', 'candidate', 'follower', 'unknown', 'other'];
export const PRESENCE_REGIONS = [
  ['asia', 'Asia'],
  ['europe', 'Europe'],
  ['north_america', 'North America'],
  ['south_america', 'South America'],
  ['africa', 'Africa'],
  ['oceania', 'Oceania'],
  ['unknown', 'Unknown'],
  ['world', 'All regions'],
];
const SCOPE = 'unverified cohort observation';
const WINDOW_SECONDS = 600;
const MINIMUM_BUCKET_SIZE = 3;
const FIELDS = new Set([
  'schema', 'available', 'scope', 'observed_at', 'ttl_seconds', 'minimum_bucket_size',
  'total', 'by_role', 'by_version', 'by_region',
]);

function counts(value, keys, total) {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null;
  const entries = Object.entries(value);
  if (!entries.every(([key, count]) => keys.includes(key) && Number.isSafeInteger(count)
    && count >= MINIMUM_BUCKET_SIZE && count <= 4096)) return null;
  // Completeness prevents an exact total from disclosing a hidden residual.
  if (entries.reduce((sum, [, count]) => sum + count, 0) !== (total ?? 0)) return null;
  return Object.fromEntries(entries);
}

/** Only schema 2 contains no individual records. The timestamp is a 10-minute
 * release bucket; counts remain fixed for that bucket at the producer. */
export function parsePresence(answer, now = Math.floor(Date.now() / 1000)) {
  if (!answer || answer.schema !== 2 || answer.available !== true || answer.scope !== SCOPE
    || Object.keys(answer).some((key) => !FIELDS.has(key))
    || !Number.isSafeInteger(now) || now < 0
    || (answer.total !== null && (!Number.isSafeInteger(answer.total)
      || answer.total < MINIMUM_BUCKET_SIZE || answer.total > 4096))
    || !Number.isSafeInteger(answer.observed_at) || answer.observed_at < 0
    || answer.observed_at % WINDOW_SECONDS !== 0
    || answer.ttl_seconds !== WINDOW_SECONDS
    || answer.minimum_bucket_size !== MINIMUM_BUCKET_SIZE
    || now - answer.observed_at >= WINDOW_SECONDS
    || answer.observed_at - now > 60) return null;
  const byRole = counts(answer.by_role, PRESENCE_ROLES, answer.total);
  const byVersion = counts(answer.by_version, ['unknown'], answer.total);
  const byRegion = counts(answer.by_region, PRESENCE_REGIONS.map(([key]) => key), answer.total);
  if (!byRole || !byVersion || !byRegion) return null;
  return { total: answer.total, byRole, byVersion, byRegion, ttlSeconds: answer.ttl_seconds };
}

export async function readPresence(node) {
  try {
    return parsePresence(await node.call('aether_presence', []));
  } catch {
    return null;
  }
}
