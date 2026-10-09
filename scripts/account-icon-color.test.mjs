import test from 'node:test';
import assert from 'node:assert/strict';
import { contrast, deltaE2000, interpolateColor, luminance, srgbToLab } from './account-icon-color.mjs';

const close = (actual, expected, tolerance = 0.00005) => assert.ok(Math.abs(actual - expected) <= tolerance, `${actual} differs from ${expected}`);

test('CIEDE2000 matches Sharma, Wu and Dalal reference cases, including hue discontinuities', () => {
  // Published supplementary data, rows 1–20. Cases 9–15 straddle 180°;
  // cases 7–8 cover zero chroma. Reversing every pair also checks symmetry.
  // https://hajim.rochester.edu/ece/sites/gsharma/ciede2000/dataNprograms/ciede2000testdata.txt
  const cases = [
    [50, 2.6772, -79.7751, 50, 0, -82.7485, 2.0425],
    [50, 3.1571, -77.2803, 50, 0, -82.7485, 2.8615],
    [50, 2.8361, -74.0200, 50, 0, -82.7485, 3.4412],
    [50, -1.3802, -84.2814, 50, 0, -82.7485, 1],
    [50, -1.1848, -84.8006, 50, 0, -82.7485, 1],
    [50, -0.9009, -85.5211, 50, 0, -82.7485, 1],
    [50, 0, 0, 50, -1, 2, 2.3669],
    [50, -1, 2, 50, 0, 0, 2.3669],
    [50, 2.49, -0.001, 50, -2.49, 0.0009, 7.1792],
    [50, 2.49, -0.001, 50, -2.49, 0.001, 7.1792],
    [50, 2.49, -0.001, 50, -2.49, 0.0011, 7.2195],
    [50, 2.49, -0.001, 50, -2.49, 0.0012, 7.2195],
    [50, -0.001, 2.49, 50, 0.0009, -2.49, 4.8045],
    [50, -0.001, 2.49, 50, 0.001, -2.49, 4.8045],
    [50, -0.001, 2.49, 50, 0.0011, -2.49, 4.7461],
    [50, 2.5, 0, 50, 0, -2.5, 4.3065],
    [50, 2.5, 0, 73, 25, -18, 27.1492],
    [50, 2.5, 0, 61, -5, 29, 22.8977],
    [50, 2.5, 0, 56, -27, -3, 31.9030],
    [50, 2.5, 0, 58, 24, 15, 19.4535],
  ];
  for (const row of cases) {
    const a = row.slice(0, 3), b = row.slice(3, 6);
    close(deltaE2000(a, b), row[6]);
    close(deltaE2000(b, a), row[6]);
  }
});

test('D65 Lab conversion preserves achromatic endpoints and standard sRGB primaries', () => {
  assert.deepEqual(srgbToLab('#000000'), [0, 0, 0]);
  for (const [color, lab] of [
    ['#ffffff', [100, 0, 0]],
    ['#ff0000', [53.2408, 80.0925, 67.2032]],
    ['#00ff00', [87.7347, -86.1827, 83.1793]],
    ['#0000ff', [32.2970, 79.1875, -107.8602]],
  ]) srgbToLab(color).forEach((value, index) => close(value, lab[index], 0.0001));
  assert.deepEqual(srgbToLab([1, 0, 0]), srgbToLab('#ff0000'));
  close(deltaE2000(srgbToLab('#0f5a75'), srgbToLab('#0f5a75')), 0);
});

test('relative luminance and contrast use linearized sRGB', () => {
  close(luminance('#000000'), 0);
  close(luminance('#ffffff'), 1);
  close(luminance('#ff0000'), 0.2126);
  close(luminance('#00ff00'), 0.7152);
  close(luminance('#0000ff'), 0.0722);
  close(luminance('#777777'), 0.1844749945);
  close(contrast('#000000', '#ffffff'), 21);
  close(contrast('#777777', '#ffffff'), 4.4780894536);
  assert.equal(contrast('#0d2135', '#f4efe6'), contrast('#f4efe6', '#0d2135'));
});

test('encoded sRGB interpolation matches the icon gradient midpoint', () => {
  assert.equal(interpolateColor('#0d2135', '#e8bf59', 0), '#0d2135');
  assert.equal(interpolateColor('#0d2135', '#e8bf59', 1), '#e8bf59');
  assert.equal(interpolateColor('#0d2135', '#e8bf59'), '#7b7047');
  assert.equal(interpolateColor('#000000', '#ffffff', 0.5), '#808080');
});

test('malformed colors and nonfinite Lab values fail explicitly', () => {
  for (const color of ['red', '#123', '#abcdefg', [255, 0, 0], [0, NaN, 0], [0, 0]]) assert.throws(() => srgbToLab(color), TypeError);
  assert.throws(() => deltaE2000([50, NaN, 0], [50, 0, 0]), TypeError);
  assert.throws(() => interpolateColor('#0d2135', '#e8bf59', -1), RangeError);
  assert.throws(() => interpolateColor('#0d2135', '#e8bf59', Infinity), RangeError);
});
