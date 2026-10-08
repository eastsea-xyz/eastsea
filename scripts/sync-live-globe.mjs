#!/usr/bin/env node
// The web deployments and native wallet bundle are independent static roots.
// Commit the local copies so none needs a build step; ordinary tests check drift.
import { copyFile, mkdir, readFile, readdir } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { join } from 'node:path';

const root = fileURLToPath(new URL('../', import.meta.url));
const source = join(root, 'apps/explorer/live-globe');
const destinations = ['site/live-globe', 'apps/wallet/Resources/LiveGlobe/live-globe'];
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
const names = (await readdir(source)).filter(name => /\.(?:js|css|json|md)$/.test(name)).sort();
let failed = false;
for (const relative of destinations) {
  const destination = join(root, relative);
  if (!check) await mkdir(destination, { recursive: true });
  for (const name of names) {
    if (check) {
      try {
        const [a, b] = await Promise.all([readFile(join(source, name)), readFile(join(destination, name))]);
        if (!a.equals(b)) throw new Error('drift');
      } catch {
        failed = true;
        console.error(`Live globe copy differs: ${relative}/${name}`);
      }
    } else {
      await copyFile(join(source, name), join(destination, name));
    }
  }
}
// Screenshot mode reads this explicit fixture locally; the live host never does.
const fixtureSource = join(root, 'apps/explorer/test/fixtures/presence-example.json');
const fixtureDestination = join(root, 'apps/wallet/Resources/LiveGlobe/presence-example.json');
if (check) {
  try {
    const [a, b] = await Promise.all([readFile(fixtureSource), readFile(fixtureDestination)]);
    if (!a.equals(b)) throw new Error('drift');
  } catch {
    failed = true;
    console.error('Wallet globe screenshot fixture differs. Run node scripts/sync-live-globe.mjs.');
  }
} else await copyFile(fixtureSource, fixtureDestination);
if (failed) process.exitCode = 1;
else console.log(`${check ? 'Checked' : 'Copied'} ${names.length} shared globe assets on ${destinations.length + 1} surfaces.`);
