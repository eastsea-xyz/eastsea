// Build, sign and send: the node gives status and nonce, wasm builds the
// envelope (the chain's own rules), the vault signs, wasm checks and attaches.

import { hex } from './vault.js';
import { LEGACY_TESTNET_CHAIN_ID } from './brand.js';

/** Shown when a paid-state chain would reject the send for want of coins. */
export const ADD_COINS_FIRST = 'Add native coins before sending: this transaction must pay for its persistent bytes and first-use account';

/**
 * Every chain but the legacy 7780 testnet charges for persistent state, as the
 * app's FFI decides (`state_price_for`). There a zero balance signs a state
 * budget of 0, which the chain rejects (A6-2), so refuse before signing.
 * `balance` is null when it could not be read.
 */
export function checkPaidStateBalance(chainId, balance) {
  if (Number(chainId) === LEGACY_TESTNET_CHAIN_ID) return;
  if (balance === null) throw new Error("Could not read the balance needed to pay this transaction's state fee");
  if (balance === 0n) throw new Error(ADD_COINS_FIRST);
}

export class Wallet {
  /** `wasm`: {prepareTx, attachSignature}; `rpc`: Rpc; `vault`: Vault. */
  constructor({ wasm, rpc, vault }) {
    this.wasm = wasm;
    this.rpc = rpc;
    this.vault = vault;
    this.lastNonce = null; // {address, nonce, chainId}: lets two quick sends not reuse a nonce
    this.queue = Promise.resolve(); // sends run one at a time, so nonces never collide
  }

  async nonceFor(address) {
    const chain = parseInt(await this.rpc.call('eth_getTransactionCount', [address]), 16);
    const next = this.lastNonce && this.lastNonce.address === address && this.lastNonce.chainId === this.rpc.chainId ? this.lastNonce.nonce + 1 : 0;
    return Math.max(chain, next);
  }

  /** Estimated worst-case fee in wei for `gas` at the current base fee (display only):
   *  0 while the base fee is 0 (below target load the network is free). */
  static maxFee(status, gas) {
    const g = BigInt(gas);
    const base = BigInt(status?.base_fee?.exec ?? '1000000000');
    const exec = base === 0n ? 0n : base * 2n + 1_000_000_000n;
    const prove = BigInt(status?.base_fee?.prove ?? '1000000000') * 2n;
    return g * exec + g * prove;
  }

  /**
   * `tx`: normalized ({to, value_wei, data, gas}). `status`: the `aether_status`
   * snapshot whose fee caps the user saw (fetched now if not given). Returns the hash.
   */
  send(tx, opts = {}) {
    const run = this.queue.then(() => this.sendNow(tx, opts));
    this.queue = run.catch(() => {});
    return run;
  }

  async sendNow(tx, { status: shown, beforeSign, afterSign } = {}) {
    const info = await this.vault.info();
    if (!info) throw new Error('No wallet yet.');
    const chainId = this.rpc.chainId;
    const status = shown || (await this.rpc.call('aether_status', []));
    const nonce = await this.nonceFor(info.address);
    // The balance decides the tip (G2): a zero balance sends with none, so a
    // new account can transact while the base fee is 0.
    const read = await this.balance(info.address).catch(() => null);
    if (Number(status.chain_id) !== chainId || this.rpc.chainId !== chainId) throw new Error('The node is on another chain.');
    checkPaidStateBalance(chainId, read);
    const balance = read ?? 0n;
    const prepared = JSON.parse(this.wasm.prepareTx(hex.dec(info.publicKey), JSON.stringify(status), BigInt(chainId), BigInt(nonce), JSON.stringify({ ...tx, balance_wei: balance.toString() })));
    // A queued dApp send may have waited while its account, permission or
    // simulated effects changed. Recheck at the actual signing boundary.
    await beforeSign?.();
    const sig = await this.vault.sign(hex.dec(prepared.signing_message));
    await afterSign?.();
    if (this.rpc.chainId !== chainId) throw new Error('The wallet network changed before the transaction was sent.');
    const env = JSON.parse(this.wasm.attachSignature(JSON.stringify(prepared.envelope), sig, hex.dec(info.publicKey)));
    const r = await this.rpc.call('aether_sendTransaction', [env]);
    this.lastNonce = { address: info.address, nonce, chainId };
    return r.hash;
  }

  /** Wait for finality: {ok, height, gasUsed} or null after `timeoutMs`. */
  async receipt(hash, { timeoutMs = 30_000, every = 500 } = {}) {
    const end = Date.now() + timeoutMs;
    while (Date.now() < end) {
      const r = await this.rpc.call('aether_getReceipt', [hash]).catch(() => null);
      if (r && r.receipt) return { ok: Boolean(r.receipt.success), height: r.height, gasUsed: r.receipt.gas_used };
      await new Promise((res) => setTimeout(res, every));
    }
    return null;
  }

  async balance(address) {
    return BigInt(await this.rpc.call('eth_getBalance', [address, 'latest']));
  }

  /** The testnet faucet (the node decides the amount and rate limit). */
  async faucet(address) {
    const r = await this.rpc.callAny('aether_faucet', [address]);
    return r.hash;
  }
}
