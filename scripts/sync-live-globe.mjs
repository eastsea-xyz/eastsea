#!/usr/bin/env node
// Both deployments are independent static roots. Commit the small local copy
// so neither needs a build step; check drift in the ordinary explorer tests.
import { copyFile, mkdir, readFile, readdir } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { join } from 'node:path';

const root = fileURLToPath(new URL('../', import.meta.url));
const source = join(root, 'apps/explorer/live-globe');
const destination = join(root, 'site/live-globe');
const check = process.argv.includes('--check');
const brandTokens = await readFile(join(root, 'site/tokens.css'));
if (check) {
  try {
    if (!brandTokens.equals(await readFile(join(source, 'tokens.css')))) throw new Error('drift');
  } catch {
    console.error('Live globe brand tokens differ. Run node scripts/sync-live-globe.mjs.');
    process.exit(1);
  }
} else {
  const { writeFile } = await import('node:fs/promises');
  await writeFile(join(source, 'tokens.css'), brandTokens);
}
if (!check) await mkdir(destination, { recursive: true });
const names = (await readdir(source)).filter(name => /\.(?:js|css|json|md)$/.test(name)).sort();
let failed = false;
for (const name of names) {
  if (check) {
    try {
      const [a, b] = await Promise.all([readFile(join(source, name)), readFile(join(destination, name))]);
      if (!a.equals(b)) throw new Error('drift');
    } catch {
      failed = true;
      console.error(`Live globe copy differs: site/live-globe/${name}`);
    }
  } else {
    await copyFile(join(source, name), join(destination, name));
  }
}
if (failed) process.exitCode = 1;
else console.log(`${check ? 'Checked' : 'Copied'} ${names.length} shared globe assets.`);
