#!/usr/bin/env node
// Per-contract markdown table for docs/research/contracts-live-*.md, from a
// contracts-live run (tmp/live/results.json) next to the in-process numbers
// (docs/research/contracts-onchain-2026-10-06.md, the CONTRACTS-ONCHAIN table).
//
//   node scripts/contracts-live/report-table.mjs [results.json] [in-process.md]
//
// Columns: live deploy gas / state units vs in-process, steps ok/total for the
// contract's flow (deploy + calls + expected reverts), the decoded revert
// reasons the receipts surfaced, eth_getLogs count, and the wallet's wait on
// a spent B5 budget for the deploy.

import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const HERE = dirname(fileURLToPath(import.meta.url));
const RESULTS = process.argv[2] || resolve(HERE, '../../tmp/live/results.json');
const INPROC = process.argv[3] || resolve(HERE, '../../docs/research/contracts-onchain-2026-10-06.md');
const ART = resolve(HERE, '../../crates/contracts-onchain/fixtures/artifacts.json');

const r = JSON.parse(readFileSync(RESULTS, 'utf8'));
const names = Object.keys(JSON.parse(readFileSync(ART, 'utf8')));

// In-process: | `name` | cases | result | gas | units | bytes | fee | fits |
const inproc = {};
for (const m of readFileSync(INPROC, 'utf8').matchAll(/^\| `([^`]+)` \|[^|]*\|[^|]*\| ([\d–]+) \| ([\d–]+) \|/gm)) {
  inproc[m[1]] = { gas: m[2], units: m[3] };
}

// Which flow labels belong to which contract (labels are the flows' own).
const ALIAS = {
  'toolbox/AllOrNothingCrowdfund': ['Crowdfund'],
  'toolbox/BondingLaunchpad': ['Launchpad'],
  'toolbox/CommitRevealRaffle': ['Raffle'],
  'toolbox/Editions1155': ['Editions'],
  'toolbox/FixedPriceMarket': ['Market ', 'Market.'],
  'toolbox/MilestoneEscrow': ['Escrow'],
  'toolbox/RewardDistributor': ['Rewards'],
  'toolbox/SubscriptionManager': ['Subscription'],
  'core/EastSeaVaultFactory': ['VaultFactory'],
  'core/EastSeaVault': ['EastSeaVault', 'aether send (21,000 gas transfer path) to EastSeaVault', 'aether send (wallet transfer path) to EastSeaVault'],
  'core/EastSeaAccount': ['EastSeaAccount', 'fund dev4', 'aether send (21,000 gas) to the 7702', 'aether send (wallet transfer path) to the 7702'],
  'core/EastSeaNames': ['core/EastSeaNames', 'EastSeaNames.'],
  'toolbox/EastSeaNames': ['toolbox/EastSeaNames'],
  'support/TestToken': ['support/TestToken', 'TestToken.'],
  'core/Randomness': ['core/Randomness'],
  'toolbox/Randomness': ['toolbox/Randomness'],
  'toolbox/AmmPair': ['toolbox/AmmPair', 'AmmPair.'],
  'toolbox/AmmFactory': ['toolbox/AmmFactory', 'AmmFactory.'],
  'toolbox/AmmRouter': ['toolbox/AmmRouter', 'AmmRouter.'],
  'core/AtomicSwap': ['core/AtomicSwap', 'AtomicSwap.'],
  'core/AtomicSwapEVM': ['core/AtomicSwapEVM', 'AtomicSwapEVM.'],
  'core/CommitteeRegistry': ['core/CommitteeRegistry.', 'core/CommitteeRegistry'],
  'core/MerkleDistributor': ['core/MerkleDistributor', 'MerkleDistributor '],
  'core/MerkleDistributorFactory': ['core/MerkleDistributorFactory', 'MerkleDistributorFactory'],
};
const owns = (name, label) => {
  const short = name.split('/')[1];
  const keys = [name, `${short}.`, `${short} `, ...(ALIAS[name] || [])];
  if (name === 'core/CommitteeRegistry' && label.startsWith('core/CommitteeRegistryV3')) return false;
  if (name === 'core/AtomicSwap' && label.includes('AtomicSwapEVM')) return false;
  if (name === 'core/MerkleDistributor' && label.includes('MerkleDistributorFactory')) return false;
  if (name === 'core/EastSeaVault' && (label.startsWith('VaultFactory') || label.startsWith('core/EastSeaVaultFactory'))) return false;
  if (name === 'core/EastSeaNames' && label.startsWith('toolbox/')) return false;
  if (name === 'toolbox/EastSeaNames' && !label.startsWith('toolbox/')) return false;
  if (name === 'toolbox/AmmPair' && label.startsWith('toolbox/AmmPair') === false && label.startsWith('AmmPair.') === false) return false;
  return keys.some((k) => label.startsWith(k));
};

const steps = r.steps || [];
const rows = [];
let totOk = 0;
let totAll = 0;
for (const name of names) {
  const mine = steps.filter((s) => s.label && owns(name, s.label));
  const dep = mine.find((s) => s.kind === 'deploy');
  const ok = mine.filter((s) => s.ok === true).length;
  totOk += ok;
  totAll += mine.length;
  const reverts = mine.filter((s) => s.kind === 'revert');
  const reasons = [...new Set(reverts.map((s) => s.revert || (s.ok ? '(no data)' : 'NOT REVERTED')))].join('; ');
  const ip = inproc[name] || {};
  const lat = mine.filter((s) => s.submittedMs != null && s.finalizedMs != null).map((s) => s.finalizedMs - s.submittedMs);
  const med = lat.length ? lat.sort((a, b) => a - b)[Math.floor(lat.length / 2)] : null;
  rows.push([
    `\`${name}\``,
    mine.length ? (ok === mine.length ? `PASS ${ok}/${mine.length}` : `**FAIL ${ok}/${mine.length}**`) : 'not run',
    dep?.height ?? '–',
    dep?.gas ?? '–',
    ip.gas ?? '–',
    dep?.stateGas ?? '–',
    ip.units ?? '–',
    dep?.budgetWaitMs ? `${Math.round(dep.budgetWaitMs / 1000)} s` : '0',
    med != null ? `${med} ms` : '–',
    `${mine.reduce((n, s) => n + (Number(s.logs) || 0), 0)} / ${typeof r.logCounts?.[name] === 'number' ? r.logCounts[name] : '–'}`,
    reasons || '–',
  ]);
}
console.log('| Contract | Live result (steps) | Deploy height | Deploy exec gas (live) | (in-process) | State units (live) | (in-process) | B5 wait before deploy | Median submit→final | Logs: receipts / eth_getLogs | Expected revert(s), as decoded from the receipt |');
console.log('|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---|');
for (const row of rows) console.log(`| ${row.join(' | ')} |`);
const unowned = steps.filter((s) => s.label && !names.some((n) => owns(n, s.label)));
console.log(`\nSteps: ${totOk}/${totAll} ok across the 41 fixtures; ${unowned.length} other steps (${unowned.map((s) => s.label).join(', ')}).`);
const multi = steps.filter((s) => s.label && names.filter((n) => owns(n, s.label)).length > 1);
if (multi.length) console.log(`Shared by two contracts' rows: ${multi.map((s) => `${s.label} (${names.filter((n) => owns(n, s.label)).join(' + ')})`).join('; ')}.`);
