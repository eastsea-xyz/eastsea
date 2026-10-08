// Quality is a deterministic model over supplied evidence; no RPC or identities.
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  QUALITY_VERSION, QUALITY_BINS, QUALITY_SCALE,
  qualityScore, qualityFromChain, summarizeQuality, qualityMean, qualityColor,
  qualityDensity,
} from '../live-globe/quality.js';

const close = (actual, expected, tolerance = 1e-12) => {
  assert.ok(Math.abs(actual - expected) <= tolerance, `${actual} != ${expected}`);
};
const chain = (extras = {}) => ({
  epoch: 5, epochHours: 1, registeredEpoch: 0, beaconEpochs: [0, 1, 2, 3, 4],
  ...extras,
});

test('the public quality encoding has one version and twenty numeric bins', () => {
  assert.equal(QUALITY_VERSION, 1);
  assert.equal(QUALITY_BINS, 20);
  assert.equal(QUALITY_SCALE, 1_000_000);
});

test('missing uptime and proof evidence gives the supplied-hour lower bounds', () => {
  assert.deepEqual([152, 24, 10, 9].map(streakHours => (
    summarizeQuality([qualityScore({ streakHours })]).score_sum
  )), [123705, 21309, 8965, 8074]);
});

test('thirty days of successful operation reaches one exponential time constant', () => {
  close(qualityScore({ streakHours: 720, uptime: 1, proofCoverage: 1 }), 1 - Math.exp(-1));
});

test('proof coverage contributes at most ten percent of time credit', () => {
  const time = 1 - Math.exp(-152 / 720);
  const base = { streakHours: 152, uptime: 1 };
  close(qualityScore(base), time * .90);
  close(qualityScore({ ...base, proofCoverage: 1 }) - qualityScore(base), time * .10);
  assert.equal(qualityScore({ streakHours: 152, proofCoverage: 1 }), qualityScore({ streakHours: 152 }));
});

test('operation duration is continuous across familiar hour thresholds', () => {
  const delta = .0001;
  for (const hours of [0, 24, 72, 168, 720, 10_000]) {
    const low = qualityScore({ streakHours: Math.max(0, hours - delta), uptime: 1, proofCoverage: 1 });
    const high = qualityScore({ streakHours: hours + delta, uptime: 1, proofCoverage: 1 });
    assert.ok(high > low);
    assert.ok(high - low < 2 * delta / 720);
  }
});

test('absence halves score every seventy-two hours without overflow', () => {
  const base = { streakHours: 720, uptime: 1, proofCoverage: 1 };
  close(qualityScore({ ...base, absenceHours: 72 }), qualityScore(base) / 2);
  close(qualityScore({ ...base, absenceHours: 144 }), qualityScore(base) / 4);
  assert.equal(qualityScore({ ...base, absenceHours: Number.MAX_VALUE }), 0);
  assert.equal(qualityScore({ streakHours: Number.MAX_VALUE, uptime: 1, proofCoverage: 1 }), 1);
});

test('quality rejects invalid duration, uptime and proof inputs', () => {
  for (const value of [-1, Infinity, NaN, '24', null]) {
    assert.throws(() => qualityScore({ streakHours: value }));
    assert.throws(() => qualityScore({ streakHours: 1, absenceHours: value }));
  }
  for (const value of [-.1, 1.01, Infinity, NaN, '1', null]) {
    assert.throws(() => qualityScore({ streakHours: 1, uptime: value }));
    assert.throws(() => qualityScore({ streakHours: 1, proofCoverage: value }));
  }
  assert.throws(() => qualityScore({}));
});

test('registration without an actual successful beacon earns no quality', () => {
  assert.equal(qualityFromChain(chain({ beaconEpochs: [] })), 0);
  assert.equal(qualityFromChain(chain({ epoch: 0, beaconEpochs: [] })), 0);
});

test('completed finalized successful beacon epochs determine earned hours', () => {
  close(qualityFromChain(chain()), qualityScore({ streakHours: 5, uptime: 1 }));
  close(qualityFromChain(chain({ epochHours: .5 })), qualityScore({ streakHours: 2.5, uptime: 1 }));
});

test('one short missed epoch reduces earned duration and uptime without resetting history', () => {
  const observed = chain({ beaconEpochs: [0, 1, 3, 4] });
  close(qualityFromChain(observed), qualityScore({ streakHours: 4, uptime: 4 / 5 }));
  assert.ok(qualityFromChain(observed) < qualityFromChain(chain()));
  assert.ok(qualityFromChain(observed) > qualityScore({ streakHours: 2, uptime: 1 }));
});

test('currently absent epochs remain in the same uptime observation window', () => {
  close(qualityFromChain(chain({ beaconEpochs: [0, 1, 3] })), qualityScore({
    streakHours: 3, uptime: 3 / 5, absenceHours: 1,
  }));
});

test('twenty-four missing hours reset maturity when a Mac returns', () => {
  const old = [0, 1, 2, 3];
  close(qualityFromChain(chain({ epoch: 30, beaconEpochs: [...old, 28, 29], proofEpochs: old })),
    qualityScore({ streakHours: 2, uptime: 1 }));
});

test('a shorter twenty-three-hour hole counts as misses in the retained segment', () => {
  close(qualityFromChain(chain({ epoch: 28, beaconEpochs: [0, 1, 25, 26, 27] })),
    qualityScore({ streakHours: 5, uptime: 5 / 28 }));
});

test('the long-gap boundary uses hours rather than a fixed number of epochs', () => {
  close(qualityFromChain(chain({ epoch: 16, epochHours: 2, beaconEpochs: [0, 1, 14, 15] })),
    qualityScore({ streakHours: 4, uptime: 1 }));
});

test('announced sleep and preserved registry credit cannot restore an old quality score', () => {
  const returned = chain({
    epoch: 202, beaconEpochs: [0, 1, 2, 3, 200, 201], proofEpochs: [0, 1, 2, 3],
    streak: 999, missed: 0, announcedSleep: true,
  });
  close(qualityFromChain(returned), qualityScore({ streakHours: 2, uptime: 1 }));
});

test('duplicate and reordered beacon and proof epochs cannot increase quality', () => {
  const input = chain({ registeredEpoch: 1, beaconEpochs: [4, 1, 3, 2, 1, 4], proofEpochs: [4, 3, 2, 1, 4, 1] });
  close(qualityFromChain(input), qualityScore({ streakHours: 4, uptime: 1, proofCoverage: 1 }));
});

test('proof acceptance counts only once per successful epoch in the same segment', () => {
  close(qualityFromChain(chain({ beaconEpochs: [0, 1, 3, 4], proofEpochs: [1, 1, 2, 4] })),
    qualityScore({ streakHours: 4, uptime: 4 / 5, proofCoverage: 2 / 4 }));
});

test('future and pre-registration evidence never contributes to the finalized score', () => {
  close(qualityFromChain(chain({
    registeredEpoch: 2, beaconEpochs: [0, 1, 2, 3, 4, 5, 900], proofEpochs: [0, 1, 2, 5, 900],
  })), qualityScore({ streakHours: 3, uptime: 1, proofCoverage: 1 / 3 }));
});

test('invalid chain epochs and durations fail instead of manufacturing time', () => {
  for (const value of [-1, .5, Infinity, NaN, '5', Number.MAX_SAFE_INTEGER + 1]) {
    assert.throws(() => qualityFromChain(chain({ epoch: value })));
    assert.throws(() => qualityFromChain(chain({ registeredEpoch: value })));
    assert.throws(() => qualityFromChain(chain({ beaconEpochs: [value] })));
    assert.throws(() => qualityFromChain(chain({ proofEpochs: [value] })));
  }
  for (const value of [0, -1, Infinity, NaN, '1']) {
    assert.throws(() => qualityFromChain(chain({ epochHours: value })));
  }
  assert.throws(() => qualityFromChain(chain({ registeredEpoch: 6 })));
  assert.throws(() => qualityFromChain(chain({ beaconEpochs: null })));
  assert.throws(() => qualityFromChain(chain({ proofEpochs: {} })));
  assert.throws(() => qualityFromChain(chain({ epochHours: Number.MAX_VALUE })));
});

test('balances and operator identities do not affect quality', () => {
  assert.equal(qualityFromChain(chain({ balance: 1_000_000, founder: true, identity: 'operator' })), qualityFromChain(chain()));
  assert.equal(qualityScore({ streakHours: 24, balance: 1_000_000, founder: true }), qualityScore({ streakHours: 24 }));
});

test('aggregation quantizes each score once and preserves exact bin boundaries', () => {
  const summary = summarizeQuality([0, .0499999, .05, .9499999, .95, 1]);
  assert.equal(summary.score_sum, 2_999_998);
  assert.deepEqual(summary.histogram, [2, 1, ...Array(16).fill(0), 1, 2]);
});

test('means weight individual populations rather than averaging regional means', () => {
  const small = summarizeQuality([1]);
  const large = summarizeQuality([0, 0, 0]);
  const merged = { score_sum: small.score_sum + large.score_sum, histogram: small.histogram.map((n, i) => n + large.histogram[i]) };
  assert.equal(qualityMean(merged, 4), .25);
  assert.equal(qualityMean(summarizeQuality([]), 0), 0);
});

test('quality aggregation and means reject invalid or inconsistent values', () => {
  for (const score of [-.01, 1.01, Infinity, NaN, '1']) {
    assert.throws(() => summarizeQuality([score]));
  }
  assert.throws(() => summarizeQuality(null));
  for (const count of [-1, .5, Infinity, '1']) {
    assert.throws(() => qualityMean(summarizeQuality([1]), count));
  }
  assert.throws(() => qualityMean({ score_sum: Number.MAX_SAFE_INTEGER + 1, histogram: Array(20).fill(0) }, 1));
  assert.throws(() => qualityMean(summarizeQuality([1]), 0));
  assert.throws(() => qualityMean(summarizeQuality([1]), 2));
});

test('large aggregate populations remain precise and count overflow is rejected', () => {
  const histogram = [Number.MAX_SAFE_INTEGER, ...Array(19).fill(0)];
  assert.equal(qualityMean({ score_sum: 0, histogram }, Number.MAX_SAFE_INTEGER), 0);
  assert.throws(() => qualityMean({
    score_sum: 0, histogram: [Number.MAX_SAFE_INTEGER, 1, ...Array(18).fill(0)],
  }, Number.MAX_SAFE_INTEGER));
});

test('histograms cannot claim scores outside their quantized bin bounds', () => {
  const histogram = [0, 1, ...Array(18).fill(0)];
  assert.throws(() => qualityMean({ score_sum: 49_999, histogram }, 1));
  assert.throws(() => qualityMean({ score_sum: 100_000, histogram }, 1));
  assert.equal(qualityMean({ score_sum: 50_000, histogram }, 1), .05);
});

test('score color interpolates continuously between the two theme tokens', () => {
  assert.equal(qualityColor(0), '#7cc4dc');
  assert.equal(qualityColor(1), '#5ccb98');
  assert.equal(qualityColor(.5), '#6cc8ba');
  assert.equal(qualityColor(.25, '#000000', '#ffffff'), '#404040');
  assert.equal(qualityColor(.75, '#000000', '#ffffff'), '#bfbfbf');
});

test('color input accepts only bounded scores and six-digit RGB theme colors', () => {
  for (const score of [-1, 1.01, NaN, Infinity, '1']) assert.throws(() => qualityColor(score));
  for (const color of ['red', '#fff', '#gg0000', '#ffffff00', null]) assert.throws(() => qualityColor(.5, color));
});

test('density provides twenty-one smooth finite samples without tier counts', () => {
  const density = qualityDensity(summarizeQuality([.45, .46, .50, .51]));
  assert.equal(density.length, 21);
  assert.ok(density.every(value => Number.isFinite(value) && value >= 0 && value <= 1));
  assert.ok(density[9] > 0 && density[10] > 0 && density[11] > 0);
  assert.ok(density[10] >= density[9] && density[10] >= density[11]);
  assert.deepEqual(qualityDensity(summarizeQuality([])), Array(21).fill(0));
});
