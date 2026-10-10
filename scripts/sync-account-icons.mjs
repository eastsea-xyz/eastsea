#!/usr/bin/env node
// Keep independently served static apps on the frozen canonical implementation.
import { readFile, writeFile } from 'node:fs/promises';

const source = new URL('../apps/extension/src/lib/accountIcon.js', import.meta.url);
const mirrors = [
  new URL('../apps/explorer/js/accountIcon.js', import.meta.url),
  new URL('../site/account-icon.js', import.meta.url),
];
const check = process.argv.includes('--check');
const canonical = await readFile(source);
for (const mirror of mirrors) {
  if (check) {
    const contents = await readFile(mirror).catch(() => null);
    if (!contents?.equals(canonical)) {
      console.error(`${mirror.pathname} differs from ${source.pathname}; run node scripts/sync-account-icons.mjs`);
      process.exitCode = 1;
    }
  } else await writeFile(mirror, canonical);
}
if (!process.exitCode) console.log(check ? 'Account icon mirrors match.' : 'Account icon mirrors synchronized.');
