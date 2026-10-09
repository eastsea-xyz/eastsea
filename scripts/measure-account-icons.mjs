#!/usr/bin/env node
// Rendered scale measurements belong on guarded poc-m3, not the development Mac.
import { createHash } from 'node:crypto';
import { readFileSync, writeFileSync, mkdirSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { hostname } from 'node:os';
import { performance } from 'node:perf_hooks';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  ACCOUNT_ICON_VERSION, ACCOUNT_ICON_PALETTES, ACCOUNT_ICON_SILHOUETTES,
  deriveAccountIcon, accountIconSilhouette,
} from '../apps/extension/src/lib/accountIcon.js';
import { contrast, deltaE2000, interpolateColor, srgbToLab } from './account-icon-color.mjs';

const pairCount = Number(process.argv[2] || 10_000);
const output = process.argv[3];
const rasterBinary = process.argv[4];
if (!Number.isSafeInteger(pairCount) || pairCount < 1 || pairCount > 100_000) throw new Error('pair count must be 1..100000');
if (!output || !rasterBinary) throw new Error('usage: measure-account-icons.mjs PAIRS OUTPUT_JSON ICON_ONLY_RASTER_BINARY');
const started = performance.now();
const domain = 'eastsea-account-icon-glance-v2';
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const scratch = path.join(root, 'tmp/account-icon-glance');
mkdirSync(scratch, { recursive: true });
// Conservatively group close outlines even though their geometry is distinct.
const silhouetteGroups = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 6, 0, 3, 10, 6, 11];
const unique = new Map();
const reviewedAddresses = ['0x1234567890abcdef1234567890abcdef12345678', '0xa2521982a17474cb2f8741c85de653b5282d72b0'];
const key = (spec) => `${spec.palette}:${accountIconSilhouette(spec)}:${spec.rotation}`;
const pairs = [];
for (let index = 0; index < pairCount * 2; index++) {
  const counter = Buffer.alloc(4);
  counter.writeUInt32BE(index);
  const address = '0x' + createHash('sha256').update(domain).update(counter).digest().subarray(0, 20).toString('hex');
  const spec = deriveAccountIcon(address);
  if (!spec) throw new Error('valid sampled address rejected');
  const signature = key(spec);
  if (!unique.has(signature)) unique.set(signature, address);
  pairs.push({ address, signature, silhouette: accountIconSilhouette(spec), palette: spec.palette, rotation: spec.rotation });
}
// The two lead-review examples are always available, including tiny smoke runs.
for (const address of reviewedAddresses) {
  const signature = key(deriveAccountIcon(address));
  if (!unique.has(signature)) unique.set(signature, address);
}
const input = path.join(scratch, 'addresses.json'), rasterOutput = path.join(scratch, 'rasters.json');
writeFileSync(input, JSON.stringify([...unique.values()]));
const raster = spawnSync(rasterBinary, ['--dominants', input, rasterOutput], {
  stdio: 'inherit', env: { ...process.env, TMPDIR: scratch }, timeout: 120_000,
});
if (raster.error || raster.status !== 0) throw new Error(`icon-only raster failed: ${raster.error || raster.status}`);
const rasters = JSON.parse(readFileSync(rasterOutput, 'utf8'));
if (rasters.length !== unique.size) throw new Error('missing unique 16px raster');

// Two-cluster dominant color from the actual opaque raster interior. The
// initial centroids are the known sea and land tones, then both are updated
// from pixel samples; the more populous cluster is the dominant color. Corner
// alpha is excluded so appearance/surface color cannot influence identity.
function dominant(rgba, palette) {
  if (rgba.length !== 16 * 16 * 4) throw new Error('expected a 16x16 RGBA raster');
  const pixels = [];
  for (let offset = 0; offset < rgba.length; offset += 4) {
    const alpha = rgba[offset + 3] / 255;
    if (alpha < 0.95) continue;
    pixels.push(rgba.slice(offset, offset + 3).map((channel) => Math.min(1, channel / 255 / alpha)));
  }
  if (!pixels.length) throw new Error('empty raster interior');
  const toRGB = (hex) => [1, 3, 5].map((offset) => parseInt(hex.slice(offset, offset + 2), 16) / 255);
  let centers = [toRGB(interpolateColor(palette.start, palette.end)), toRGB(palette.ink)];
  let counts;
  for (let iteration = 0; iteration < 12; iteration++) {
    counts = [0, 0];
    const sums = [[0, 0, 0], [0, 0, 0]];
    for (const pixel of pixels) {
      const distances = centers.map((center) => pixel.reduce((sum, channel, index) => sum + (channel - center[index]) ** 2, 0));
      const cluster = distances[0] <= distances[1] ? 0 : 1;
      counts[cluster]++;
      pixel.forEach((channel, index) => { sums[cluster][index] += channel; });
    }
    centers = centers.map((center, cluster) => counts[cluster] ? sums[cluster].map((sum) => sum / counts[cluster]) : center);
  }
  const index = counts[0] >= counts[1] ? 0 : 1;
  return { rgb: centers[index], lab: srgbToLab(centers[index]), share: counts[index] / pixels.length, cluster: index };
}
const colors = new Map();
for (const raster of rasters) {
  const spec = deriveAccountIcon(raster.address), signature = key(spec);
  if (!unique.has(signature) || colors.has(signature)) throw new Error('unexpected or duplicate raster');
  colors.set(signature, dominant(raster.rgba, ACCOUNT_ICON_PALETTES[spec.palette]));
}
let matchingPairs = 0, nearColorPairs = 0, sameClassPairs = 0, exact16Pairs = 0;
let ignoreRotationPairs = 0, conservativePairs = 0;
const examples = [];
for (let index = 0; index < pairs.length; index += 2) {
  const first = pairs[index], second = pairs[index + 1];
  const distance = deltaE2000(colors.get(first.signature).lab, colors.get(second.signature).lab);
  const near = distance < 15, same = first.silhouette === second.silhouette;
  if (near) nearColorPairs++;
  if (same) sameClassPairs++;
  if (near && silhouetteGroups[first.silhouette] === silhouetteGroups[second.silhouette]) conservativePairs++;
  if (near && same) {
    matchingPairs++;
    if (first.rotation !== second.rotation) ignoreRotationPairs++;
    if (examples.length < 5) examples.push({ first: first.address, second: second.address, deltaE: distance, silhouetteClass: first.silhouette });
  }
  if (first.signature === second.signature) exact16Pairs++;
}
const backgrounds = { light: '#f4efe6', dark: '#071320' };
const palettes = ACCOUNT_ICON_PALETTES.map((palette) => ({ ...palette,
  midpoint: interpolateColor(palette.start, palette.end),
  contrast: Object.fromEntries(['start', 'end'].map((stop) => [stop, {
    ink: contrast(palette[stop], palette.ink), light: contrast(palette[stop], backgrounds.light), dark: contrast(palette[stop], backgrounds.dark),
  }])),
}));
const paletteDistances = [];
for (let a = 0; a < palettes.length; a++) for (let b = a + 1; b < palettes.length; b++) {
  paletteDistances.push({ a, b, deltaE: deltaE2000(srgbToLab(palettes[a].midpoint), srgbToLab(palettes[b].midpoint)) });
}
paletteDistances.sort((a, b) => a.deltaE - b.deltaE);
const fraction = matchingPairs / pairCount;
const report = {
  version: ACCOUNT_ICON_VERSION, pairCount, addressCount: pairs.length,
  stream: `SHA-256(UTF8("${domain}") || UInt32BE(i))[0..20]; consecutive independent pairs; i=0..${pairs.length - 1}`,
  raster: { renderer: 'SwiftUI ImageRenderer, 16x16 pixels, sRGB, alpha uncomposited', uniqueRenders: colors.size,
    signature: 'palette + unrotated silhouette class + quarter-turn; remaining detail is absent below 32 px',
    dominantColor: '12 iterations of two-cluster sRGB k-means on alpha>=0.95 interior; premultiplied channels unpremultiplied; largest cluster centroid converted to D65 Lab',
    minimumDominantShare: Math.min(...[...colors.values()].map((color) => color.share)),
    foregroundDominatesCount: [...colors.values()].filter((color) => color.cluster === 1).length,
  },
  glance: { definition: 'CIEDE2000 between rendered dominant colors <15 AND same unrotated coastline silhouette class. Rotation deliberately ignored.',
    thresholdDeltaE: 15, silhouetteClassCount: ACCOUNT_ICON_SILHOUETTES.length,
    matchingPairs, totalPairs: pairCount, fraction, percentage: fraction * 100, targetFraction: 0.01, passes: fraction < 0.01,
    nearColorPairs, sameClassPairs, matchingPairsWithDifferentRotations: ignoreRotationPairs, exact16Pairs,
    conservativeGrouping: { definition: 'Merge cove/crescent, inlet/hook/arch, and twinpeaks/ridge; ignore rotation.', silhouetteGroups, groupCount: new Set(silhouetteGroups).size, matchingPairs: conservativePairs, percentage: conservativePairs / pairCount * 100, passes: conservativePairs / pairCount < 0.01 },
    wilson95Percent: (() => { const z = 1.9599639845, n = pairCount, denominator = 1 + z * z / n;
      const center = (fraction + z * z / (2 * n)) / denominator;
      const half = z * Math.sqrt(fraction * (1 - fraction) / n + z * z / (4 * n * n)) / denominator;
      return [(center - half) * 100, (center + half) * 100]; })(), examples },
  palette: { colorSpace: 'CIEDE2000, D65 Lab, kL=kC=kH=1, encoded sRGB background midpoint',
    minimumDeltaE: paletteDistances[0].deltaE, closestPair: paletteDistances[0], distances: paletteDistances,
    allContrastPairsPass: palettes.every((palette) => Object.values(palette.contrast).every((stop) => Math.min(stop.ink, stop.light, stop.dark) >= 3)),
    minimumInkContrast: Math.min(...palettes.flatMap((palette) => Object.values(palette.contrast).map((stop) => stop.ink))),
    palettes, backgrounds },
  reviewedPair: (() => {
    const addresses = reviewedAddresses;
    const specs = addresses.map((address) => deriveAccountIcon(address));
    return { addresses, features: specs, palettes: specs.map((spec) => ACCOUNT_ICON_PALETTES[spec.palette].name), silhouetteClasses: specs.map(accountIconSilhouette), dominantDeltaE: deltaE2000(...specs.map((spec) => colors.get(key(spec)).lab)) };
  })(),
  provenance: { scriptSha256: createHash('sha256').update(readFileSync(new URL(import.meta.url))).digest('hex'), canonicalSourceSha256: createHash('sha256').update(readFileSync(new URL('../apps/extension/src/lib/accountIcon.js', import.meta.url))).digest('hex'), rasterBinarySha256: createHash('sha256').update(readFileSync(rasterBinary)).digest('hex') },
  environment: { host: hostname(), platform: process.platform, arch: process.arch, node: process.version },
  caveat: 'This requested dominant-color/silhouette proxy is not a human recognition experiment or proof against attacker address grinding. Distinct coastline classes may still look similar to a person.',
  elapsedSeconds: (performance.now() - started) / 1000,
};
writeFileSync(output, JSON.stringify(report, null, 2) + '\n');
console.log(`Glance: ${matchingPairs}/${pairCount}=${report.glance.percentage.toFixed(2)}%; minimum palette ΔE=${report.palette.minimumDeltaE.toFixed(4)}`);
if (!report.glance.passes || !report.palette.allContrastPairsPass) process.exitCode = 1;
