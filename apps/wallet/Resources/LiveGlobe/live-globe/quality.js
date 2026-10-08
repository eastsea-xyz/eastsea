// A continuous operation score. No identity, balance, role or named tier enters it.
export const QUALITY_VERSION = 1;
export const QUALITY_BINS = 20;
export const QUALITY_SCALE = 1_000_000;

const TIME_HOURS = 720;
const ABSENCE_HALF_LIFE = 72;
const RESET_GAP_HOURS = 24;
const BIN_WIDTH = QUALITY_SCALE / QUALITY_BINS;

function invalid() { throw new TypeError('Invalid quality data.'); }

function duration(value) {
  if (typeof value !== 'number' || !Number.isFinite(value) || value < 0) invalid();
  return value;
}

function unit(value) {
  if (duration(value) > 1) invalid();
  return value;
}

function count(value) {
  if (!Number.isSafeInteger(value) || value < 0) invalid();
  return value;
}

function add(a, b) { return count(a + b); }

/** Unknown uptime/proof coverage earns only the conservative time component. */
export function qualityScore({ streakHours, uptime = 0, proofCoverage = 0, absenceHours = 0 }) {
  duration(streakHours);
  unit(uptime);
  unit(proofCoverage);
  duration(absenceHours);
  const time = -Math.expm1(-streakHours / TIME_HOURS);
  const score = time * (.65 + .25 * uptime + .10 * uptime * proofCoverage)
    * 2 ** (-absenceHours / ABSENCE_HALF_LIFE);
  return Math.max(0, Math.min(1, score));
}

function epochs(value) {
  if (!Array.isArray(value)) invalid();
  for (const epoch of value) count(epoch);
  return value;
}

/**
 * Producer-only calculation, not a public browser-input contract. The producer
 * must supply complete successful-beacon evidence for the reconstructed segment
 * and accepted proof epochs attributable to this particular Mac, all from the
 * same finalized chain snapshot. A shared payout address alone cannot attribute
 * an operator's proofs to each Mac. `epoch` is the current finalized epoch;
 * its still-open epoch and pre-registration evidence earn no credit.
 *
 * Registration and announced availability are not successful beacons. Missing
 * gaps of 24 hours restart the segment on return even if the registry preserved
 * its streak during announced sleep. Shorter holes reduce uptime instead.
 */
export function qualityFromChain({ epoch, epochHours, registeredEpoch, beaconEpochs, proofEpochs = [] }) {
  count(epoch);
  count(registeredEpoch);
  duration(epochHours);
  if (epochHours === 0 || registeredEpoch > epoch) invalid();
  epochs(beaconEpochs);
  epochs(proofEpochs);

  const successful = [...new Set(beaconEpochs.filter(n => n >= registeredEpoch && n < epoch))]
    .sort((a, b) => a - b);
  if (!successful.length) return 0;

  let first = successful.length - 1;
  while (first > 0) {
    const missingHours = duration((successful[first] - successful[first - 1] - 1) * epochHours);
    if (missingHours >= RESET_GAP_HOURS) break;
    first--;
  }
  const segment = successful.slice(first);
  const last = segment[segment.length - 1];
  const absenceEpochs = Math.max(0, epoch - 1 - last);
  // Equals last-first+1+absence, without a potentially overflowing addition.
  const observedEpochs = epoch - segment[0];
  const proofSet = new Set(proofEpochs);
  const covered = segment.reduce((total, n) => total + Number(proofSet.has(n)), 0);
  return qualityScore({
    streakHours: duration(segment.length * epochHours),
    uptime: segment.length / observedEpochs,
    proofCoverage: covered / segment.length,
    absenceHours: duration(absenceEpochs * epochHours),
  });
}

/** Quantize once so the mean and histogram describe exactly the same scores. */
export function summarizeQuality(scores) {
  if (!Array.isArray(scores)) invalid();
  const histogram = Array(QUALITY_BINS).fill(0);
  let score_sum = 0;
  for (const score of scores) {
    const units = Math.floor(unit(score) * QUALITY_SCALE);
    score_sum = add(score_sum, units);
    const bin = Math.min(QUALITY_BINS - 1, Math.floor(units / BIN_WIDTH));
    histogram[bin] = add(histogram[bin], 1);
  }
  return { score_sum, histogram };
}

function population(quality) {
  if (!quality || typeof quality !== 'object' || Array.isArray(quality)) invalid();
  const score = count(quality.score_sum);
  if (!Array.isArray(quality.histogram) || quality.histogram.length !== QUALITY_BINS) invalid();
  let total = 0;
  let minimum = 0n;
  let maximum = 0n;
  for (let bin = 0; bin < QUALITY_BINS; bin++) {
    const size = count(quality.histogram[bin]);
    total = add(total, size);
    const lower = bin * BIN_WIDTH;
    const upper = bin === QUALITY_BINS - 1 ? QUALITY_SCALE : (bin + 1) * BIN_WIDTH - 1;
    // Valid counts can have a bound beyond JSON's safe range even when the
    // actual score sum is representable. Compare these bounds exactly.
    minimum += BigInt(size) * BigInt(lower);
    maximum += BigInt(size) * BigInt(upper);
  }
  if (BigInt(score) < minimum || BigInt(score) > maximum) invalid();
  return total;
}

/** A population-weighted mean; division order never multiplies huge counts. */
export function qualityMean(quality, size) {
  count(size);
  if (population(quality) !== size) invalid();
  return size ? quality.score_sum / size / QUALITY_SCALE : 0;
}

function rgb(color) {
  if (typeof color !== 'string' || !/^#[0-9a-f]{6}$/i.test(color)) invalid();
  return [1, 3, 5].map(at => Number.parseInt(color.slice(at, at + 2), 16));
}

/** Continuous sRGB interpolation, also used for the shared end-labelled legend. */
export function qualityColor(score, start = '#7CC4DC', end = '#5CCB98') {
  unit(score);
  const from = rgb(start);
  const to = rgb(end);
  return '#' + from.map((value, i) => Math.round(value + (to[i] - value) * score)
    .toString(16).padStart(2, '0')).join('');
}

/** Twenty-one peak-normalized samples interpolated between neighboring bins. */
export function qualityDensity(quality) {
  population(quality);
  const bins = quality.histogram;
  const peak = Math.max(...bins);
  if (!peak) return Array(QUALITY_BINS + 1).fill(0);
  return Array.from({ length: QUALITY_BINS + 1 }, (_, at) => {
    const left = bins[Math.max(0, at - 1)] / peak;
    const right = bins[Math.min(QUALITY_BINS - 1, at)] / peak;
    return (left + right) / 2;
  });
}
