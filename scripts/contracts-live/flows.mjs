// Per-contract live flows for scripts/contracts-live/deploy-flows.mjs.
//
// Every fixture in crates/contracts-onchain/fixtures gets: a wallet-path
// deploy (constructor args exactly as the in-process fixtures build them),
// 2–5 wallet-path calls of its main user flow, and one call expected to
// revert on-chain. State changes go exclusively through the aether CLI
// (`send`/`deploy`/`call`) — the same `sign_call_with` +
// `recommended_state_budget` path the wallet app and extension use. Views run
// over the node's eth_call. In-contract signatures (vault spend, secp256k1
// multisig/DAO/permit) are computed locally over digests the contracts
// publish.

import { keccak256 } from '@noble/hashes/sha3';
import { sha256 } from '@noble/hashes/sha256';
import { Wallet, getCreateAddress } from 'ethers';
import {
  call, deploy, send, ethCall, iface, coder, devAddress, devKeyXY, devSignDigest,
  nonceOf, sleep, nowSec, CHAIN_ID,
} from './lib.mjs';

const E = coder();
const u8 = (b) => '0x' + Buffer.from(b).toString('hex');
const pad32 = (n) => Buffer.from(n.toString(16).padStart(64, '0'), 'hex');
const addr0 = '0x0000000000000000000000000000000000000000';
const EIP191 = (hash) => Buffer.concat([Buffer.from('\x19Ethereum Signed Message:\n32'), hash]);

/// secp256k1 identities for the ecrecover contracts (multisig, DAO, permit).
function secp(seed) {
  return new Wallet('0x' + Buffer.from(keccak256(Buffer.from(`eastsea-live-${seed}`))).toString('hex'));
}

/// Sorted-pair Merkle tree over packed leaves (core/MerkleDistributor).
function merkleRoot(leaves) {
  let level = leaves.map((l) => Buffer.from(keccak256(l)));
  while (level.length > 1) {
    if (level.length % 2) level.push(level[level.length - 1]);
    level = Array.from({ length: level.length / 2 }, (_, i) =>
      Buffer.from(keccak256(Buffer.concat(level[i] <= level[i + 1] ? [level[i], level[i + 1]] : [level[i + 1], level[i]]))));
  }
  return level[0];
}
function merkleProof(leaves, index) {
  let idx = index;
  let level = leaves.map((l) => Buffer.from(keccak256(l)));
  const proof = [];
  while (level.length > 1) {
    if (level.length % 2) level.push(level[level.length - 1]);
    proof.push(Buffer.from(level[idx ^ 1]));
    level = Array.from({ length: level.length / 2 }, (_, i) =>
      Buffer.from(keccak256(Buffer.concat(level[i] <= level[i + 1] ? [level[i], level[i + 1]] : [level[i + 1], level[i]]))));
    idx >>= 1;
  }
  return proof.map((p) => u8(p));
}

export async function runFlows(ctx) {
  const { art, log, results } = ctx;
  const D = { 1: devAddress(1), 2: devAddress(2), 3: devAddress(3) };
  const deployed = {};
  ctx.addresses.dev = D;

  const I = (name) => iface(art[name].abi);
  const enc = (name, fn, ...args) => I(name).encodeFunctionData(fn, args);
  const ctor = (name, args) => art[name].bytecode + I(name).encodeDeploy(args).slice(10);
  const view = async (name, fn, args = []) => ethCall(deployed[name], I(name).encodeFunctionData(fn, args), { from: D[1] });

  const step = async (kind, label, fn) => {
    try {
      const r = await fn();
      results.push({ kind, label, ...(r || {}) });
      return r;
    } catch (e) {
      results.push({ kind, label, ok: false, error: String((e && e.message) || e) });
      log(`  ERROR ${label}: ${(e && e.message) || e}`);
      return null;
    }
  };
  const must = (label, r) => {
    if (!r || r.ok !== true || r.success !== true) {
      throw new Error(`${label}: unexpected receipt ${JSON.stringify({ ok: r && r.ok, success: r && r.success, stderr: r && r.stderr })}`);
    }
    return r;
  };
  /// The one deliberate on-chain revert per contract: included, success=false.
  const expectRevert = (label, p) =>
    step('revert', label, async () => {
      const r = await p;
      const good = !!r && r.ok === true && r.success === false && r.height != null;
      return { ...r, ok: good, expected: 'revert' };
    });

  // ------------------------------------------------------------------ support
  await step('deploy', 'support/TestToken(feeBps=0)', async () => {
    const r = must('TestToken', await deploy({ dev: 1, code: ctor('support/TestToken', [0]) }));
    deployed['support/TestToken'] = r.contractAddress;
    deployed.TT2 = null;
    return r;
  });
  await step('deploy', 'support/MarketNFT', async () => {
    const r = must('MarketNFT', await deploy({ dev: 1, code: art['support/MarketNFT'].bytecode }));
    deployed['support/MarketNFT'] = r.contractAddress;
    return r;
  });
  await step('deploy', 'support/TestToken#2 (reward token)', async () => {
    const r = must('TT2', await deploy({ dev: 1, code: ctor('support/TestToken', [0]) }));
    deployed.TT2 = r.contractAddress;
    return r;
  });
  await step('call', 'TestToken.mint dev2/dev3', async () => {
    must('mint2', await call({ dev: 1, to: deployed['support/TestToken'], data: enc('support/TestToken', 'mint', D[2], 1_000_000n * 10n ** 18n) }));
    return must('mint3', await call({ dev: 1, to: deployed['support/TestToken'], data: enc('support/TestToken', 'mint', D[3], 1_000_000n * 10n ** 18n) }));
  });
  await expectRevert('support/TestToken.transfer over balance', call({ dev: 2, to: deployed['support/TestToken'], data: enc('support/TestToken', 'transfer', D[3], 1_000_001n * 10n ** 18n) }));
  await step('deploy', 'support/NativeCallback (bonus)', async () => {
    const r = must('NativeCallback', await deploy({ dev: 1, code: art['support/NativeCallback'].bytecode }));
    deployed['support/NativeCallback'] = r.contractAddress;
    return r;
  });
  await step('call', 'NativeCallback.execute native roundtrip', async () =>
    must('execute', await call({ dev: 1, to: deployed['support/NativeCallback'], data: enc('support/NativeCallback', 'execute', D[3], '0x', 10n ** 15n), value: 10n ** 15n })));

  // ------------------------------------------------------------------ core
  await step('deploy', 'core/AtomicSwap', async () => {
    const r = must('deploy', await deploy({ dev: 1, code: art['core/AtomicSwap'].bytecode }));
    deployed['core/AtomicSwap'] = r.contractAddress;
    return r;
  });
  const secret = Buffer.from('secret'); // sha256 matches the fixtures' hashlock
  const hash = { sha: u8(sha256(secret)), kec: u8(keccak256(secret)) };
  const lock = (name, hl) => enc(name, 'lock', D[2], hl, BigInt(nowSec() + 3600), addr0, 10n ** 15n);
  await step('call', 'AtomicSwap.lock native', async () =>
    must('lock', await call({ dev: 1, to: deployed['core/AtomicSwap'], data: lock('core/AtomicSwap', hash.sha), value: 10n ** 15n })));
  await expectRevert('AtomicSwap.claim wrong preimage', call({ dev: 2, to: deployed['core/AtomicSwap'], data: enc('core/AtomicSwap', 'claim', 0n, u8(Buffer.from('wrong'))) }));
  await step('call', 'AtomicSwap.claim correct preimage', async () =>
    must('claim', await call({ dev: 2, to: deployed['core/AtomicSwap'], data: enc('core/AtomicSwap', 'claim', 0n, u8(secret)) })));

  await step('deploy', 'core/AtomicSwapEVM', async () => {
    const r = must('deploy', await deploy({ dev: 1, code: art['core/AtomicSwapEVM'].bytecode }));
    deployed['core/AtomicSwapEVM'] = r.contractAddress;
    return r;
  });
  await step('call', 'AtomicSwapEVM.lock native', async () =>
    must('lock', await call({ dev: 1, to: deployed['core/AtomicSwapEVM'], data: lock('core/AtomicSwapEVM', hash.kec), value: 10n ** 15n })));
  await step('call', 'AtomicSwapEVM.claim', async () =>
    must('claim', await call({ dev: 2, to: deployed['core/AtomicSwapEVM'], data: enc('core/AtomicSwapEVM', 'claim', 0n, u8(secret)) })));
  await expectRevert('AtomicSwapEVM.claim twice', call({ dev: 1, to: deployed['core/AtomicSwapEVM'], data: enc('core/AtomicSwapEVM', 'claim', 0n, u8(secret)) }));

  // A user-deployed registry has empty genesis registrar slots, so every
  // register reverts BadAttestation by design. The real flow (predeploy with
  // genesis slots) runs via `aether candidate-register` in contracts-live.sh.
  for (const name of ['core/CommitteeRegistry', 'core/CommitteeRegistryV3']) {
    await step('deploy', name, async () => {
      const r = must('deploy', await deploy({ dev: 1, code: art[name].bytecode }));
      deployed[name] = r.contractAddress;
      return r;
    });
    await expectRevert(`${name}.register without genesis registrar`, call({ dev: 1, to: deployed[name], data: enc(name, 'register', '0x' + 'ab'.repeat(32), '0x' + 'cd'.repeat(32), D[2], '0x' + '11'.repeat(32), '0x' + '22'.repeat(32)) }));
    await expectRevert(`${name}.beacon unknown`, call({ dev: 1, to: deployed[name], data: enc(name, 'beacon', '0x' + 'ee'.repeat(32)) }));
  }

  await step('deploy', 'core/EastSeaAccount', async () => {
    // 3.4 M exec gas in-process; the CLI's 3 M default must be raised, the
    // way the wallet app sizes gas per call.
    const r = must('deploy', await deploy({ dev: 1, code: art['core/EastSeaAccount'].bytecode, gas: 6_000_000 }));
    deployed['core/EastSeaAccount'] = r.contractAddress;
    return r;
  });
  await step('call', 'EastSeaAccount fund + execute native transfer', async () => {
    must('fund', await send({ dev: 1, to: deployed['core/EastSeaAccount'], value: 2n * 10n ** 15n }));
    const data = enc('core/EastSeaAccount', 'execute', [[[D[2], 10n ** 15n, '0x']]]); // Call{to,value,data} by position
    return must('execute', await call({ dev: 1, to: deployed['core/EastSeaAccount'], data, value: 10n ** 15n }));
  });
  await expectRevert('EastSeaAccount.execute by non-owner', call({ dev: 3, to: deployed['core/EastSeaAccount'], data: enc('core/EastSeaAccount', 'execute', [[[D[3], 1n, '0x']]]), value: 1n }));

  // Names: both variants commit first, share one 65 s maturity wait, then
  // register + setReverse. The toolbox copy feeds NameGatedDrop later.
  const namesFlow = async (name, label, devIdx) => {
    await step('deploy', name, async () => {
      const r = must('deploy', await deploy({ dev: 1, code: art[name].bytecode }));
      deployed[name] = r.contractAddress;
      return r;
    });
    const owner = D[devIdx];
    const salt = '0x' + Buffer.from(keccak256(Buffer.from(`salt-${label}`))).toString('hex');
    const commitment = u8(keccak256(Buffer.concat([Buffer.from(label), Buffer.from(owner.slice(2), 'hex'), Buffer.from(salt.slice(2), 'hex'), Buffer.alloc(20)])));
    const bond = 10n ** 16n;
    const st = { commitment, salt, owner, bond };
    ctx.names ??= {};
    ctx.names[name] = st;
    await step('call', `${name}.commit bond`, async () =>
      must('commit', await call({ dev: devIdx, to: deployed[name], data: enc(name, 'commit', commitment), value: bond })));
    return st;
  };
  await namesFlow('core/EastSeaNames', 'live', 1);
  await namesFlow('toolbox/EastSeaNames', 'toolive', 2);
  await expectRevert('EastSeaNames.register before MIN_COMMIT_AGE', call({ dev: 1, to: deployed['core/EastSeaNames'], data: enc('core/EastSeaNames', 'register', 'live', ctx.names['core/EastSeaNames'].owner, ctx.names['core/EastSeaNames'].salt, addr0), value: 9n * 10n ** 16n }));
  log('  waiting 65 s for the name commitments to mature');
  await sleep(65_000);
  const register = async (name, label) => {
    const st = ctx.names[name];
    await step('call', `${name}.register "${label}"`, async () =>
      must('register', await call({ dev: label === 'live' ? 1 : 2, to: deployed[name], data: enc(name, 'register', label, st.owner, st.salt, addr0), value: 9n * 10n ** 16n })));
    await step('call', `${name}.setReverse`, async () =>
      must('setReverse', await call({ dev: label === 'live' ? 1 : 2, to: deployed[name], data: enc(name, 'setReverse', label) })));
  };
  await register('core/EastSeaNames', 'live');
  await register('toolbox/EastSeaNames', 'toolive');
  await expectRevert('EastSeaNames.register the same name again', call({ dev: 1, to: deployed['core/EastSeaNames'], data: enc('core/EastSeaNames', 'register', 'live', ctx.names['core/EastSeaNames'].owner, ctx.names['core/EastSeaNames'].salt, addr0), value: 9n * 10n ** 16n }));

  // Vault: owners are P-256 keys; spend signs the contract's own digest.
  await step('deploy', 'core/EastSeaVault', async () => {
    const key = devKeyXY(1);
    const code = ctor('core/EastSeaVault', [[[key.x, key.y]], 1, 2n * 10n ** 15n, 86_400n]);
    const r = must('deploy', await deploy({ dev: 1, code, gas: 6_000_000 }));
    deployed['core/EastSeaVault'] = r.contractAddress;
    return r;
  });
  await step('call', 'EastSeaVault deposit + owner-signed spend', async () => {
    must('deposit', await send({ dev: 1, to: deployed['core/EastSeaVault'], value: 3n * 10n ** 15n }));
    const amount = 10n ** 15n;
    const digest = await view('core/EastSeaVault', 'spendDigest', [D[2], amount, 0n]);
    const sig = devSignDigest(1, digest);
    return must('spend', await call({ dev: 1, to: deployed['core/EastSeaVault'], data: enc('core/EastSeaVault', 'spend', D[2], amount, 0n, sig.r, sig.s) }));
  });
  {
    // Nonce 0 was consumed by the spend above; the same signature must now
    // be refused. spendDigest is a pure view, so it still resolves offline.
    const amount = 10n ** 15n;
    const digest = await view('core/EastSeaVault', 'spendDigest', [D[2], amount, 0n]);
    const sig = devSignDigest(1, digest);
    await expectRevert('EastSeaVault.spend replay (stale nonce)', call({ dev: 1, to: deployed['core/EastSeaVault'], data: enc('core/EastSeaVault', 'spend', D[2], amount, 0n, sig.r, sig.s) }));
  }

  await step('deploy', 'core/EastSeaVaultFactory', async () => {
    const r = must('deploy', await deploy({ dev: 1, code: art['core/EastSeaVaultFactory'].bytecode }));
    deployed['core/EastSeaVaultFactory'] = r.contractAddress;
    return r;
  });
  const fsalt = '0x' + Buffer.from(keccak256(Buffer.from('factory-salt'))).toString('hex');
  const vkey = () => [[devKeyXY(1).x, devKeyXY(1).y], 1, 10n ** 18n, 86_400n];
  await step('view', 'VaultFactory.predict', async () => ({ ok: true, value: await view('core/EastSeaVaultFactory', 'predict', [...vkey(), fsalt]) }));
  await step('call', 'VaultFactory.create', async () =>
    must('create', await call({ dev: 1, to: deployed['core/EastSeaVaultFactory'], data: enc('core/EastSeaVaultFactory', 'create', ...vkey(), fsalt), gas: 6_000_000 })));
  await expectRevert('VaultFactory.create duplicate salt', call({ dev: 1, to: deployed['core/EastSeaVaultFactory'], data: enc('core/EastSeaVaultFactory', 'create', ...vkey(), fsalt), gas: 6_000_000 }));

  // MerkleDistributor: leaf = keccak(index ‖ account ‖ amount), packed.
  await step('deploy', 'core/MerkleDistributor', async () => {
    const leaves = [
      Buffer.concat([pad32(0n), Buffer.from(D[2].slice(2), 'hex'), pad32(10n ** 18n)]),
      Buffer.concat([pad32(1n), Buffer.from(D[3].slice(2), 'hex'), pad32(10n ** 18n)]),
    ];
    ctx.merkle = { leaves };
    const root = u8(merkleRoot(leaves));
    const r = must('deploy', await deploy({ dev: 1, code: ctor('core/MerkleDistributor', [deployed['support/TestToken'], D[1], root, BigInt(nowSec() + 3600)]) }));
    deployed['core/MerkleDistributor'] = r.contractAddress;
    return r;
  });
  await step('call', 'MerkleDistributor fund + sponsored claim', async () => {
    must('fund', await call({ dev: 1, to: deployed['support/TestToken'], data: enc('support/TestToken', 'transfer', deployed['core/MerkleDistributor'], 2n * 10n ** 18n) }));
    return must('claim', await call({ dev: 3, to: deployed['core/MerkleDistributor'], data: enc('core/MerkleDistributor', 'claim', 0n, D[2], 10n ** 18n, merkleProof(ctx.merkle.leaves, 0)) }));
  });
  await expectRevert('MerkleDistributor.claim twice', call({ dev: 1, to: deployed['core/MerkleDistributor'], data: enc('core/MerkleDistributor', 'claim', 0n, D[2], 10n ** 18n, merkleProof(ctx.merkle.leaves, 0)) }));

  await step('deploy', 'core/MerkleDistributorFactory', async () => {
    const r = must('deploy', await deploy({ dev: 1, code: art['core/MerkleDistributorFactory'].bytecode }));
    deployed['core/MerkleDistributorFactory'] = r.contractAddress;
    return r;
  });
  const mdArgs = () => [deployed['support/TestToken'], u8(merkleRoot(ctx.merkle.leaves)), BigInt(nowSec() + 3600), 10n ** 17n];
  await expectRevert('MerkleDistributorFactory.create without allowance', call({ dev: 1, to: deployed['core/MerkleDistributorFactory'], data: enc('core/MerkleDistributorFactory', 'create', ...mdArgs()) }));
  await step('call', 'MerkleDistributorFactory approve + create', async () => {
    must('approve', await call({ dev: 1, to: deployed['support/TestToken'], data: enc('support/TestToken', 'approve', deployed['core/MerkleDistributorFactory'], 10n ** 18n) }));
    return must('create', await call({ dev: 1, to: deployed['core/MerkleDistributorFactory'], data: enc('core/MerkleDistributorFactory', 'create', ...mdArgs()), gas: 6_000_000 }));
  });

  for (const name of ['core/Randomness', 'toolbox/Randomness']) {
    await step('deploy', name, async () => {
      const r = must('deploy', await deploy({ dev: 1, code: art[name].bytecode }));
      deployed[name] = r.contractAddress;
      return r;
    });
    await step('view', `${name}.randomness(0)`, async () => ({ ok: true, value: await view(name, 'randomness', [0n]) }));
  }

  await step('deploy', 'core/ReleaseLog (fresh copy)', async () => {
    const r = must('deploy', await deploy({ dev: 1, code: art['core/ReleaseLog'].bytecode }));
    deployed['core/ReleaseLog'] = r.contractAddress;
    return r;
  });
  await expectRevert('ReleaseLog.publish empty manifest', call({ dev: 1, to: deployed['core/ReleaseLog'], data: enc('core/ReleaseLog', 'publish', '0x', '0x' + 'ab'.repeat(32), '0x' + 'cd'.repeat(64), false) }));
  await step('call', 'ReleaseLog.publish', async () =>
    must('publish', await call({ dev: 1, to: deployed['core/ReleaseLog'], data: enc('core/ReleaseLog', 'publish', '0x' + 'ee'.repeat(64), '0x' + 'ab'.repeat(32), '0x' + 'cd'.repeat(64), false) })));

  await step('deploy', 'core/TokenBatch', async () => {
    const r = must('deploy', await deploy({ dev: 1, code: art['core/TokenBatch'].bytecode }));
    deployed['core/TokenBatch'] = r.contractAddress;
    return r;
  });
  await step('call', 'TokenBatch.send native ×2', async () =>
    must('send', await call({ dev: 1, to: deployed['core/TokenBatch'], data: enc('core/TokenBatch', 'send', addr0, [D[2], D[3]], [10n ** 15n, 10n ** 15n]), value: 2n * 10n ** 15n })));
  await expectRevert('TokenBatch.send length mismatch', call({ dev: 1, to: deployed['core/TokenBatch'], data: enc('core/TokenBatch', 'send', addr0, [D[2]], [10n ** 15n, 10n ** 15n]), value: 2n * 10n ** 15n }));

  await step('deploy', 'core/TokenLocker', async () => {
    const r = must('deploy', await deploy({ dev: 1, code: art['core/TokenLocker'].bytecode }));
    deployed['core/TokenLocker'] = r.contractAddress;
    return r;
  });
  await step('call', 'TokenLocker approve + lock until now+40s', async () => {
    must('approve', await call({ dev: 1, to: deployed['support/TestToken'], data: enc('support/TestToken', 'approve', deployed['core/TokenLocker'], 10n ** 18n) }));
    return must('lock', await call({ dev: 1, to: deployed['core/TokenLocker'], data: enc('core/TokenLocker', 'lock', deployed['support/TestToken'], D[2], 10n ** 17n, BigInt(nowSec() + 40)) }));
  });
  await expectRevert('TokenLocker.withdraw before unlock', call({ dev: 2, to: deployed['core/TokenLocker'], data: enc('core/TokenLocker', 'withdraw', 0n) }));
  await expectRevert('TokenLocker.lock zero beneficiary', call({ dev: 1, to: deployed['core/TokenLocker'], data: enc('core/TokenLocker', 'lock', deployed['support/TestToken'], addr0, 1n, BigInt(nowSec() + 60)) }));

  await step('deploy', 'core/TokenVesting', async () => {
    const r = must('deploy', await deploy({ dev: 1, code: art['core/TokenVesting'].bytecode }));
    deployed['core/TokenVesting'] = r.contractAddress;
    return r;
  });
  await step('call', 'TokenVesting approve + create stream', async () => {
    must('approve', await call({ dev: 1, to: deployed['support/TestToken'], data: enc('support/TestToken', 'approve', deployed['core/TokenVesting'], 10n ** 18n) }));
    return must('create', await call({ dev: 1, to: deployed['core/TokenVesting'], data: enc('core/TokenVesting', 'create', deployed['support/TestToken'], D[2], 10n ** 17n, BigInt(nowSec()), 20n, 40n, false) }));
  });
  await expectRevert('TokenVesting.claim before cliff', call({ dev: 2, to: deployed['core/TokenVesting'], data: enc('core/TokenVesting', 'claim', 0n) }));
  await expectRevert('TokenVesting.create zero duration', call({ dev: 1, to: deployed['core/TokenVesting'], data: enc('core/TokenVesting', 'create', deployed['support/TestToken'], D[2], 1n, BigInt(nowSec()), 0n, 0n, false) }));

  // ------------------------------------------------------------------ toolbox
  await step('deploy', 'toolbox/AgentVending', async () => {
    const r = must('deploy', await deploy({ dev: 1, code: ctor('toolbox/AgentVending', [D[1], D[2], 10n ** 15n, 120n]) }));
    deployed['toolbox/AgentVending'] = r.contractAddress;
    return r;
  });
  await step('call', 'AgentVending.order', async () =>
    must('order', await call({ dev: 3, to: deployed['toolbox/AgentVending'], data: enc('toolbox/AgentVending', 'order', '0x' + 'aa'.repeat(32)), value: 10n ** 15n })));
  await expectRevert('AgentVending.order wrong price', call({ dev: 3, to: deployed['toolbox/AgentVending'], data: enc('toolbox/AgentVending', 'order', '0x' + 'bb'.repeat(32)), value: 10n ** 14n }));
  await expectRevert('AgentVending.deliver by non-agent', call({ dev: 3, to: deployed['toolbox/AgentVending'], data: enc('toolbox/AgentVending', 'deliver', 1n, '0x' + 'cc'.repeat(32)) }));
  await step('call', 'AgentVending.deliver by agent', async () =>
    must('deliver', await call({ dev: 2, to: deployed['toolbox/AgentVending'], data: enc('toolbox/AgentVending', 'deliver', 1n, '0x' + 'cc'.repeat(32)) })));

  await step('deploy', 'toolbox/AllOrNothingCrowdfund', async () => {
    const r = must('deploy', await deploy({ dev: 1, code: ctor('toolbox/AllOrNothingCrowdfund', [D[1], D[2], 2n * 10n ** 15n, 45n]) }));
    deployed['toolbox/AllOrNothingCrowdfund'] = r.contractAddress;
    return r;
  });
  await step('call', 'Crowdfund.contribute', async () =>
    must('contribute', await call({ dev: 3, to: deployed['toolbox/AllOrNothingCrowdfund'], data: enc('toolbox/AllOrNothingCrowdfund', 'contribute'), value: 15n * 10n ** 14n })));
  await expectRevert('Crowdfund.refund while open', call({ dev: 3, to: deployed['toolbox/AllOrNothingCrowdfund'], data: enc('toolbox/AllOrNothingCrowdfund', 'refund') }));
  await step('call', 'Crowdfund.contribute reach goal', async () =>
    must('contribute2', await call({ dev: 3, to: deployed['toolbox/AllOrNothingCrowdfund'], data: enc('toolbox/AllOrNothingCrowdfund', 'contribute'), value: 5n * 10n ** 14n })));
  ctx.deadlines ??= {};
  ctx.deadlines.crowdfund = nowSec() + 47;
  log('  waiting 47 s for the campaign deadline');
  await sleep(48_000);
  await step('call', 'Crowdfund.withdraw after deadline', async () =>
    must('withdraw', await call({ dev: 1, to: deployed['toolbox/AllOrNothingCrowdfund'], data: enc('toolbox/AllOrNothingCrowdfund', 'withdraw') })));

  await step('deploy', 'toolbox/AmmFactory', async () => {
    const r = must('deploy', await deploy({ dev: 1, code: ctor('toolbox/AmmFactory', [D[1]]) }));
    deployed['toolbox/AmmFactory'] = r.contractAddress;
    return r;
  });
  await step('deploy', 'toolbox/AmmPair (direct)', async () => {
    const [t0, t1] = [deployed['support/TestToken'], deployed.TT2].sort();
    const r = must('deploy', await deploy({ dev: 1, code: ctor('toolbox/AmmPair', [t0, t1, deployed['toolbox/AmmFactory']]) }));
    deployed['toolbox/AmmPair'] = r.contractAddress;
    return r;
  });
  await step('call', 'AmmPair.sync on empty pair', async () =>
    must('sync', await call({ dev: 1, to: deployed['toolbox/AmmPair'], data: enc('toolbox/AmmPair', 'sync') })));
  await expectRevert('AmmPair.mint empty', call({ dev: 1, to: deployed['toolbox/AmmPair'], data: enc('toolbox/AmmPair', 'mint', D[1]) }));
  await expectRevert('AmmFactory.createPair identical tokens', call({ dev: 1, to: deployed['toolbox/AmmFactory'], data: enc('toolbox/AmmFactory', 'createPair', deployed['support/TestToken'], deployed['support/TestToken']) }));
  await step('deploy', 'toolbox/AmmRouter', async () => {
    const r = must('deploy', await deploy({ dev: 1, code: ctor('toolbox/AmmRouter', [deployed['toolbox/AmmFactory']]) }));
    deployed['toolbox/AmmRouter'] = r.contractAddress;
    return r;
  });
  await step('call', 'AmmFactory.createPair TT/TT2', async () =>
    must('createPair', await call({ dev: 1, to: deployed['toolbox/AmmFactory'], data: enc('toolbox/AmmFactory', 'createPair', deployed['support/TestToken'], deployed.TT2), gas: 6_000_000 })));
  await step('call', 'AmmRouter.addLiquidity', async () => {
    const [t0, t1] = [deployed['support/TestToken'], deployed.TT2].sort();
    must('mintA', await call({ dev: 1, to: deployed['support/TestToken'], data: enc('support/TestToken', 'mint', D[1], 10n ** 21n) }));
    must('mintB', await call({ dev: 1, to: deployed.TT2, data: enc('support/TestToken', 'mint', D[1], 10n ** 21n) }));
    must('apprA', await call({ dev: 1, to: deployed['support/TestToken'], data: enc('support/TestToken', 'approve', deployed['toolbox/AmmRouter'], 2n * 10n ** 70n) }));
    must('apprB', await call({ dev: 1, to: deployed.TT2, data: enc('support/TestToken', 'approve', deployed['toolbox/AmmRouter'], 2n * 10n ** 70n) }));
    return must('addLiquidity', await call({ dev: 1, to: deployed['toolbox/AmmRouter'], data: enc('toolbox/AmmRouter', 'addLiquidity', t0, t1, 10n ** 20n, 10n ** 20n, 1n, 1n, D[1], 2n ** 64n - 1n), gas: 6_000_000 }));
  });
  await step('view', 'AmmRouter.getAmountsOut', async () => ({ ok: true, value: await view('toolbox/AmmRouter', 'getAmountsOut', [10n ** 18n, [deployed['support/TestToken'], deployed.TT2].sort()]) }));
  await step('call', 'AmmRouter.swapExactTokensForTokens', async () =>
    must('swap', await call({ dev: 1, to: deployed['toolbox/AmmRouter'], data: enc('toolbox/AmmRouter', 'swapExactTokensForTokens', 10n ** 18n, 1n, [deployed['support/TestToken'], deployed.TT2].sort(), D[2], 2n ** 64n - 1n), gas: 6_000_000 })));
  await expectRevert('AmmRouter.swap path too short', call({ dev: 1, to: deployed['toolbox/AmmRouter'], data: enc('toolbox/AmmRouter', 'swapExactTokensForTokens', 10n ** 18n, 1n, [deployed['support/TestToken']], D[2], 2n ** 64n - 1n) }));

  await step('deploy', 'toolbox/BondingLaunchpad', async () => {
    // LaunchConfig in ABI declaration order (ethers v6 codes tuples by
    // position, not field name): name, symbol, tokenSupply, quoteFloor,
    // tokenFloor, graduationTarget, feeBps, snipeTaxBps, snipeWindow,
    // perBuyerCap, treasury.
    const cfg = ['LiveCurve', 'LIVE', 10n ** 21n, 10n ** 20n, 10n ** 20n, 10n ** 20n, 100n, 1000n, 30n, 0n, D[3]];
    const r = must('deploy', await deploy({ dev: 1, code: ctor('toolbox/BondingLaunchpad', [D[1], deployed['support/TestToken'], deployed['toolbox/AmmFactory'], cfg]), gas: 8_000_000 }));
    deployed['toolbox/BondingLaunchpad'] = r.contractAddress;
    const curve = await view('toolbox/BondingLaunchpad', 'curveToken');
    deployed.curveToken = curve;
    return r;
  });
  await step('call', 'Launchpad buy (snipe window)', async () => {
    must('approve', await call({ dev: 2, to: deployed['support/TestToken'], data: enc('support/TestToken', 'approve', deployed['toolbox/BondingLaunchpad'], 2n * 10n ** 70n) }));
    return must('buy', await call({ dev: 2, to: deployed['toolbox/BondingLaunchpad'], data: enc('toolbox/BondingLaunchpad', 'buy', 2n * 10n ** 19n, 1n), gas: 6_000_000 }));
  });
  await expectRevert('Launchpad.buy zero input', call({ dev: 2, to: deployed['toolbox/BondingLaunchpad'], data: enc('toolbox/BondingLaunchpad', 'buy', 0n, 0n) }));
  await step('call', 'Launchpad.sell', async () => {
    must('approveCurve', await call({ dev: 2, to: deployed.curveToken, data: enc('support/TestToken', 'approve', deployed['toolbox/BondingLaunchpad'], 2n * 10n ** 70n) }));
    return must('sell', await call({ dev: 2, to: deployed['toolbox/BondingLaunchpad'], data: enc('toolbox/BondingLaunchpad', 'sell', 10n ** 18n, 1n), gas: 6_000_000 }));
  });

  await step('deploy', 'toolbox/CommitRevealRaffle', async () => {
    const seed = Buffer.from(keccak256(Buffer.from('raffle-live-seed')));
    ctx.raffleSeed = '0x' + seed.toString('hex');
    const commit = '0x' + Buffer.from(keccak256(seed)).toString('hex');
    const r = must('deploy', await deploy({ dev: 1, code: ctor('toolbox/CommitRevealRaffle', [D[1], commit, 10n ** 15n, 25n, 60n]) }));
    deployed['toolbox/CommitRevealRaffle'] = r.contractAddress;
    return r;
  });
  await step('call', 'Raffle.enter ×2', async () => {
    must('enter1', await call({ dev: 2, to: deployed['toolbox/CommitRevealRaffle'], data: enc('toolbox/CommitRevealRaffle', 'enter'), value: 10n ** 15n }));
    return must('enter2', await call({ dev: 3, to: deployed['toolbox/CommitRevealRaffle'], data: enc('toolbox/CommitRevealRaffle', 'enter'), value: 10n ** 15n }));
  });
  await expectRevert('Raffle.enter wrong ticket value', call({ dev: 2, to: deployed['toolbox/CommitRevealRaffle'], data: enc('toolbox/CommitRevealRaffle', 'enter'), value: 10n ** 14n }));
  await expectRevert('Raffle.reveal before close', call({ dev: 1, to: deployed['toolbox/CommitRevealRaffle'], data: enc('toolbox/CommitRevealRaffle', 'reveal', ctx.raffleSeed) }));
  log('  waiting 26 s for the raffle entry window');
  await sleep(26_000);
  await expectRevert('Raffle.reveal wrong seed', call({ dev: 1, to: deployed['toolbox/CommitRevealRaffle'], data: enc('toolbox/CommitRevealRaffle', 'reveal', '0x' + '99'.repeat(32)) }));
  await step('call', 'Raffle.reveal + payout', async () =>
    must('reveal', await call({ dev: 1, to: deployed['toolbox/CommitRevealRaffle'], data: enc('toolbox/CommitRevealRaffle', 'reveal', ctx.raffleSeed) })));

  await step('deploy', 'toolbox/Editions1155', async () => {
    const r = must('deploy', await deploy({ dev: 1, code: ctor('toolbox/Editions1155', [D[1]]) }));
    deployed['toolbox/Editions1155'] = r.contractAddress;
    return r;
  });
  await step('call', 'Editions.createEdition + mint', async () => {
    must('createEdition', await call({ dev: 1, to: deployed['toolbox/Editions1155'], data: enc('toolbox/Editions1155', 'createEdition', 'live-edition', 3n, 1n, 10n ** 15n, 100n) }));
    return must('mint', await call({ dev: 2, to: deployed['toolbox/Editions1155'], data: enc('toolbox/Editions1155', 'mint', 1n), value: 10n ** 15n }));
  });
  await expectRevert('Editions.mint unknown edition', call({ dev: 2, to: deployed['toolbox/Editions1155'], data: enc('toolbox/Editions1155', 'mint', 99n), value: 10n ** 15n }));
  await expectRevert('Editions.mint over wallet cap', call({ dev: 2, to: deployed['toolbox/Editions1155'], data: enc('toolbox/Editions1155', 'mint', 1n), value: 10n ** 15n }));

  await step('deploy', 'toolbox/FixedPriceMarket', async () => {
    const r = must('deploy', await deploy({ dev: 1, code: ctor('toolbox/FixedPriceMarket', [D[1]]) }));
    deployed['toolbox/FixedPriceMarket'] = r.contractAddress;
    return r;
  });
  await step('call', 'Market list + buy + withdraw', async () => {
    must('mint', await call({ dev: 1, to: deployed['support/MarketNFT'], data: enc('support/MarketNFT', 'mint', D[2]) }));
    must('approve', await call({ dev: 2, to: deployed['support/MarketNFT'], data: enc('support/MarketNFT', 'approve', deployed['toolbox/FixedPriceMarket'], 1n) }));
    must('list', await call({ dev: 2, to: deployed['toolbox/FixedPriceMarket'], data: enc('toolbox/FixedPriceMarket', 'list', deployed['support/MarketNFT'], 1n, 10n ** 15n) }));
    must('buy', await call({ dev: 3, to: deployed['toolbox/FixedPriceMarket'], data: enc('toolbox/FixedPriceMarket', 'buy', 1n), value: 10n ** 15n }));
    return must('withdraw', await call({ dev: 2, to: deployed['toolbox/FixedPriceMarket'], data: enc('toolbox/FixedPriceMarket', 'withdraw') }));
  });
  await expectRevert('Market.buy twice', call({ dev: 2, to: deployed['toolbox/FixedPriceMarket'], data: enc('toolbox/FixedPriceMarket', 'buy', 1n), value: 10n ** 15n }));

  // FixedSupplyToken with a real EIP-2612 permit: a secp256k1 holder signs,
  // dev3 pulls with transferFrom.
  await step('deploy', 'toolbox/FixedSupplyToken (standalone)', async () => {
    const r = must('deploy', await deploy({ dev: 1, code: ctor('toolbox/FixedSupplyToken', ['LiveCoin', 'LIVE', 10n ** 21n, D[1]]) }));
    deployed['toolbox/FixedSupplyToken'] = r.contractAddress;
    return r;
  });
  await step('call', 'FixedSupplyToken transfer + permit + transferFrom', async () => {
    const holder = secp('permit');
    must('fund', await call({ dev: 1, to: deployed['toolbox/FixedSupplyToken'], data: enc('toolbox/FixedSupplyToken', 'transfer', holder.address, 100n * 10n ** 18n) }));
    const domain = await view('toolbox/FixedSupplyToken', 'DOMAIN_SEPARATOR');
    const nonce = await view('toolbox/FixedSupplyToken', 'nonces', [holder.address]);
    const value = 40n * 10n ** 18n;
    const deadline = BigInt(nowSec() + 3600);
    const PERMIT_TYPEHASH = '0x' + Buffer.from(keccak256(Buffer.from('Permit(address owner,address spender,uint256 value,uint256 deadline,uint8 v,bytes32 r,bytes32 s)'))).toString('hex');
    const structHash = Buffer.from(keccak256(E.encode(['bytes32', 'address', 'address', 'uint256', 'uint256', 'uint256'], [PERMIT_TYPEHASH, holder.address, D[3], value, BigInt(nonce), deadline])));
    const digest = Buffer.from(keccak256(Buffer.concat([Buffer.from('1901', 'hex'), Buffer.from(domain.slice(2), 'hex'), structHash])));
    const sig = holder.signingKey.sign(digest);
    must('permit', await call({ dev: 1, to: deployed['toolbox/FixedSupplyToken'], data: enc('toolbox/FixedSupplyToken', 'permit', holder.address, D[3], value, deadline, sig.v, '0x' + sig.r.toString(16).padStart(64, '0'), '0x' + sig.s.toString(16).padStart(64, '0')) }));
    return must('transferFrom', await call({ dev: 3, to: deployed['toolbox/FixedSupplyToken'], data: enc('toolbox/FixedSupplyToken', 'transferFrom', holder.address, D[3], value) }));
  });
  await expectRevert('FixedSupplyToken.transferFrom over allowance', call({ dev: 3, to: deployed['toolbox/FixedSupplyToken'], data: enc('toolbox/FixedSupplyToken', 'transferFrom', deployed['toolbox/FixedSupplyToken'], D[3], 1n) }));

  await step('deploy', 'toolbox/InvoiceBook', async () => {
    const r = must('deploy', await deploy({ dev: 1, code: ctor('toolbox/InvoiceBook', [D[1], D[2]]) }));
    deployed['toolbox/InvoiceBook'] = r.contractAddress;
    return r;
  });
  await step('call', 'InvoiceBook issue + settle', async () => {
    must('issue', await call({ dev: 2, to: deployed['toolbox/InvoiceBook'], data: enc('toolbox/InvoiceBook', 'issue', 10n ** 15n, 60n, 'live-invoice-1') }));
    return must('settle', await call({ dev: 3, to: deployed['toolbox/InvoiceBook'], data: enc('toolbox/InvoiceBook', 'settle', 1n), value: 10n ** 15n }));
  });
  await expectRevert('InvoiceBook.issue by non-payee', call({ dev: 3, to: deployed['toolbox/InvoiceBook'], data: enc('toolbox/InvoiceBook', 'issue', 1n, 60n, 'x') }));

  await step('deploy', 'toolbox/LinearVesting', async () => {
    const token = deployed['support/TestToken'];
    const predicted = getCreateAddress(D[1], Number((await nonceOf(D[1])) + 1n));
    must('approve', await call({ dev: 1, to: token, data: enc('support/TestToken', 'approve', predicted, 10n ** 18n) }));
    const r = must('deploy', await deploy({ dev: 1, code: ctor('toolbox/LinearVesting', [token, D[2], 10n ** 17n, 20n, 40n]) }));
    deployed['toolbox/LinearVesting'] = r.contractAddress;
    r.check = r.contractAddress === predicted;
    return r;
  });
  await step('call', 'LinearVesting.claim early (pays zero)', async () =>
    must('claim0', await call({ dev: 2, to: deployed['toolbox/LinearVesting'], data: enc('toolbox/LinearVesting', 'claim') })));
  await expectRevert('LinearVesting.deploy zero beneficiary', deploy({ dev: 1, code: ctor('toolbox/LinearVesting', [deployed['support/TestToken'], addr0, 1n, 0n, 1n]) }));

  await step('deploy', 'toolbox/MerkleAirdrop', async () => {
    const root = u8(keccak256(Buffer.concat([Buffer.from(D[2].slice(2), 'hex'), pad32(10n ** 15n)])));
    const r = must('deploy', await deploy({ dev: 1, code: ctor('toolbox/MerkleAirdrop', [root, D[1], 300n]) }));
    deployed['toolbox/MerkleAirdrop'] = r.contractAddress;
    return r;
  });
  await step('call', 'MerkleAirdrop fund + claim', async () => {
    must('fund', await send({ dev: 1, to: deployed['toolbox/MerkleAirdrop'], value: 2n * 10n ** 15n }));
    return must('claim', await call({ dev: 2, to: deployed['toolbox/MerkleAirdrop'], data: enc('toolbox/MerkleAirdrop', 'claim', 10n ** 15n, []) }));
  });
  await expectRevert('MerkleAirdrop.claim twice', call({ dev: 2, to: deployed['toolbox/MerkleAirdrop'], data: enc('toolbox/MerkleAirdrop', 'claim', 10n ** 15n, []) }));

  await step('deploy', 'toolbox/MilestoneEscrow', async () => {
    const r = must('deploy', await deploy({ dev: 1, code: ctor('toolbox/MilestoneEscrow', [D[1]]) }));
    deployed['toolbox/MilestoneEscrow'] = r.contractAddress;
    return r;
  });
  await step('call', 'Escrow createDeal + approve + sellerWithdraw', async () => {
    must('createDeal', await call({ dev: 3, to: deployed['toolbox/MilestoneEscrow'], data: enc('toolbox/MilestoneEscrow', 'createDeal', D[2], 2n), value: 2n * 10n ** 15n }));
    must('approve', await call({ dev: 3, to: deployed['toolbox/MilestoneEscrow'], data: enc('toolbox/MilestoneEscrow', 'approveMilestone', 1n, 0n, 10n ** 15n) }));
    return must('sellerWithdraw', await call({ dev: 2, to: deployed['toolbox/MilestoneEscrow'], data: enc('toolbox/MilestoneEscrow', 'sellerWithdraw', 1n) }));
  });
  await expectRevert('Escrow.sellerWithdraw nothing approved', call({ dev: 2, to: deployed['toolbox/MilestoneEscrow'], data: enc('toolbox/MilestoneEscrow', 'sellerWithdraw', 1n) }));

  await step('deploy', 'toolbox/OnchainNFT', async () => {
    const r = must('deploy', await deploy({ dev: 1, code: ctor('toolbox/OnchainNFT', ['LiveArt', 'LART', 5n, 500n, D[1]]) }));
    deployed['toolbox/OnchainNFT'] = r.contractAddress;
    return r;
  });
  await step('call', 'OnchainNFT mint + transfer + view', async () => {
    must('mint', await call({ dev: 1, to: deployed['toolbox/OnchainNFT'], data: enc('toolbox/OnchainNFT', 'mint', D[2], 1, 2, 3, 4) }));
    must('transfer', await call({ dev: 2, to: deployed['toolbox/OnchainNFT'], data: enc('toolbox/OnchainNFT', 'safeTransferFrom', D[2], D[3], 1n) }));
    return { ok: true, tokenURI: await view('toolbox/OnchainNFT', 'tokenURI', [1n]) };
  });
  await expectRevert('OnchainNFT.mint by non-creator', call({ dev: 2, to: deployed['toolbox/OnchainNFT'], data: enc('toolbox/OnchainNFT', 'mint', D[2], 1, 1, 1, 1) }));

  await step('deploy', 'toolbox/RewardDistributor', async () => {
    const r = must('deploy', await deploy({ dev: 1, code: ctor('toolbox/RewardDistributor', [D[1], deployed['support/TestToken'], deployed.TT2]) }));
    deployed['toolbox/RewardDistributor'] = r.contractAddress;
    return r;
  });
  await step('call', 'Rewards fund + stake + unstake', async () => {
    must('fundRewards', await call({ dev: 1, to: deployed.TT2, data: enc('support/TestToken', 'mint', D[1], 10n ** 20n) }));
    must('approveReward', await call({ dev: 1, to: deployed.TT2, data: enc('support/TestToken', 'approve', deployed['toolbox/RewardDistributor'], 10n ** 20n) }));
    must('fund', await call({ dev: 1, to: deployed['toolbox/RewardDistributor'], data: enc('toolbox/RewardDistributor', 'fundRewards', 10n ** 20n, 100n) }));
    must('approveStake', await call({ dev: 2, to: deployed['support/TestToken'], data: enc('support/TestToken', 'approve', deployed['toolbox/RewardDistributor'], 10n ** 20n) }));
    must('stake', await call({ dev: 2, to: deployed['toolbox/RewardDistributor'], data: enc('toolbox/RewardDistributor', 'stake', 10n ** 19n) }));
    return must('unstake', await call({ dev: 2, to: deployed['toolbox/RewardDistributor'], data: enc('toolbox/RewardDistributor', 'unstake', 10n ** 18n) }));
  });
  await expectRevert('Rewards.unstake over balance', call({ dev: 2, to: deployed['toolbox/RewardDistributor'], data: enc('toolbox/RewardDistributor', 'unstake', 10n ** 25n) }));

  // SimpleDAO: secp voter with votes-token weight; short periods so the
  // live run does not wait 2000 s.
  await step('deploy', 'toolbox/SimpleDAO', async () => {
    const r = must('deploy', await deploy({ dev: 1, code: ctor('toolbox/SimpleDAO', [D[1], deployed['support/TestToken'], 100n * 10n ** 18n, 30n, 30n, 300n]) }));
    deployed['toolbox/SimpleDAO'] = r.contractAddress;
    return r;
  });
  // Vote signature = personal_sign over the contract's own getVoteHash(id).
  const daoSig = async (id) => {
    const voteHash = await view('toolbox/SimpleDAO', 'getVoteHash', [id]);
    return ctx.daoVoter.signMessage(Buffer.from(voteHash.slice(2), 'hex'));
  };
  await step('call', 'SimpleDAO fund + propose', async () => {
    must('fund', await send({ dev: 1, to: deployed['toolbox/SimpleDAO'], value: 10n ** 15n }));
    const voter = secp('dao-voter');
    ctx.daoVoter = voter;
    must('votes', await call({ dev: 1, to: deployed['support/TestToken'], data: enc('support/TestToken', 'mint', voter.address, 100n * 10n ** 18n) }));
    const target = D[3];
    const value = 10n ** 15n;
    const execHash = '0x' + Buffer.from(keccak256(E.encode(['address', 'uint256', 'bytes'], [target, value, '0x']))).toString('hex');
    ctx.dao = { target, value, execHash };
    return must('propose', await call({ dev: 1, to: deployed['toolbox/SimpleDAO'], data: enc('toolbox/SimpleDAO', 'propose', execHash) }));
  });
  await expectRevert('SimpleDAO.execute during voting', call({ dev: 1, to: deployed['toolbox/SimpleDAO'], data: enc('toolbox/SimpleDAO', 'execute', 1n, ctx.dao.target, ctx.dao.value, '0x', [await daoSig(1n)]), gas: 2_000_000 }));
  log('  waiting 65 s for the DAO vote + timelock');
  await sleep(65_000);
  await step('call', 'SimpleDAO.execute after timelock', async () =>
    must('execute', await call({ dev: 1, to: deployed['toolbox/SimpleDAO'], data: enc('toolbox/SimpleDAO', 'execute', 1n, ctx.dao.target, ctx.dao.value, '0x', [await daoSig(1n)]), gas: 2_000_000 })));
  await expectRevert('SimpleDAO.execute wrong target (hash mismatch)', call({ dev: 1, to: deployed['toolbox/SimpleDAO'], data: enc('toolbox/SimpleDAO', 'execute', 1n, D[2], ctx.dao.value, '0x', [await daoSig(1n)]), gas: 2_000_000 }));

  // SimpleMultisig: two secp owners, threshold 2, signatures sorted by owner.
  await step('deploy', 'toolbox/SimpleMultisig', async () => {
    const a = secp('msig-a');
    const b = secp('msig-b');
    ctx.msig = { a, b, owners: [a.address, b.address].sort() };
    const r = must('deploy', await deploy({ dev: 1, code: ctor('toolbox/SimpleMultisig', [ctx.msig.owners, 2n]) }));
    deployed['toolbox/SimpleMultisig'] = r.contractAddress;
    return r;
  });
  await step('call', 'SimpleMultisig fund + execute payout', async () => {
    const w = deployed['toolbox/SimpleMultisig'];
    must('fund', await send({ dev: 1, to: w, value: 10n ** 15n }));
    const { a, b } = ctx.msig;
    // The contract publishes the exact hash the owners must personal_sign.
    const txHash = await view('toolbox/SimpleMultisig', 'getTransactionHash', [D[3], 10n ** 15n, '0x', 7n]);
    const sigOf = (k) => k.signMessage(Buffer.from(txHash.slice(2), 'hex'));
    const sigs = await Promise.all([a, b].sort((x, y) => (x.address < y.address ? -1 : 1)).map(sigOf));
    ctx.msigInput = enc('toolbox/SimpleMultisig', 'execute', D[3], 10n ** 15n, '0x', 7n, sigs);
    return must('execute', await call({ dev: 1, to: w, data: ctx.msigInput }));
  });
  await expectRevert('SimpleMultisig.execute replay', call({ dev: 1, to: deployed['toolbox/SimpleMultisig'], data: ctx.msigInput }));

  await step('deploy', 'toolbox/SubscriptionManager', async () => {
    const r = must('deploy', await deploy({ dev: 1, code: ctor('toolbox/SubscriptionManager', [D[1], D[2], 2n]) }));
    deployed['toolbox/SubscriptionManager'] = r.contractAddress;
    return r;
  });
  await step('call', 'Subscription subscribe + cancel', async () => {
    must('subscribe', await call({ dev: 3, to: deployed['toolbox/SubscriptionManager'], data: enc('toolbox/SubscriptionManager', 'subscribe'), value: 2n * 10n ** 15n }));
    return must('cancel', await call({ dev: 3, to: deployed['toolbox/SubscriptionManager'], data: enc('toolbox/SubscriptionManager', 'cancel') }));
  });
  await expectRevert('Subscription.subscribe payment too small', call({ dev: 3, to: deployed['toolbox/SubscriptionManager'], data: enc('toolbox/SubscriptionManager', 'subscribe'), value: 1n }));
  await step('call', 'Subscription.claimRevenue', async () =>
    must('claimRevenue', await call({ dev: 2, to: deployed['toolbox/SubscriptionManager'], data: enc('toolbox/SubscriptionManager', 'claimRevenue') })));

  await step('deploy', 'toolbox/TokenTimeLock', async () => {
    const r = must('deploy', await deploy({ dev: 1, code: ctor('toolbox/TokenTimeLock', [D[1], deployed['support/TestToken']]) }));
    deployed['toolbox/TokenTimeLock'] = r.contractAddress;
    return r;
  });
  await step('call', 'TokenTimeLock lockFor', async () => {
    must('approve', await call({ dev: 1, to: deployed['support/TestToken'], data: enc('support/TestToken', 'approve', deployed['toolbox/TokenTimeLock'], 10n ** 18n) }));
    return must('lockFor', await call({ dev: 1, to: deployed['toolbox/TokenTimeLock'], data: enc('toolbox/TokenTimeLock', 'lockFor', D[2], 10n ** 17n, BigInt(nowSec()), 40n) }));
  });
  await expectRevert('TokenTimeLock.release early', call({ dev: 2, to: deployed['toolbox/TokenTimeLock'], data: enc('toolbox/TokenTimeLock', 'release') }));
  log('  waiting 41 s for the timelock to mature');
  await sleep(41_000);
  await step('call', 'TokenTimeLock.release', async () =>
    must('release', await call({ dev: 2, to: deployed['toolbox/TokenTimeLock'], data: enc('toolbox/TokenTimeLock', 'release') })));

  // NameGatedDrop: dev2 holds the primary name "toolive" from the toolbox
  // names flow above.
  await step('deploy', 'toolbox/NameGatedDrop', async () => {
    const r = must('deploy', await deploy({ dev: 1, code: ctor('toolbox/NameGatedDrop', [deployed['toolbox/EastSeaNames'], D[1], 10n ** 15n, 600n]) }));
    deployed['toolbox/NameGatedDrop'] = r.contractAddress;
    return r;
  });
  await step('call', 'NameGatedDrop fund + claim', async () => {
    must('fund', await send({ dev: 1, to: deployed['toolbox/NameGatedDrop'], value: 2n * 10n ** 15n }));
    return must('claim', await call({ dev: 2, to: deployed['toolbox/NameGatedDrop'], data: enc('toolbox/NameGatedDrop', 'claim') }));
  });
  await expectRevert('NameGatedDrop.claim without primary name', call({ dev: 3, to: deployed['toolbox/NameGatedDrop'], data: enc('toolbox/NameGatedDrop', 'claim') }));
  await expectRevert('NameGatedDrop.claim twice', call({ dev: 2, to: deployed['toolbox/NameGatedDrop'], data: enc('toolbox/NameGatedDrop', 'claim') }));

  // Late claims that only needed time: TokenVesting (cliff 20 s, duration 40)
  // and TokenLocker (unlock at +40 s) matured during the DAO/timelock waits.
  await step('call', 'TokenVesting.claim vested half', async () =>
    must('claim', await call({ dev: 2, to: deployed['core/TokenVesting'], data: enc('core/TokenVesting', 'claim', 0n) })));
  await step('call', 'TokenLocker.withdraw after unlock', async () =>
    must('withdraw', await call({ dev: 2, to: deployed['core/TokenLocker'], data: enc('core/TokenLocker', 'withdraw', 0n) })));

  ctx.deployed = deployed;
  return deployed;
}
