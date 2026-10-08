import test from 'node:test';
import assert from 'node:assert/strict';
import { Wallet } from '../src/lib/wallet.js';

// Import after the test definitions are loaded so the old wallet reports a
// separate red result for every newly covered behavior, rather than one
// module-loader error. See tmp/extension-red.log for the baseline run.
let api = {};
try { api = await import('../src/lib/dappSigning.js'); } catch {}
function feature(name) { assert.equal(typeof api[name], 'function', `${name} must exist`); return api[name]; }

const OWN = '0x1111111111111111111111111111111111111111';
const OTHER = '0x2222222222222222222222222222222222222222';
const TOKEN = '0x3333333333333333333333333333333333333333';
const IMPLEMENTATION = '0x0000000000000000000000000000000000007702';
const CHAIN = 7781; // synthetic new-genesis chain; legacy typed signing stays refused
const TX = { to: TOKEN, value_wei: '0', data: '0x095ea7b3', gas: 100000 };
const WORD = (n) => BigInt(n).toString(16).padStart(64, '0');
const TOPIC_ADDRESS = (a) => `0x${a.slice(2).padStart(64, '0')}`;
const TRANSFER = '0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef';
const APPROVAL = '0x8c5be1e5ebec7d5bd14f71427d1e84f3dd0314c0f7b2291e5b200ac8c7c3b925';
const ALL = '0x17307eab39ab6107e8899845ad3d59bd9653f200f220920489ca2b5937696c31';
const success = (logs = []) => ({ success: true, gasUsed: '0x5208', output: '0x', failureReason: null, nativeChanges: [{ address: OWN, deltaWei: '-1' }], logs });
const transfer = (from, to, n, address = TOKEN) => ({ address, topics: [TRANSFER, TOPIC_ADDRESS(from), TOPIC_ADDRESS(to)], data: `0x${WORD(n)}` });
const approve = (n) => ({ address: TOKEN, topics: [APPROVAL, TOPIC_ADDRESS(OWN), TOPIC_ADDRESS(OTHER)], data: `0x${WORD(n)}` });
const data = () => ({ types: { EIP712Domain: [{ name: 'name', type: 'string' }, { name: 'chainId', type: 'uint256' }, { name: 'verifyingContract', type: 'address' }], Permit: [{ name: 'owner', type: 'address' }, { name: 'value', type: 'uint256' }, { name: 'memo', type: 'string' }] }, primaryType: 'Permit', domain: { name: 'Example DEX', chainId: CHAIN, verifyingContract: TOKEN }, message: { owner: OWN, value: '900719925474099300000', memo: '<script>alert(1)</script>' } });

function fixture() {
  const calls = [];
  const f = { address: OWN, permitted: OWN, simulated: success(), signed: 0, support: true };
  f.rpc = { chainId: CHAIN, generation: 1, call: async (method, params) => {
    calls.push({ method, params });
    if (method === 'aether_status') return { chain_id: f.rpc.chainId, base_fee: { exec: '0', prove: '0' } };
    if (method === 'eth_call') { if (!f.simulated.success) throw Object.assign(new Error('execution reverted: denied'), { code: 3 }); return '0x'; }
    if (method === 'eth_estimateGas') { if (!f.simulated.success) throw Object.assign(new Error('execution reverted: denied'), { code: 3 }); return '0x5208'; }
    if (method === 'aether_simulateTransaction') return structuredClone(f.simulated);
    if (method === 'eth_getCode') return params[0] === IMPLEMENTATION ? '0x6002' : `0xef0100${IMPLEMENTATION.slice(2)}`;
    throw new Error(`unexpected ${method}`);
  } };
  f.vault = { info: async () => ({ address: f.address, publicKey: '04' + '11'.repeat(64) }), sign: async () => { f.signed++; return new Uint8Array(64); } };
  f.wasm = {
    accountSigningSupport: (accountCode, implementationCode) => f.support && accountCode.startsWith('0xef0100') && implementationCode === '0x6002',
    prepareTypedMessage: (_pub, json, chain) => JSON.stringify({ account: OWN, chain_id: Number(chain), signing_message: '0x1234', digest_hex: '0x' + 'aa'.repeat(32), typed_data: JSON.parse(json) }),
    attachTypedSignature: (_json, _chain, account) => { assert.equal(account, OWN); return '0x' + 'ab'.repeat(128); },
  };
  f.signer = (options = {}) => new (feature('DappSigning'))({ rpc: f.rpc, vault: f.vault, wasm: f.wasm, connectedAddress: async () => f.permitted, metadata: async () => ({ symbol: 'TOK', decimals: 18, trusted: true }), ...options });
  f.calls = calls;
  return f;
}

test('simulation tries the exact sender, value and gas on all three read-only node methods', async () => {
  const simulate = feature('simulateTransaction');
  const f = fixture();
  const result = await simulate({ rpc: f.rpc, tx: TX, account: OWN, chainId: CHAIN });
  assert.equal(result.success, true);
  assert.equal(result.contract, TOKEN);
  for (const method of ['eth_call', 'eth_estimateGas', 'aether_simulateTransaction']) {
    const c = f.calls.find((x) => x.method === method);
    assert.deepEqual(c.params, [{ from: OWN, to: TOKEN, value: '0x0', data: TX.data, gas: '0x186a0' }, 'latest']);
  }
});

test('executed Transfer logs net exact per-token wallet changes, without rounding or calldata guesses', async () => {
  const simulate = feature('simulateTransaction');
  const f = fixture();
  f.simulated = success([transfer(OWN, OTHER, 1000000000000000001n), transfer(OTHER, OWN, 1n)]);
  const result = await simulate({ rpc: f.rpc, tx: TX, account: OWN, chainId: CHAIN, metadata: async () => ({ symbol: 'TOK', decimals: 18, trusted: true }) });
  assert.deepEqual(result.balanceChanges.map((x) => [x.kind, x.amount, x.delta]), [['native', '-0.000000000000000001', '-1'], ['erc20', '-1', '-1000000000000000000']]);
  f.simulated = success([]);
  assert.equal((await simulate({ rpc: f.rpc, tx: TX, account: OWN, chainId: CHAIN })).approvals.length, 0, 'approve calldata alone is not an executed grant');
});

test('approval log previews include spender, exact allowance, unlimited and revoke states', async () => {
  const simulate = feature('simulateTransaction');
  const f = fixture();
  f.simulated = success([approve(1000000000000000001n), approve(2n ** 256n - 1n), approve(0n), { address: TOKEN, topics: [ALL, TOPIC_ADDRESS(OWN), TOPIC_ADDRESS(OTHER)], data: `0x${WORD(1)}` }]);
  const result = await simulate({ rpc: f.rpc, tx: TX, account: OWN, chainId: CHAIN, metadata: async () => ({ symbol: 'TOK', decimals: 18, trusted: true }) });
  assert.equal(result.approvals[0].amount, '1.000000000000000001');
  assert.equal(result.approvals[0].spender, OTHER);
  assert.equal(result.approvals[1].unlimited, true);
  assert.equal(result.approvals[2].revoked, true);
  assert.equal(result.approvals[3].kind, 'all');
});

test('canonical Approval topic independently derived from cast keccak decodes as an executed grant', async () => {
  // cast keccak 'Approval(address,address,uint256)' (2026-10-08).
  // This literal must not be imported from the implementation or a fixture
  // constant: matching wrong constants once hid missing allowance warnings.
  const canonical = '0x8c5be1e5ebec7d5bd14f71427d1e84f3dd0314c0f7b2291e5b200ac8c7c3b925';
  const f = fixture();
  f.simulated = success([{ address: TOKEN, topics: [canonical, TOPIC_ADDRESS(OWN), TOPIC_ADDRESS(OTHER)], data: `0x${WORD(2n ** 256n - 1n)}` }]);
  const preview = await feature('simulateTransaction')({ rpc: f.rpc, tx: TX, account: OWN, chainId: CHAIN });
  assert.equal(preview.approvals.length, 1);
  assert.equal(preview.approvals[0].unlimited, true);
});

test('a hardcoded ERC-721 Approval to the zero spender removes the NFT permission', async () => {
  const f = fixture();
  f.simulated = success([{ address: TOKEN, topics: [
    '0x8c5be1e5ebec7d5bd14f71427d1e84f3dd0314c0f7b2291e5b200ac8c7c3b925',
    '0x0000000000000000000000001111111111111111111111111111111111111111',
    '0x0000000000000000000000000000000000000000000000000000000000000000',
    '0x000000000000000000000000000000000000000000000000000000000000002a',
  ], data: '0x' }]);
  const preview = await feature('simulateTransaction')({ rpc: f.rpc, tx: TX, account: OWN, chainId: CHAIN });
  assert.equal(preview.approvals.length, 1);
  assert.equal(preview.approvals[0].kind, 'nft');
  assert.equal(preview.approvals[0].tokenId, '42');
  assert.equal(preview.approvals[0].spender, '0x0000000000000000000000000000000000000000');
  assert.equal(preview.approvals[0].revoked, true);
});

test('unknown token units remain explicit base units and malformed logs cannot invent effects', async () => {
  const simulate = feature('simulateTransaction');
  const f = fixture();
  f.simulated = success([transfer(OWN, OTHER, 900719925474099312345n), { address: TOKEN, topics: [TRANSFER, '0x1', TOPIC_ADDRESS(OWN)], data: `0x${WORD(1)}` }]);
  const result = await simulate({ rpc: f.rpc, tx: TX, account: OWN, chainId: CHAIN });
  const token = result.balanceChanges.find((x) => x.kind === 'erc20');
  assert.equal(token.amount, '-900719925474099312345');
  assert.equal(token.baseUnits, true);
  assert.ok(result.unrecognizedLogs > 0);
});

test('actual simulated ERC-20 balances override misleading Transfer logs, including unchanged balances', async () => {
  const simulate = feature('simulateTransaction');
  const f = fixture();
  f.simulated = { ...success([transfer(OTHER, OWN, 999999n)]), tokenChanges: [{ token: TOKEN, delta: '-1' }], measuredTokens: [TOKEN], tokenCoverageComplete: true };
  const result = await simulate({ rpc: f.rpc, tx: TX, account: OWN, chainId: CHAIN });
  assert.equal(result.balanceChanges.find((x) => x.kind === 'erc20').delta, '-1');
  assert.equal(result.balanceChanges.find((x) => x.kind === 'erc20').source, 'balance');
  f.simulated.tokenChanges = [];
  assert.equal((await simulate({ rpc: f.rpc, tx: TX, account: OWN, chainId: CHAIN })).balanceChanges.filter((x) => x.kind === 'erc20').length, 0);
});

test('partial measured token coverage keeps explicitly labeled log movements for unmeasured tokens', async () => {
  const unmeasured = '0x4444444444444444444444444444444444444444';
  const f = fixture();
  f.simulated = { ...success([transfer(OTHER, OWN, 999n), transfer(OTHER, OWN, 15n, unmeasured)]), tokenChanges: [], measuredTokens: [TOKEN], tokenCoverageComplete: false };
  const preview = await feature('simulateTransaction')({ rpc: f.rpc, tx: TX, account: OWN, chainId: CHAIN });
  const tokens = preview.balanceChanges.filter((x) => x.kind === 'erc20');
  assert.equal(tokens.length, 1, 'unmeasured token remains visible; measured zero delta overrides a fabricated event');
  assert.equal(tokens[0].address, unmeasured);
  assert.equal(tokens[0].source, 'log');
  assert.equal(tokens[0].delta, '15');
  assert.equal(preview.tokenCoverageComplete, false);
});

test('simulation failure has a readable reason and no reverted balance or allowance effects', async () => {
  const simulate = feature('simulateTransaction');
  const f = fixture();
  f.simulated = { ...success([approve(10)]), success: false, failureReason: 'insufficient allowance' };
  const result = await simulate({ rpc: f.rpc, tx: TX, account: OWN, chainId: CHAIN });
  assert.equal(result.success, false);
  assert.match(result.failureReason, /insufficient allowance/);
  assert.deepEqual(result.balanceChanges, []);
  assert.deepEqual(result.approvals, []);
});

test('a node outage, missing trace, malformed trace or inconsistent eth_call blocks simulation', async () => {
  const simulate = feature('simulateTransaction');
  for (const bad of ['outage', 'missing', 'malformed', 'inconsistent']) {
    const f = fixture();
    const call = f.rpc.call;
    f.rpc.call = async (method, params) => {
      if (bad === 'outage' && method === 'eth_estimateGas') throw new Error('node did not answer');
      if (method === 'aether_simulateTransaction' && bad === 'missing') return null;
      if (method === 'aether_simulateTransaction' && bad === 'malformed') return { ...success(), gasUsed: 'not gas' };
      if (method === 'eth_call' && bad === 'inconsistent') throw Object.assign(new Error('execution reverted'), { code: 3 });
      return call(method, params);
    };
    await assert.rejects(simulate({ rpc: f.rpc, tx: TX, account: OWN, chainId: CHAIN }), (e) => e.key === 'simulationUnavailable', bad);
  }
});

test('reverted dApp transactions require a separate backend acknowledgement and the displayed preview id', async () => {
  const f = fixture();
  f.simulated = { ...success(), success: false, failureReason: 'denied' };
  const signer = f.signer();
  const request = await signer.prepareTransaction('https://dapp.test', TX);
  await assert.rejects(signer.confirmTransaction(request, { previewId: request.previewId }), (e) => e.key === 'confirmRevert');
  await assert.rejects(signer.confirmTransaction(request, { previewId: 'different', confirmRevert: true }), (e) => e.key === 'previewChanged');
  await signer.confirmTransaction(request, { previewId: request.previewId, confirmRevert: true });
  assert.equal(f.signed, 0, 'guards do not sign');
});

test('final transaction guard reruns simulation and refuses changed executed effects', async () => {
  const f = fixture();
  const signer = f.signer();
  const request = await signer.prepareTransaction('https://dapp.test', TX);
  f.simulated = success([approve(2n ** 256n - 1n)]);
  await assert.rejects(signer.confirmTransaction(request, { previewId: request.previewId }), (e) => e.key === 'previewChanged');
  assert.equal(f.signed, 0);
});

test('account, chain, RPC generation, request mutation and site revocation invalidate an open transaction', async () => {
  for (const change of ['account', 'chain', 'generation', 'tx', 'permission']) {
    const f = fixture();
    const signer = f.signer();
    const request = await signer.prepareTransaction('https://dapp.test', TX);
    if (change === 'account') f.address = OTHER;
    if (change === 'chain') f.rpc.chainId++;
    if (change === 'generation') f.rpc.generation++;
    if (change === 'tx') request.tx.value_wei = '1';
    if (change === 'permission') f.permitted = null;
    await assert.rejects(signer.confirmTransaction(request, { previewId: request.previewId }), (e) => ['requestChanged', 'notConnected'].includes(e.key), change);
    assert.equal(f.signed, 0);
  }
});

test('permission generation invalidates a transaction after the same account is revoked and regranted', async () => {
  const f = fixture(); let revision = 0;
  const signer = f.signer({ permissionGeneration: () => revision });
  const request = await signer.prepareTransaction('https://dapp.test', TX);
  f.permitted = null; revision++;
  f.permitted = OWN; revision++;
  await assert.rejects(signer.confirmTransaction(request, { previewId: request.previewId }), (e) => e.key === 'requestChanged');
  assert.equal(f.signed, 0);
});

test('permission generation changes during asynchronous context capture are refused', async () => {
  const f = fixture(); let revision = 0;
  const signer = f.signer({ permissionGeneration: () => revision });
  const info = f.vault.info;
  f.vault.info = async () => { revision += 2; return info(); };
  await assert.rejects(signer.prepareTransaction('https://dapp.test', TX), (e) => e.key === 'requestChanged');
});

test('typed requests reject wrong chain, absent chain, malformed domain and account mismatch before signing', () => {
  const normalize = feature('normalizeTypedRequest');
  for (const chainId of [1, '0x1', undefined, 7780, 7781.5, '7781garbage']) {
    const typed = data(); typed.domain.chainId = chainId;
    assert.throws(() => normalize([OWN, JSON.stringify(typed)], OWN, CHAIN));
  }
  for (const domain of [{ ...data().domain, verifyingContract: '0x1234' }, { ...data().domain, name: '' }, { ...data().domain, name: 'Trusted\u202Eevil' }]) {
    assert.throws(() => normalize([OWN, { ...data(), domain }], OWN, CHAIN));
  }
  assert.throws(() => normalize([OTHER, data()], OWN, CHAIN), (e) => e.key === 'wrongAccount');
  assert.equal(normalize([OWN, JSON.stringify(data())], OWN, CHAIN).message.value, '900719925474099300000');
});

test('readable typed fields preserve exact integers and nested paths while putting bytes in details', () => {
  const view = feature('typedFieldView');
  const typed = data();
  typed.types.Permit.push({ name: 'items', type: 'Item[]' }, { name: 'proof', type: 'bytes32' });
  typed.types.Item = [{ name: 'ok', type: 'bool' }, { name: 'amount', type: 'uint256' }];
  typed.message.items = [{ ok: true, amount: '1000000000000000001' }];
  typed.message.proof = '0x' + 'aa'.repeat(32);
  const fields = view(typed);
  assert.equal(fields.find((x) => x.path === 'value').value, '900719925474099300000');
  assert.equal(fields.find((x) => x.path === 'items[0].amount').value, '1000000000000000001');
  assert.equal(fields.find((x) => x.path === 'proof').byteLength, 32);
  assert.equal(fields.find((x) => x.path === 'proof').detail, typed.message.proof);
  assert.ok(!fields.find((x) => x.path === 'proof').value.startsWith('0x'));
  assert.equal(fields.find((x) => x.path === 'memo').value, typed.message.memo, 'rendered as text, never HTML');
});

test('typed field review refuses hidden, missing or unbounded fields instead of signing unseen data', () => {
  const view = feature('typedFieldView');
  const extra = data(); extra.message.hidden = 'not reviewed';
  assert.throws(() => view(extra));
  const missing = data(); delete missing.message.owner;
  assert.throws(() => view(missing));
  const many = data(); many.types.Permit = [{ name: 'items', type: 'uint256[]' }]; many.message = { items: Array.from({ length: 300 }, () => '1') };
  assert.throws(() => view(many));
});

test('typed signing requires an actual 7702 delegation and exact account-v2 code on the node', async () => {
  const f = fixture();
  const signer = f.signer();
  await signer.prepareTyped('https://dapp.test', [OWN, data()]);
  assert.ok(f.calls.some((x) => x.method === 'eth_getCode' && x.params[0] === OWN));
  assert.ok(f.calls.some((x) => x.method === 'eth_getCode' && x.params[0] === IMPLEMENTATION));
  f.support = false;
  await assert.rejects(signer.prepareTyped('https://dapp.test', [OWN, data()]), (e) => e.key === 'typedUnsupported');
  assert.equal(f.signed, 0);
});

test('typed approval returns the account-bound ERC-1271 signature, never a transaction or EOA signature', async () => {
  const f = fixture();
  const signer = f.signer();
  const request = await signer.prepareTyped('https://dapp.test', [OWN, JSON.stringify(data())]);
  const signature = await signer.signTyped(request, { previewId: request.previewId });
  assert.match(signature, /^0x[0-9a-f]{256}$/);
  assert.equal(f.signed, 1);
  assert.equal(f.calls.some((x) => x.method === 'aether_sendTransaction'), false);
});

test('typed signing rechecks chain, account, permission, domain contents and deployed support at confirmation', async () => {
  for (const change of ['account', 'chain', 'permission', 'typed', 'support']) {
    const f = fixture(); const signer = f.signer();
    const request = await signer.prepareTyped('https://dapp.test', [OWN, data()]);
    if (change === 'account') f.address = OTHER;
    if (change === 'chain') f.rpc.chainId++;
    if (change === 'permission') f.permitted = null;
    if (change === 'typed') request.typed_data.message.value = '2';
    if (change === 'support') f.support = false;
    await assert.rejects(signer.signTyped(request, { previewId: request.previewId }), undefined, change);
    assert.equal(f.signed, 0, change);
  }
});

test('a permission revoked while a typed signature is being made prevents its release to the dApp', async () => {
  const f = fixture(); const signer = f.signer();
  const request = await signer.prepareTyped('https://dapp.test', [OWN, data()]);
  f.vault.sign = async () => { f.permitted = null; f.signed++; return new Uint8Array(64); };
  await assert.rejects(signer.signTyped(request, { previewId: request.previewId }), (e) => e.key === 'notConnected');
});

test('permission generation prevents typed signature release after a revoke and regrant during signing', async () => {
  const f = fixture(); let revision = 0;
  const signer = f.signer({ permissionGeneration: () => revision });
  const request = await signer.prepareTyped('https://dapp.test', [OWN, data()]);
  f.vault.sign = async () => { f.permitted = null; revision++; f.permitted = OWN; revision++; f.signed++; return new Uint8Array(64); };
  await assert.rejects(signer.signTyped(request, { previewId: request.previewId }), (e) => e.key === 'requestChanged');
  assert.equal(f.signed, 1, 'the already-made signature must never be released');
});

function sendWallet() {
  const f = { signed: 0, sent: 0 };
  f.wallet = new Wallet({
    rpc: { chainId: CHAIN, call: async (method) => {
      if (method === 'aether_status') return { chain_id: CHAIN };
      if (method === 'eth_getBalance') return '0xde0b6b3a7640000';
      if (method === 'eth_getTransactionCount') return '0x0';
      if (method === 'aether_sendTransaction') { f.sent++; return { hash: '0xsent' }; }
      throw new Error(method);
    } },
    vault: { info: async () => ({ address: OWN, publicKey: '04' + '11'.repeat(64) }), sign: async () => { f.signed++; return new Uint8Array(64); } },
    wasm: { prepareTx: () => JSON.stringify({ signing_message: '1234', envelope: {} }), attachSignature: () => '{}' },
  });
  return f;
}

test('the wallet executes the dApp guard immediately before its vault signs', async () => {
  const f = sendWallet();
  await assert.rejects(f.wallet.send(TX, { beforeSign: async () => { throw new Error('preview invalid'); } }), /preview invalid/);
  assert.equal(f.signed, 0);
  assert.equal(f.sent, 0);
});

test('the wallet checks dApp permission again after signing and before submission', async () => {
  const f = sendWallet();
  await assert.rejects(f.wallet.send(TX, { afterSign: async () => { throw new Error('permission revoked'); } }), /permission revoked/);
  assert.equal(f.signed, 1);
  assert.equal(f.sent, 0);
});
