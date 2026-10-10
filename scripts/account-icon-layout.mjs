#!/usr/bin/env node
// Exhaustive/scale measurements run on guarded poc-m3, never the development Mac.
import { createHash } from 'node:crypto';
import { readFileSync, writeFileSync } from 'node:fs';
import { hostname } from 'node:os';
import { performance } from 'node:perf_hooks';
import { fileURLToPath } from 'node:url';
import {
  ACCOUNT_ICON_VERSION, accountIconSVG,
} from '../site/account-icon.js';

export const FACE_LAYOUT_THRESHOLDS = Object.freeze({
  minimumSmallerAreaRatio: 0.65,
  maximumSmallerToLargerAreaRatio: 0.65,
  maximumVerticalSeparationInMeanHeights: 0.5,
  minimumHorizontalSeparationInMeanWidths: 1,
});
export const CURVE_SAMPLES = 64;
export const ROUND2_ORANGE_ADDRESS = '0x1234567890abcdef1234567890abcdef12345678';

// The actual SVG land group emitted at HEAD 20e669a for the orange review icon.
// Keeping a fixed positive control avoids relying on the changing generator,
// git availability or a retired algorithm implementation at measurement time.
export const ROUND2_ORANGE_SVG = '<svg viewBox="0 0 64 64"><g transform="rotate(180 32 32)"><path d="M 11 10 L 25 10 L 25 32 C 25 45 43 44 43 32 L 43 22 L 55 22 L 55 35 C 55 59 11 58 11 35 Z" transform="translate(0 0) scale(1 0.8)"/><path d="M 9 52 C 11 45 18 49 21 46 L 23 54 C 20 59 12 61 9 52 Z"/><path d="M 35 50 C 37 43 44 47 47 44 L 49 52 C 46 57 38 59 35 50 Z"/></g></svg>';

function attributes(tag) {
  return Object.fromEntries([...tag.matchAll(/([\w:-]+)="([^"]*)"/g)].map((match) => [match[1], match[2]]));
}

// This is deliberately a reader for the canonical icon's closed, absolute
// M/L/C/Z paths, rather than a permissive SVG parser that could silently omit
// an unsupported command. Every cubic contributes 64 polygon edges.
export function samplePath(d, samplesPerCurve = CURVE_SAMPLES) {
  if (typeof d !== 'string' || !Number.isSafeInteger(samplesPerCurve) || samplesPerCurve < 2) throw new TypeError('Invalid path or curve sample count');
  const tokens = d.match(/[MLCZ]|[-+]?(?:\d+\.?\d*|\.\d+)(?:[eE][-+]?\d+)?/g) || [];
  if (d.replace(/[MLCZ]|[-+]?(?:\d+\.?\d*|\.\d+)(?:[eE][-+]?\d+)?/g, '').replace(/[\s,]/g, '')) throw new TypeError('Unsupported SVG path command');
  const points = [];
  let offset = 0, current, closed = false;
  const number = () => {
    const value = Number(tokens[offset++]);
    if (!Number.isFinite(value)) throw new TypeError('Malformed SVG path coordinate');
    return value;
  };
  while (offset < tokens.length) {
    const command = tokens[offset++];
    if (closed || (command !== 'M' && !current)) throw new TypeError('Expected a single closed SVG path');
    if (command === 'M') {
      if (current) throw new TypeError('Multiple SVG subpaths are not supported');
      current = [number(), number()];
      points.push(current);
    } else if (command === 'L') {
      current = [number(), number()];
      points.push(current);
    } else if (command === 'C') {
      const start = current, first = [number(), number()], second = [number(), number()], end = [number(), number()];
      for (let step = 1; step <= samplesPerCurve; step++) {
        const t = step / samplesPerCurve, u = 1 - t;
        points.push([0, 1].map((axis) => u ** 3 * start[axis] + 3 * u ** 2 * t * first[axis] + 3 * u * t ** 2 * second[axis] + t ** 3 * end[axis]));
      }
      current = end;
    } else if (command === 'Z') {
      closed = true;
    } else {
      throw new TypeError('Unsupported or incomplete SVG path command');
    }
  }
  if (!closed || points.length < 3) throw new TypeError('Expected a nonempty closed SVG path');
  return points;
}

// Affine matrices use SVG's [a,b,c,d,e,f] convention. Multiplication preserves
// SVG transform-list order: translate(0 5) scale(1 .8) scales before moving.
function multiply(a, b) {
  return [a[0] * b[0] + a[2] * b[1], a[1] * b[0] + a[3] * b[1],
    a[0] * b[2] + a[2] * b[3], a[1] * b[2] + a[3] * b[3],
    a[0] * b[4] + a[2] * b[5] + a[4], a[1] * b[4] + a[3] * b[5] + a[5]];
}

function transformMatrix(text = '') {
  let result = [1, 0, 0, 1, 0, 0];
  const transforms = [...text.matchAll(/(translate|scale|rotate)\(([^)]*)\)/g)];
  if (text.replace(/(translate|scale|rotate)\(([^)]*)\)/g, '').trim()) throw new TypeError('Unsupported SVG transform');
  for (const [, kind, argumentsText] of transforms) {
    const values = argumentsText.trim().split(/[\s,]+/).map(Number);
    if (!values.length || values.some((value) => !Number.isFinite(value))) throw new TypeError('Malformed SVG transform');
    let next;
    if (kind === 'translate' && (values.length === 1 || values.length === 2)) next = [1, 0, 0, 1, values[0], values[1] ?? 0];
    else if (kind === 'scale' && (values.length === 1 || values.length === 2)) next = [values[0], 0, 0, values[1] ?? values[0], 0, 0];
    else if (kind === 'rotate' && (values.length === 1 || values.length === 3)) {
      const angle = values[0] * Math.PI / 180, cosine = Math.cos(angle), sine = Math.sin(angle);
      next = [cosine, sine, -sine, cosine, 0, 0];
      if (values.length === 3) next = multiply(multiply([1, 0, 0, 1, values[1], values[2]], next), [1, 0, 0, 1, -values[1], -values[2]]);
    } else throw new TypeError('Malformed SVG transform arguments');
    result = multiply(result, next);
  }
  return result;
}

export function polygonGeometry(points) {
  if (!Array.isArray(points) || points.length < 3 || points.some((point) => !Array.isArray(point) || point.length !== 2 || point.some((value) => !Number.isFinite(value)))) throw new TypeError('Invalid polygon');
  let twiceArea = 0, momentX = 0, momentY = 0;
  let minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity;
  for (let index = 0; index < points.length; index++) {
    const [x, y] = points[index], [nextX, nextY] = points[(index + 1) % points.length];
    const cross = x * nextY - nextX * y;
    twiceArea += cross; momentX += (x + nextX) * cross; momentY += (y + nextY) * cross;
    minX = Math.min(minX, x); minY = Math.min(minY, y); maxX = Math.max(maxX, x); maxY = Math.max(maxY, y);
  }
  if (Math.abs(twiceArea) < 1e-9) throw new TypeError('Degenerate SVG path');
  return {
    area: Math.abs(twiceArea) / 2,
    centroid: { x: momentX / (3 * twiceArea), y: momentY / (3 * twiceArea) },
    bounds: { minX, minY, maxX, maxY, width: maxX - minX, height: maxY - minY },
  };
}

function inkGroup(svg) {
  if (typeof svg !== 'string') throw new TypeError('Expected canonical SVG');
  const groups = [...svg.matchAll(/<g\b([^>]*)>([\s\S]*?)<\/g>/g)];
  if (groups.length !== 1 || groups[0][2].includes('<g')) throw new TypeError('Expected one canonical land group');
  return { transform: attributes(groups[0][1]).transform || '', body: groups[0][2] };
}

export function geometryFromSVG(svg, { samplesPerCurve = CURVE_SAMPLES, cache } = {}) {
  const group = inkGroup(svg), paths = [...group.body.matchAll(/<path\b([^>]*)\/>/g)];
  if (!paths.length || group.body.replace(/<path\b([^>]*)\/>/g, '').trim()) throw new TypeError('Expected canonical SVG land paths');
  return paths.map((path, index) => {
    const attrs = attributes(path[1]);
    const key = `${samplesPerCurve}|${group.transform}|${attrs.transform || ''}|${attrs.d}`;
    let geometry = cache?.get(key);
    if (!geometry) {
      const matrix = multiply(transformMatrix(group.transform), transformMatrix(attrs.transform));
      geometry = polygonGeometry(samplePath(attrs.d, samplesPerCurve).map(([x, y]) => [matrix[0] * x + matrix[2] * y + matrix[4], matrix[1] * x + matrix[3] * y + matrix[5]]));
      cache?.set(key, geometry);
    }
    return { pathIndex: index, ...geometry };
  });
}

/** Flags the requested two similar, horizontal small islands above a larger
 * form. Areas and centroids come from the actual filled paths, not boxes or
 * seed bits. A match does not claim that a human necessarily perceives a face. */
export function detectFaceLikeLayout(svgOrGeometry) {
  const elements = typeof svgOrGeometry === 'string' ? geometryFromSVG(svgOrGeometry) : svgOrGeometry;
  if (!Array.isArray(elements)) throw new TypeError('Expected SVG or path geometry');
  const matches = [], thresholds = FACE_LAYOUT_THRESHOLDS;
  for (let larger = 0; larger < elements.length; larger++) {
    const base = elements[larger];
    for (let first = 0; first < elements.length; first++) for (let second = first + 1; second < elements.length; second++) {
      if (first === larger || second === larger) continue;
      const a = elements[first], b = elements[second];
      const smallerAreaRatio = Math.min(a.area, b.area) / Math.max(a.area, b.area);
      const maximumSmallerToLargerAreaRatio = Math.max(a.area, b.area) / base.area;
      const verticalSeparationInMeanHeights = Math.abs(a.centroid.y - b.centroid.y) / ((a.bounds.height + b.bounds.height) / 2);
      const horizontalSeparationInMeanWidths = Math.abs(a.centroid.x - b.centroid.x) / ((a.bounds.width + b.bounds.width) / 2);
      if (smallerAreaRatio + 1e-9 >= thresholds.minimumSmallerAreaRatio
        && maximumSmallerToLargerAreaRatio <= thresholds.maximumSmallerToLargerAreaRatio + 1e-9
        && verticalSeparationInMeanHeights <= thresholds.maximumVerticalSeparationInMeanHeights + 1e-9
        && horizontalSeparationInMeanWidths + 1e-9 >= thresholds.minimumHorizontalSeparationInMeanWidths
        && a.centroid.y < base.centroid.y && b.centroid.y < base.centroid.y) {
        matches.push({ smaller: [first, second], larger, smallerAreaRatio, maximumSmallerToLargerAreaRatio,
          verticalSeparationInMeanHeights, horizontalSeparationInMeanWidths });
      }
    }
  }
  return { flagged: matches.length > 0, matches, elements, thresholds };
}

/** A stronger placement check independent of size similarity: the two known
 * secondary paths cannot both be above the main path's filled-area centroid.
 * Positive bracketing margins in both axes also prove the main centroid lies
 * between the secondary centroids under every quarter-turn. */
export function secondaryIslandPlacement(svgOrGeometry) {
  const elements = typeof svgOrGeometry === 'string' ? geometryFromSVG(svgOrGeometry) : svgOrGeometry;
  if (!Array.isArray(elements) || elements.length !== 3) throw new TypeError('Expected one main and two secondary land paths');
  const [main, first, second] = elements;
  const bracketingMargins = Object.fromEntries(['x', 'y'].map((axis) => [axis, Math.min(
    main.centroid[axis] - Math.min(first.centroid[axis], second.centroid[axis]),
    Math.max(first.centroid[axis], second.centroid[axis]) - main.centroid[axis],
  )]));
  return {
    bothAboveMain: first.centroid.y < main.centroid.y && second.centroid.y < main.centroid.y,
    mainBetweenSecondaryCentroids: bracketingMargins.x > 0 && bracketingMargins.y > 0,
    bracketingMargins,
  };
}

export function exhaustiveLayoutReport() {
  const started = performance.now(), cache = new Map(), flaggedExamples = [];
  let outcomes = 0, flaggedOutcomes = 0, maximumSecondaryAreaRatio = 0, secondaryPairsAboveMain = 0;
  const secondaryAboveMainExamples = [];
  const centroidBracketing = { unrotatedOutcomes: 0, failures: 0, minimumMargins: { x: Infinity, y: Infinity } };
  // Cache individual paths read from the real emitted SVG: 16 main coastlines
  // and 64 alternatives per secondary island, with each group quarter-turn.
  // This samples only a few hundred paths instead of millions of cubics.
  for (let shape = 0; shape < 4; shape++) for (let layout = 0; layout <= 0x3fff; layout++) for (let rotation = 0; rotation < 4; rotation++) {
    const spec = { version: ACCOUNT_ICON_VERSION, palette: 0, shape, layout, rotation };
    const largeSVG = accountIconSVG(spec, 64), smallSVG = accountIconSVG(spec, 32);
    const largeGroup = inkGroup(largeSVG), smallGroup = inkGroup(smallSVG);
    if (largeGroup.transform !== smallGroup.transform || largeGroup.body !== smallGroup.body) throw new Error('32px and 64px paths differ; exhaustively check both size classes');
    const detection = detectFaceLikeLayout(geometryFromSVG(largeSVG, { cache }));
    outcomes++;
    if (detection.elements.length !== 3) throw new Error('Expected three Islands land paths at 32/64px');
    const [, first, second] = detection.elements;
    maximumSecondaryAreaRatio = Math.max(maximumSecondaryAreaRatio, Math.min(first.area, second.area) / Math.max(first.area, second.area));
    const placement = secondaryIslandPlacement(detection.elements);
    if (placement.bothAboveMain) {
      secondaryPairsAboveMain++;
      if (secondaryAboveMainExamples.length < 10) secondaryAboveMainExamples.push({ spec, elements: detection.elements });
    }
    if (rotation === 0) {
      centroidBracketing.unrotatedOutcomes++;
      if (!placement.mainBetweenSecondaryCentroids) centroidBracketing.failures++;
      for (const axis of ['x', 'y']) centroidBracketing.minimumMargins[axis] = Math.min(centroidBracketing.minimumMargins[axis], placement.bracketingMargins[axis]);
    }
    if (detection.flagged) {
      flaggedOutcomes++;
      if (flaggedExamples.length < 10) flaggedExamples.push({ spec, matches: detection.matches });
    }
  }
  const positiveControl = detectFaceLikeLayout(ROUND2_ORANGE_SVG);
  const positivePlacementControl = secondaryIslandPlacement(positiveControl.elements);
  const source = readFileSync(new URL('../site/account-icon.js', import.meta.url), 'utf8');
  const seedDomain = source.match(/const DOMAIN = '([^']+)';/)?.[1];
  if (!seedDomain) throw new Error('Missing canonical seed domain');
  return {
    algorithmVersion: ACCOUNT_ICON_VERSION, seedDomain,
    coverage: { mode: 'exhaustive geometry outcomes', shapes: 4, layouts: 16384, rotations: 4, outcomes,
      sizes: [32, 64], sizeEquivalence: 'Verified identical actual emitted land paths and transforms for every outcome',
      palette: 'Color-independent: all geometry outcomes rendered with palette 0; color does not enter the detector' },
    detector: { thresholds: FACE_LAYOUT_THRESHOLDS, curveSamplesPerSegment: CURVE_SAMPLES,
      geometry: 'Closed actual canonical SVG M/L/C/Z paths; cubic curves sampled uniformly; filled polygon area and centroid by shoelace; path and group transforms applied',
      definition: 'Two elements with min/max area >=0.65, each area <=0.65 of the larger element, centroid Y separation <=0.5 mean heights, centroid X separation >=1 mean width, both centroids above the larger centroid',
      strongerPlacementDefinition: 'Both secondary filled-area centroids above the main filled-area centroid, regardless of their sizes or horizontal alignment; must be absent at every quarter-turn',
      centroidBracketingDefinition: 'For each unrotated outcome, the main filled-area centroid is strictly inside the secondary centroid span in both X and Y. Each margin is the distance to the nearest span endpoint; positive margins prove the placement condition under all quarter-turns' },
    flaggedOutcomes, flaggedExamples, maximumSecondaryAreaRatio,
    secondaryPairsAboveMain, secondaryAboveMainExamples, centroidBracketing,
    passes: outcomes === 262144 && flaggedOutcomes === 0 && secondaryPairsAboveMain === 0
      && centroidBracketing.failures === 0 && positiveControl.flagged && positivePlacementControl.bothAboveMain,
    positiveControl: { revision: '20e669a', address: ROUND2_ORANGE_ADDRESS, algorithmVersion: 2,
      flaggedLayouts: Number(positiveControl.flagged), secondaryPairsAboveMain: Number(positivePlacementControl.bothAboveMain),
      matches: positiveControl.matches },
    sampledPathCacheEntries: cache.size,
    provenance: { scriptSha256: createHash('sha256').update(readFileSync(new URL(import.meta.url))).digest('hex'),
      canonicalSourceSha256: createHash('sha256').update(source).digest('hex') },
    environment: { host: hostname(), platform: process.platform, arch: process.arch, node: process.version },
    caveat: 'These numeric regression checks cover the specified two-small-elements-over-larger-form pattern and the stronger two-secondary-islands-above-main pattern. Human pareidolia and recognition remain unmeasured; other face-like perceptions can exist outside these checks.',
    elapsedSeconds: (performance.now() - started) / 1000,
  };
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  if (process.argv.length !== 4 || process.argv[2] !== '--exhaustive') throw new Error('usage: account-icon-layout.mjs --exhaustive OUTPUT_JSON (guarded poc-m3 only)');
  const report = exhaustiveLayoutReport();
  writeFileSync(process.argv[3], JSON.stringify(report, null, 2) + '\n');
  console.log(`Face-like layouts: ${report.flaggedOutcomes}/${report.coverage.outcomes}; both secondary islands above main: ${report.secondaryPairsAboveMain}; round 2 positive controls: ${report.positiveControl.flaggedLayouts}/${report.positiveControl.secondaryPairsAboveMain}; maximum secondary area ratio: ${report.maximumSecondaryAreaRatio.toFixed(6)}`);
  if (!report.passes) process.exitCode = 1;
}
