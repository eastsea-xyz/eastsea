import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { ACCOUNT_ICON_VERSION, deriveAccountIcon, accountIconSVG } from '../site/account-icon.js';
import { CURVE_SAMPLES, FACE_LAYOUT_THRESHOLDS, ROUND2_ORANGE_ADDRESS, ROUND2_ORANGE_SVG,
  samplePath, polygonGeometry, geometryFromSVG, detectFaceLikeLayout, secondaryIslandPlacement } from './account-icon-layout.mjs';

const rectangle = (x, y, width, height) => `<path d="M ${x} ${y} L ${x + width} ${y} L ${x + width} ${y + height} L ${x} ${y + height} Z"/>`;
const svg = (paths, transform = '') => `<svg viewBox="0 0 64 64"><g${transform ? ` transform="${transform}"` : ''}>${paths.join('')}</g></svg>`;
const face = (left = rectangle(8, 8, 8, 5), right = rectangle(36, 8, 8, 5), bottom = rectangle(12, 32, 36, 14), transform = '') => svg([left, right, bottom], transform);
const close = (actual, expected, tolerance = 1e-8) => assert.ok(Math.abs(actual - expected) <= tolerance, `${actual} differs from ${expected}`);

test('filled path area and centroid use shoelace, including clockwise paths', () => {
  const points = [[2, 4], [12, 4], [12, 10], [2, 10]];
  for (const ordered of [points, points.toReversed()]) {
    const geometry = polygonGeometry(ordered);
    close(geometry.area, 60);
    assert.deepEqual(geometry.centroid, { x: 7, y: 7 });
    assert.deepEqual(geometry.bounds, { minX: 2, minY: 4, maxX: 12, maxY: 10, width: 10, height: 6 });
  }
});

test('cubic coastline sampling measures the filled curve rather than its control box', () => {
  const points = samplePath('M 0 0 C 0 6 6 6 6 0 Z');
  assert.equal(points.length, CURVE_SAMPLES + 1);
  const geometry = polygonGeometry(points);
  close(geometry.area, 21.6, 0.01);
  close(geometry.centroid.x, 3);
  close(geometry.bounds.maxY, 4.5);
  assert.ok(geometry.area < geometry.bounds.width * geometry.bounds.height);
});

test('SVG path and group transforms follow actual SVG composition order', () => {
  const plain = '<svg><g transform="rotate(180 32 32)"><path d="M 2 4 L 12 4 L 12 10 L 2 10 Z" transform="translate(0 5) scale(1 0.8)"/></g></svg>';
  const [geometry] = geometryFromSVG(plain);
  close(geometry.area, 48);
  close(geometry.centroid.x, 57);
  close(geometry.centroid.y, 53.4);
  close(geometry.bounds.minY, 51);
  close(geometry.bounds.maxY, 55.8);
});

test('round 2 orange review icon is a non-vacuous positive control', () => {
  const result = detectFaceLikeLayout(ROUND2_ORANGE_SVG);
  assert.equal(result.flagged, true);
  assert.equal(result.matches.length, 1);
  assert.deepEqual(result.matches[0].smaller, [1, 2]);
  assert.equal(result.matches[0].larger, 0);
  close(result.matches[0].smallerAreaRatio, 1);
  assert.ok(result.elements[1].centroid.y < result.elements[0].centroid.y);
  assert.ok(result.elements[2].centroid.y < result.elements[0].centroid.y);
  assert.equal(Number(secondaryIslandPlacement(result.elements).bothAboveMain), 1);
  assert.equal(secondaryIslandPlacement(result.elements).mainBetweenSecondaryCentroids, false);
});

test('two similar small horizontally separated elements above a larger form are flagged', () => {
  assert.equal(detectFaceLikeLayout(face()).flagged, true);
  assert.equal(detectFaceLikeLayout(svg([rectangle(12, 32, 36, 14), rectangle(36, 8, 8, 5), rectangle(8, 8, 8, 5)])).flagged, true);
  assert.equal(detectFaceLikeLayout(face(undefined, undefined, undefined, 'rotate(90 32 32)')).flagged, false);
  assert.equal(detectFaceLikeLayout(face(undefined, undefined, undefined, 'rotate(180 32 32)')).flagged, false);
  const inverted = face(rectangle(8, 42, 8, 5), rectangle(36, 42, 8, 5), rectangle(12, 8, 36, 14), 'rotate(180 32 32)');
  assert.equal(detectFaceLikeLayout(inverted).flagged, true);
});

test('area similarity threshold is inclusive and rejects a genuinely smaller second island', () => {
  assert.equal(FACE_LAYOUT_THRESHOLDS.minimumSmallerAreaRatio, 0.65);
  const left = rectangle(8, 8, 10, 4);
  assert.equal(detectFaceLikeLayout(face(left, rectangle(36, 8, 6.5, 4))).flagged, true);
  assert.equal(detectFaceLikeLayout(face(left, rectangle(36, 8, 6.499, 4))).flagged, false);
  assert.equal(detectFaceLikeLayout(face(left, rectangle(36, 8, 3, 4))).flagged, false);
});

test('stronger secondary placement check catches unequal eyes and validates opposite-side centroids', () => {
  const unequalFace = svg([rectangle(12, 32, 36, 14), rectangle(8, 8, 10, 4), rectangle(36, 8, 3, 4)]);
  assert.equal(detectFaceLikeLayout(unequalFace).flagged, false);
  assert.equal(secondaryIslandPlacement(unequalFace).bothAboveMain, true);
  const opposite = [rectangle(24, 24, 16, 16), rectangle(8, 44, 10, 4), rectangle(44, 8, 3, 4)];
  for (const rotation of [0, 90, 180, 270]) {
    const placement = secondaryIslandPlacement(svg(opposite, `rotate(${rotation} 32 32)`));
    assert.equal(placement.bothAboveMain, false);
    assert.equal(placement.mainBetweenSecondaryCentroids, true);
    assert.ok(placement.bracketingMargins.x > 0 && placement.bracketingMargins.y > 0);
  }
  assert.throws(() => secondaryIslandPlacement(svg([rectangle(24, 24, 16, 16)])), TypeError);
});

test('vertical staggering threshold uses centroid separation and mean actual height', () => {
  const left = rectangle(8, 8, 8, 4);
  assert.equal(detectFaceLikeLayout(face(left, rectangle(36, 10, 8, 4))).flagged, true);
  assert.equal(detectFaceLikeLayout(face(left, rectangle(36, 10.001, 8, 4))).flagged, false);
  assert.equal(detectFaceLikeLayout(face(left, rectangle(36, 16, 8, 4))).flagged, false);
});

test('larger-form ratio and horizontal separation are checked at their numeric boundaries', () => {
  const left = rectangle(8, 8, 6.5, 4), right = rectangle(36, 8, 6.5, 4);
  assert.equal(detectFaceLikeLayout(face(left, right, rectangle(20, 32, 10, 4))).flagged, true);
  assert.equal(detectFaceLikeLayout(face(left, right, rectangle(20, 32, 9.999, 4))).flagged, false);
  assert.equal(detectFaceLikeLayout(face(rectangle(8, 8, 8, 4), rectangle(16, 8, 8, 4))).flagged, true);
  assert.equal(detectFaceLikeLayout(face(rectangle(8, 8, 8, 4), rectangle(15.999, 8, 8, 4))).flagged, false);
});

test('elements below the larger centroid and one or two land paths cannot form the pattern', () => {
  assert.equal(detectFaceLikeLayout(face(rectangle(8, 42, 8, 5), rectangle(36, 42, 8, 5))).flagged, false);
  assert.equal(detectFaceLikeLayout(svg([rectangle(12, 32, 36, 14)])).flagged, false);
  assert.equal(detectFaceLikeLayout(svg([rectangle(8, 8, 8, 5), rectangle(12, 32, 36, 14)])).flagged, false);
});

test('unsupported or degenerate path input fails rather than silently dropping geometry', () => {
  for (const path of ['M 0 0 Q 5 5 10 0 Z', 'm 0 0 l 5 0 l 0 5 z', 'M 0 0 L 5 0', 'M 0 0 Z', 'M 0 0 L 5 Z', 'M 0 0 L 5 0 L 5 5 Z M 8 8 L 9 8 L 9 9 Z']) assert.throws(() => samplePath(path), TypeError);
  assert.throws(() => polygonGeometry([[0, 0], [1, 0], [2, 0]]), TypeError);
  assert.throws(() => geometryFromSVG('<svg><g><path d="M 0 0 L 5 0 L 5 5 Z" transform="skewX(20)"/></g></svg>'), TypeError);
  assert.throws(() => geometryFromSVG('<svg/>'), TypeError);
});

test('round 3 representative emitted address icons are clear at 32px and 64px', () => {
  assert.ok(ACCOUNT_ICON_VERSION >= 3, 'v2 requires a geometry fix before this regression can pass');
  const addresses = [ROUND2_ORANGE_ADDRESS, '0xa2521982a17474cb2f8741c85de653b5282d72b0',
    '0x0000000000000000000000000000000000000000', '0xffffffffffffffffffffffffffffffffffffffff',
    '0x1111111111111111111111111111111111111111'];
  for (const address of addresses) for (const size of [32, 64]) {
    const result = detectFaceLikeLayout(accountIconSVG(deriveAccountIcon(address), size));
    assert.equal(result.flagged, false, `${address} at ${size}px`);
    assert.equal(secondaryIslandPlacement(result.elements).bothAboveMain, false, `${address} at ${size}px`);
    assert.equal(result.elements.length, 3);
    const [, first, second] = result.elements;
    assert.ok(Math.min(first.area, second.area) / Math.max(first.area, second.area) < 0.65);
  }
});

test('all 16 golden addresses keep the main centroid between the secondary centroids at every quarter-turn', () => {
  const fixture = JSON.parse(readFileSync(new URL('../crates/client/tests/account-icon-vectors.json', import.meta.url), 'utf8'));
  assert.equal(fixture.vectors.length, 16);
  assert.equal(fixture.version, ACCOUNT_ICON_VERSION);
  for (const vector of fixture.vectors) {
    const derived = deriveAccountIcon(vector.address);
    assert.deepEqual(derived, vector.features);
    for (const rotation of [0, 1, 2, 3]) for (const size of [32, 64]) {
      const emitted = accountIconSVG({ ...derived, rotation }, size);
      const placement = secondaryIslandPlacement(emitted);
      assert.equal(placement.bothAboveMain, false, `${vector.address}, rotation ${rotation}, ${size}px`);
      assert.equal(placement.mainBetweenSecondaryCentroids, true, `${vector.address}, rotation ${rotation}, ${size}px`);
      assert.equal(detectFaceLikeLayout(emitted).flagged, false);
    }
  }
});

test('path geometry caching preserves independent transforms and measured values', () => {
  const cache = new Map();
  const source = face(), first = geometryFromSVG(source, { cache });
  assert.deepEqual(geometryFromSVG(source, { cache }), first);
  assert.equal(cache.size, 3);
  const rotated = geometryFromSVG(face(undefined, undefined, undefined, 'rotate(180 32 32)'), { cache });
  assert.equal(cache.size, 6);
  assert.notDeepEqual(rotated[0].centroid, first[0].centroid);
  close(rotated[0].area, first[0].area);
});
