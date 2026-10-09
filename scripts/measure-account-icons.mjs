#!/usr/bin/env node
// Run scale measurements on guarded poc-m3, not the development Mac.
import { createHash } from 'node:crypto';
import { writeFileSync } from 'node:fs';
import { hostname } from 'node:os';
import { performance } from 'node:perf_hooks';
import { deriveAccountIcon, ACCOUNT_ICON_PALETTES, ACCOUNT_ICON_INK } from '../apps/extension/src/lib/accountIcon.js';

const count = Number(process.argv[2] || 100_000);
if (!Number.isSafeInteger(count) || count < 2 || count > 1_000_000) throw new Error('sample count must be 2..1000000');
const output = process.argv[3];
const domain = 'eastsea-account-icon-measure-v1';
const started = performance.now();
const counters = Object.fromEntries(['tuple', 'coarse', 'paletteAndMask', 'grayscaleMask'].map((name) => [name, { groups: new Map(), pairs: 0 }]));

function record(counter, key) {
  const previous = counter.groups.get(key) || 0;
  counter.pairs += previous;
  counter.groups.set(key, previous + 1);
}

function rotatedMask(spec) {
  let mask = 0;
  for (let cell = 0; cell < 15; cell++) {
    if (cell > 0 && !((spec.layout >> (cell - 1)) & 1)) continue;
    let row = Math.floor(cell / 4), col = cell % 4;
    for (let turn = 0; turn < spec.rotation; turn++) [row, col] = [col, 3 - row];
    mask |= 1 << (4 * row + col);
  }
  return mask;
}

for (let index = 0; index < count; index++) {
  const counter = Buffer.alloc(4);
  counter.writeUInt32BE(index);
  // Reproducible pseudorandom addresses, independent of the icon's hash domain.
  const bytes = createHash('sha256').update(domain).update(counter).digest().subarray(0, 20);
  const spec = deriveAccountIcon('0x' + bytes.toString('hex'));
  if (!spec) throw new Error('valid sampled address rejected');
  const mask = rotatedMask(spec);
  // Squares/discs have no orientation. Triangles/quarter-discs rotate visibly.
  const orientation = spec.shape < 2 ? 0 : spec.rotation;
  record(counters.tuple, `${spec.palette}:${spec.layout}:${spec.shape}:${spec.rotation}`);
  record(counters.coarse, `${spec.palette}:${mask}:${spec.shape}:${orientation}`);
  record(counters.paletteAndMask, `${spec.palette}:${mask}`);
  record(counters.grayscaleMask, mask);
}

function luminance(hex) {
  const linear = [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16) / 255)
    .map((v) => v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4);
  return linear.reduce((sum, v, i) => sum + v * [0.2126, 0.7152, 0.0722][i], 0);
}
function contrast(a, b) {
  const [low, high] = [luminance(a), luminance(b)].sort((x, y) => x - y);
  return (high + 0.05) / (low + 0.05);
}

const totalPairs = count * (count - 1) / 2;
const metrics = Object.fromEntries(Object.entries(counters).map(([name, { groups, pairs }]) => [name, {
  distinctSignatures: groups.size, matchingPairs: pairs, fraction: pairs / totalPairs,
  oneIn: pairs ? totalPairs / pairs : null,
}]));
const backgrounds = { light: '#f7f5f0', dark: '#101820' };
const palettes = ACCOUNT_ICON_PALETTES.map((fill) => ({ fill, ink: ACCOUNT_ICON_INK,
  light: contrast(fill, backgrounds.light), dark: contrast(fill, backgrounds.dark), inkContrast: contrast(fill, ACCOUNT_ICON_INK) }));
const report = {
  version: 1, sampleCount: count, totalUnorderedPairs: totalPairs,
  measurementStream: `SHA-256(UTF8("${domain}") || UInt32BE(i))[0..20], i=0..${count - 1}`,
  metricDefinitions: {
    tuple: 'palette, 14-bit layout, glyph style, raw quarter-turn (can overcount distinct images)',
    coarse: 'palette, actually rotated 4x4 occupied mask, glyph style, visible glyph orientation',
    paletteAndMask: 'palette and actually rotated mask; discard glyph style/orientation',
    grayscaleMask: 'actually rotated mask only; discard hue, glyph style and orientation',
  },
  metrics, targetFraction: 0.0001, passes: metrics.coarse.fraction < 0.0001,
  palettes, backgrounds, allContrastPairsPass: palettes.every((p) => Math.min(p.light, p.dark, p.inkContrast) >= 3),
  caveat: 'Exact coarse-feature equality is not a measured human/perceptual confusion rate. Rasterization can introduce further aliases.',
  environment: { host: hostname(), platform: process.platform, arch: process.arch, node: process.version },
  elapsedSeconds: (performance.now() - started) / 1000,
};
if (output) {
  writeFileSync(output, JSON.stringify(report, null, 2) + '\n');
  console.log(`Account icon sample: ${count}; coarse matching pairs ${metrics.coarse.matchingPairs}/${totalPairs}; fraction ${metrics.coarse.fraction}`);
} else console.log(JSON.stringify(report, null, 2));
if (!report.passes || !report.allContrastPairsPass) process.exitCode = 1;
