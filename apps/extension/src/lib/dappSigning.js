// dApp previews and typed-message guards. The UI only receives reviewed
// fields; private signing messages remain in the service worker. Node calls
// here execute against ephemeral state and never submit a transaction.
import { hex } from './vault.js';
import { formatTokenAmountExact } from './tokens.js';
import { revertReason } from './safety.js';
import { coinTicker } from './brand.js';
import { t } from './i18n.js';

const ADDRESS = /^0x[0-9a-fA-F]{40}$/;
const HEX = /^0x(?:[0-9a-fA-F]{2})*$/;
const WORD = /^0x[0-9a-fA-F]{64}$/;
const UINT = /^(?:0x[0-9a-fA-F]+|[0-9]+)$/;
const INVISIBLE = /[\u0000-\u001f\u007f-\u009f\u200b-\u200f\u202a-\u202e\u2066-\u2069]/;
export const ACCOUNT_IMPLEMENTATION = '0x0000000000000000000000000000000000007702';
const TRANSFER = '0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef';
const APPROVAL = '0x8c5be1e5ebec7d5bd14f71427d1e84f3dd0314c0f7b2291e5b200ac8c7c3b925';
const APPROVAL_ALL = '0x17307eab39ab6107e8899845ad3d59bd9653f200f220920489ca2b5937696c31';

function refusal(key, code = -32603) { return Object.assign(new Error(t(key)), { key, code }); }
function object(value) { return value !== null && typeof value === 'object' && !Array.isArray(value); }
function integer(value) {
  if (typeof value === 'number') {
    if (!Number.isSafeInteger(value) || value < 0) throw refusal('malformedTyped', -32602);
    return BigInt(value);
  }
  if (typeof value !== 'string' || !UINT.test(value)) throw refusal('malformedTyped', -32602);
  return BigInt(value);
}
function stable(value) {
  if (Array.isArray(value)) return `[${value.map(stable).join(',')}]`;
  if (object(value)) return `{${Object.keys(value).sort().map((key) => `${JSON.stringify(key)}:${stable(value[key])}`).join(',')}}`;
  return JSON.stringify(value);
}
function transactionKey(tx) { return stable({ to: tx.to.toLowerCase(), value_wei: tx.value_wei, data: tx.data, gas: tx.gas }); }
function exact(raw, decimals) {
  const n = BigInt(raw);
  const out = formatTokenAmountExact(n < 0n ? -n : n, decimals);
  const amount = out.includes('.') ? out.replace(/0+$/, '').replace(/\.$/, '') : out;
  return `${n < 0n ? '-' : ''}${amount}`;
}
function indexedAddress(topic) {
  if (!WORD.test(topic) || !/^0x0{24}/i.test(topic)) throw new Error('invalid indexed address');
  return `0x${topic.slice(-40).toLowerCase()}`;
}
function signedDecimal(value) {
  if (typeof value !== 'string' || !/^-?[0-9]+$/.test(value)) throw new Error('invalid balance delta');
  return BigInt(value);
}
function isRevert(error) { return error?.code === 3 || /revert|out of gas|invalid opcode/i.test(error?.message || ''); }
function readableFailure(value, output) {
  const text = String(value || '');
  const decoded = revertReason(text || (output?.startsWith('0x08c379a0') ? `execution reverted: ${output}` : ''));
  if (decoded && !/0x[0-9a-fA-F]{8,}/.test(decoded)) return decoded.slice(0, 512).replace(/[\u202a-\u202e\u2066-\u2069]/g, '');
  if (output?.startsWith('0x4e487b71') || /panic/i.test(text)) return t('panicFailure');
  return output && output !== '0x' ? t('customError') : t('unknownFailure');
}

/** The same effective gas limit used by prepareTx when the page omits gas. */
export function simulationCall(tx, account) {
  return { from: account, ...(tx.to ? { to: tx.to } : {}), value: `0x${BigInt(tx.value_wei).toString(16)}`, data: tx.data,
    gas: `0x${BigInt(tx.gas || (tx.data === '0x' ? 21_000 : 3_000_000)).toString(16)}` };
}

export async function simulateTransaction({ rpc, tx, account, chainId, metadata = async () => null }) {
  const params = [simulationCall(tx, account), 'latest'];
  const [called, estimated, traced] = await Promise.allSettled(['eth_call', 'eth_estimateGas', 'aether_simulateTransaction'].map((method) => rpc.call(method, params)));
  const result = traced.status === 'fulfilled' ? traced.value : null;
  try {
    if (!object(result) || typeof result.success !== 'boolean' || !/^0x[0-9a-fA-F]+$/.test(result.gasUsed) || !HEX.test(result.output)
      || !Array.isArray(result.nativeChanges) || !Array.isArray(result.logs) || result.logs.length > 512 || result.nativeChanges.length > 512
      || (result.failureReason !== null && typeof result.failureReason !== 'string')) throw new Error('invalid simulation');
    for (const check of [called, estimated]) if (check.status === 'rejected' && !isRevert(check.reason)) throw new Error('simulation unavailable');
    if (called.status === 'fulfilled' && !HEX.test(called.value)) throw new Error('invalid eth_call');
    if (called.status === 'rejected' && result.success) throw new Error('inconsistent simulation');
    if (called.status === 'fulfilled' && !result.success) throw new Error('inconsistent simulation');
    if (estimated.status === 'rejected' && result.success) throw new Error('inconsistent estimate');
    if (estimated.status === 'fulfilled' && !/^0x[0-9a-fA-F]+$/.test(estimated.value)) throw new Error('invalid gas estimate');
    if (rpc.chainId !== chainId) throw new Error('network changed');

    const preview = { contract: tx.to || null, success: result.success, gasUsed: BigInt(result.gasUsed).toString(),
      estimatedGas: estimated.status === 'fulfilled' ? BigInt(estimated.value).toString() : null,
      failureReason: result.success ? null : readableFailure(result.failureReason, result.output), balanceChanges: [], approvals: [], unrecognizedLogs: 0,
      tokenCoverageComplete: result.tokenCoverageComplete === true };
    if (!result.success) return preview; // reverted writes and logs are not effects
    const own = account.toLowerCase();
    let native = 0n;
    for (const change of result.nativeChanges) {
      if (!ADDRESS.test(change.address)) throw new Error('invalid native account');
      const delta = signedDecimal(change.deltaWei);
      if (change.address.toLowerCase() === own) native += delta;
    }
    if (native !== 0n) preview.balanceChanges.push({ kind: 'native', address: account, symbol: coinTicker(chainId), amount: exact(native, 18), delta: native.toString(), source: 'balance', baseUnits: false, trusted: true });
    const movements = new Map(), grants = [];
    const move = (address, kind, delta, id = null) => {
      const key = `${address}:${kind}:${id ?? ''}`;
      const current = movements.get(key) || { kind, address, delta: 0n, tokenId: id, source: 'log' };
      current.delta += delta;
      movements.set(key, current);
    };
    for (const log of result.logs) {
      try {
        if (!ADDRESS.test(log.address) || !Array.isArray(log.topics) || !log.topics.every((topic) => WORD.test(topic)) || !HEX.test(log.data)) throw new Error('invalid log');
        const address = log.address.toLowerCase(), [topic, from, to, token] = log.topics;
        if (topic?.toLowerCase() === TRANSFER && [3, 4].includes(log.topics.length)) {
          const sender = indexedAddress(from), recipient = indexedAddress(to);
          if (log.topics.length === 3 && WORD.test(log.data)) {
            const value = BigInt(log.data);
            move(address, 'erc20', (recipient === own ? value : 0n) - (sender === own ? value : 0n));
          } else if (log.topics.length === 4 && log.data === '0x') {
            move(address, 'nft', (recipient === own ? 1n : 0n) - (sender === own ? 1n : 0n), BigInt(token).toString());
          } else throw new Error('invalid transfer');
        } else if (topic?.toLowerCase() === APPROVAL && [3, 4].includes(log.topics.length)) {
          const owner = indexedAddress(from), spender = indexedAddress(to);
          if (owner !== own) continue;
          if (log.topics.length === 3 && WORD.test(log.data)) {
            const allowance = BigInt(log.data);
            grants.push({ kind: 'erc20', address, spender, allowance: allowance.toString(), unlimited: allowance === 2n ** 256n - 1n, revoked: allowance === 0n });
          } else if (log.topics.length === 4 && log.data === '0x') grants.push({ kind: 'nft', address, spender, tokenId: BigInt(token).toString(), revoked: /^0x0{40}$/.test(spender) });
          else throw new Error('invalid approval');
        } else if (topic?.toLowerCase() === APPROVAL_ALL && log.topics.length === 3 && WORD.test(log.data)) {
          const owner = indexedAddress(from), spender = indexedAddress(to), approved = BigInt(log.data);
          if (approved > 1n) throw new Error('invalid approval flag');
          if (owner === own) grants.push({ kind: 'all', address, spender, revoked: approved === 0n });
        } else preview.unrecognizedLogs++;
      } catch { preview.unrecognizedLogs++; }
    }
    // New nodes read balanceOf before/after ephemeral execution. These deltas
    // take precedence over Transfer events, which a contract can fabricate.
    if (result.tokenChanges !== undefined) {
      if (!Array.isArray(result.tokenChanges) || result.tokenChanges.length > 512) throw new Error('invalid token changes');
      const measured = result.measuredTokens ?? result.tokenChanges.map((change) => change.token);
      if (!Array.isArray(measured) || measured.length > 512 || !measured.every((address) => ADDRESS.test(address))) throw new Error('invalid measured token set');
      const measuredSet = new Set(measured.map((address) => address.toLowerCase()));
      for (const [key, movement] of movements) if (movement.kind === 'erc20' && measuredSet.has(movement.address)) movements.delete(key);
      for (const change of result.tokenChanges) {
        if (!ADDRESS.test(change.token)) throw new Error('invalid token account');
        const address = change.token.toLowerCase();
        if (!measuredSet.has(address)) throw new Error('unmeasured token delta');
        if (movements.has(`${address}:erc20:`)) throw new Error('duplicate token delta');
        movements.set(`${address}:erc20:`, { kind: 'erc20', address, delta: signedDecimal(change.delta), tokenId: null, source: 'balance' });
      }
    }
    const tokenMetadata = new Map();
    for (const address of new Set([...movements.values(), ...grants].map((x) => x.address))) {
      const token = await metadata(address).catch(() => null);
      tokenMetadata.set(address, token && Number.isInteger(token.decimals) && token.decimals >= 0 && token.decimals <= 77 ? token : null);
    }
    const units = (address, delta) => {
      const token = tokenMetadata.get(address);
      return { symbol: token?.symbol || null, amount: token ? exact(delta, token.decimals) : delta.toString(), baseUnits: !token, trusted: Boolean(token?.trusted) };
    };
    for (const movement of [...movements.values()].sort((a, b) => `${a.address}:${a.tokenId || ''}`.localeCompare(`${b.address}:${b.tokenId || ''}`))) {
      if (movement.delta === 0n) continue;
      preview.balanceChanges.push({ ...movement, delta: movement.delta.toString(), ...(movement.kind === 'nft'
        ? { amount: movement.delta.toString(), baseUnits: false, symbol: tokenMetadata.get(movement.address)?.symbol || null, trusted: false }
        : units(movement.address, movement.delta)) });
    }
    preview.approvals = grants.map((grant) => ({ ...grant, ...(grant.kind === 'erc20' ? units(grant.address, BigInt(grant.allowance)) : {}) }));
    if (rpc.chainId !== chainId) throw new Error('network changed');
    return preview;
  } catch { throw refusal('simulationUnavailable'); }
}

/** Require a reviewable EIP-712 domain, explicit current chain, and account. */
export function normalizeTypedRequest(params, account, chainId) {
  if (!Array.isArray(params) || params.length !== 2 || !ADDRESS.test(params[0])) throw refusal('malformedTyped', -32602);
  if (params[0].toLowerCase() !== account.toLowerCase()) throw refusal('wrongAccount', 4100);
  let typed;
  try {
    const json = typeof params[1] === 'string' ? params[1] : JSON.stringify(params[1]);
    if (typeof json !== 'string' || json.length > 65_536) throw refusal('typedTooLarge', -32602);
    typed = JSON.parse(json);
  } catch (e) { throw e.key ? e : refusal('malformedTyped', -32602); }
  if (!object(typed) || !object(typed.domain) || !object(typed.types) || !object(typed.message)
    || typeof typed.primaryType !== 'string' || typed.primaryType === 'EIP712Domain'
    || typeof typed.domain.name !== 'string' || !typed.domain.name.trim() || typed.domain.name.length > 256 || INVISIBLE.test(typed.domain.name)
    || (typed.domain.verifyingContract !== undefined && !ADDRESS.test(typed.domain.verifyingContract))) throw refusal('malformedTyped', -32602);
  let requested;
  try { requested = integer(typed.domain.chainId); } catch { throw refusal('wrongChain', 4901); }
  if (requested !== BigInt(chainId)) throw refusal('wrongChain', 4901);
  const domainTypes = typed.types.EIP712Domain;
  if (!Array.isArray(domainTypes) || !domainTypes.some((f) => f.name === 'chainId' && f.type === 'uint256')
    || !domainTypes.some((f) => f.name === 'name' && f.type === 'string')) throw refusal('malformedTyped', -32602);
  typedFieldView(typed, 'EIP712Domain', typed.domain);
  typedFieldView(typed);
  return typed;
}

/** Flatten struct/array paths; integers stay exact and bytes use an explicit
 * byte count with an expandable detail, never raw JSON as the primary view. */
export function typedFieldView(typed, rootType = typed.primaryType, rootValue = typed.message) {
  const rows = [];
  function row(value) { if (rows.length >= 128) throw refusal('typedTooLarge', -32602); rows.push(value); }
  function visit(type, value, path, depth) {
    if (depth > 12) throw refusal('typedTooLarge', -32602);
    const array = /^(.*)\[([0-9]*)\]$/.exec(type);
    if (array) {
      if (!Array.isArray(value) || (array[2] && value.length !== Number(array[2]))) throw refusal('malformedTyped', -32602);
      if (value.length > 64) throw refusal('typedTooLarge', -32602);
      if (!value.length) row({ path, type, value: '', displayKey: 'emptyList' });
      for (let i = 0; i < value.length; i++) visit(array[1], value[i], `${path}[${i}]`, depth + 1);
      return;
    }
    const fields = Object.hasOwn(typed.types, type) ? typed.types[type] : null;
    if (fields) {
      if (!Array.isArray(fields) || !object(value) || fields.length > 128) throw refusal('malformedTyped', -32602);
      const names = fields.map((f) => f.name);
      if (new Set(names).size !== names.length || Object.keys(value).some((key) => !names.includes(key))) throw refusal('malformedTyped', -32602);
      for (const field of fields) {
        if (!object(field) || typeof field.name !== 'string' || !/^[A-Za-z_][A-Za-z0-9_]*$/.test(field.name) || typeof field.type !== 'string' || !Object.hasOwn(value, field.name)) throw refusal('malformedTyped', -32602);
        visit(field.type, value[field.name], path ? `${path}.${field.name}` : field.name, depth + 1);
      }
      return;
    }
    let display;
    const intType = /^(u?int)([0-9]*)$/.exec(type), bytesType = /^bytes([0-9]*)$/.exec(type);
    if (intType) {
      const bits = Number(intType[2] || 256);
      if (bits < 8 || bits > 256 || bits % 8) throw refusal('malformedTyped', -32602);
      let n;
      if (intType[1] === 'int' && typeof value === 'string' && /^-[0-9]+$/.test(value)) n = BigInt(value);
      else if (intType[1] === 'int' && typeof value === 'number' && Number.isSafeInteger(value) && value < 0) n = BigInt(value);
      else n = integer(value);
      const signed = intType[1] === 'int';
      if (n < (signed ? -(2n ** BigInt(bits - 1)) : 0n) || n >= 2n ** BigInt(signed ? bits - 1 : bits)) throw refusal('malformedTyped', -32602);
      display = n.toString();
    } else if (bytesType) {
      if (typeof value !== 'string' || !HEX.test(value) || (bytesType[1] && ((Number(bytesType[1]) < 1 || Number(bytesType[1]) > 32) || value.length !== 2 + Number(bytesType[1]) * 2))) throw refusal('malformedTyped', -32602);
      row({ path, type, value: t('byteCount', { count: (value.length - 2) / 2 }), byteLength: (value.length - 2) / 2, detail: value });
      return;
    } else if (type === 'bool') {
      if (typeof value !== 'boolean') throw refusal('malformedTyped', -32602);
      row({ path, type, value: '', displayKey: value ? 'yes' : 'no' }); return;
    } else if (type === 'address') {
      if (typeof value !== 'string' || !ADDRESS.test(value)) throw refusal('malformedTyped', -32602);
      display = value;
    } else if (type === 'string') {
      if (typeof value !== 'string' || value.length > 4096) throw refusal('malformedTyped', -32602);
      display = value;
    } else throw refusal('malformedTyped', -32602);
    row({ path, type, value: display });
  }
  visit(rootType, rootValue, '', 0);
  return rows;
}

export class DappSigning {
  constructor({ rpc, vault, wasm, connectedAddress, metadata = async () => null, permissionGeneration = () => 0 }) {
    Object.assign(this, { rpc, vault, wasm, connectedAddress, metadata, permissionGeneration });
  }

  async context(origin) {
    const chainId = this.rpc.chainId, generation = this.rpc.generation, permissionGeneration = this.permissionGeneration(origin);
    const info = await this.vault.info(), connected = await this.connectedAddress(origin);
    if (!info || !connected || connected.toLowerCase() !== info.address.toLowerCase()) throw refusal('notConnected', 4100);
    if (this.permissionGeneration(origin) !== permissionGeneration) throw refusal('requestChanged', 4100);
    if (this.rpc.chainId !== chainId || this.rpc.generation !== generation) throw refusal('requestChanged', 4901);
    return { chainId, generation, permissionGeneration, account: info.address, publicKey: info.publicKey };
  }

  async assertContext(request) {
    const current = await this.context(request.origin);
    if (current.permissionGeneration !== request.context.permissionGeneration) throw refusal('requestChanged', 4100);
    if (stable(current) !== stable(request.context) || (request.tx && transactionKey(request.tx) !== request.transactionKey)) throw refusal('requestChanged', 4901);
    return current;
  }

  async transactionRequest(origin, tx) {
    const context = await this.context(origin);
    return { origin, kind: 'send', tx: structuredClone(tx), context, transactionKey: transactionKey(tx) };
  }

  async prepareTransaction(origin, tx) {
    const request = await this.transactionRequest(origin, tx), { context } = request;
    const [status, simulation] = await Promise.all([this.rpc.call('aether_status', []), simulateTransaction({ rpc: this.rpc, tx, account: context.account, chainId: context.chainId, metadata: this.metadata })]);
    await this.assertContext(request);
    if (Number(status.chain_id) !== context.chainId) throw refusal('requestChanged', 4901);
    return { ...request, status, simulation, simulationKey: stable(simulation), previewId: crypto.randomUUID() };
  }

  async refreshTransaction(request) {
    await this.assertContext(request);
    const fresh = await this.prepareTransaction(request.origin, request.tx);
    await this.assertContext(request);
    return fresh;
  }

  async confirmTransaction(request, { previewId, confirmRevert = false } = {}) {
    await this.assertContext(request);
    if (!request.simulation || previewId !== request.previewId) throw refusal('previewChanged');
    if (!request.simulation.success && confirmRevert !== true) throw refusal('confirmRevert', 4001);
    const current = await simulateTransaction({ rpc: this.rpc, tx: request.tx, account: request.context.account, chainId: request.context.chainId, metadata: this.metadata });
    await this.assertContext(request);
    if (stable(current) !== request.simulationKey) throw refusal('previewChanged');
  }

  async supported(request) {
    if (typeof this.wasm.accountSigningSupport !== 'function' || typeof this.wasm.prepareTypedMessage !== 'function' || typeof this.wasm.attachTypedSignature !== 'function') throw refusal('typedUnsupported', 4200);
    const [accountCode, implementationCode] = await Promise.all([
      this.rpc.call('eth_getCode', [request.context.account, 'latest']),
      this.rpc.call('eth_getCode', [ACCOUNT_IMPLEMENTATION, 'latest']),
    ]);
    await this.assertContext(request);
    if (!this.wasm.accountSigningSupport(accountCode, implementationCode)) throw refusal('typedUnsupported', 4200);
  }

  prepared(typed, context) {
    let result;
    try { result = JSON.parse(this.wasm.prepareTypedMessage(hex.dec(context.publicKey), JSON.stringify(typed), BigInt(context.chainId))); }
    catch { throw refusal('malformedTyped', -32602); }
    if (!ADDRESS.test(result.account) || result.account.toLowerCase() !== context.account.toLowerCase() || Number(result.chain_id) !== context.chainId || !object(result.typed_data) || !/^0x[0-9a-fA-F]{64}$/.test(result.digest_hex)) throw refusal('malformedTyped', -32602);
    return result;
  }

  async prepareTyped(origin, params) {
    const context = await this.context(origin), typed = normalizeTypedRequest(params, context.account, context.chainId);
    const request = { origin, kind: 'typed', context };
    await this.supported(request);
    const prepared = this.prepared(typed, context);
    const canonical = normalizeTypedRequest([context.account, prepared.typed_data], context.account, context.chainId);
    return { ...request, typed_data: canonical, fields: typedFieldView(canonical), domainFields: typedFieldView(canonical, 'EIP712Domain', canonical.domain),
      typedKey: stable(canonical), digest: prepared.digest_hex, previewId: crypto.randomUUID() };
  }

  async signTyped(request, { previewId } = {}) {
    await this.assertContext(request);
    if (previewId !== request.previewId || stable(request.typed_data) !== request.typedKey) throw refusal('previewChanged');
    await this.supported(request);
    const prepared = this.prepared(normalizeTypedRequest([request.context.account, request.typed_data], request.context.account, request.context.chainId), request.context);
    if (stable(prepared.typed_data) !== request.typedKey || prepared.digest_hex !== request.digest) throw refusal('previewChanged');
    await this.assertContext(request);
    const signature = await this.vault.sign(hex.dec(prepared.signing_message));
    await this.supported(request); // do not release a signature after revocation
    const wrapped = this.wasm.attachTypedSignature(JSON.stringify(request.typed_data), BigInt(request.context.chainId), request.context.account, signature, hex.dec(request.context.publicKey));
    const result = wrapped.startsWith('0x') ? wrapped : `0x${wrapped}`;
    if (!/^0x[0-9a-fA-F]{256}$/.test(result)) throw refusal('malformedTyped', -32603);
    return result;
  }
}
