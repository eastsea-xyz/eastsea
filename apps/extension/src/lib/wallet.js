// Build, sign and send: the node gives status and nonce, wasm builds the
// envelope (the chain's own rules), the vault signs, wasm checks and attaches.

import { hex } from './vault.js';
import { CHAIN_ID } from './methods.js';

export class Wallet {
  /** `wasm`: {prepareTx, attachSignature}; `rpc`: Rpc; `vault`: Vault. */
  constructor({ wasm, rpc, vault }) {
    this.wasm = wasm;
    this.rpc = rpc;
    this.vault = vault;
    this.lastNonce = null; // {address, nonce}: lets two quick sends not reuse a nonce
    this.queue = Promise.resolve(); // sends run one at a time, so nonces never collide
  }

  async nonceFor(address) {
    const chain = parseInt(await this.rpc.call('eth_getTransactionCount', [address]), 16);
    const next = this.lastNonce && this.lastNonce.address === address ? this.lastNonce.nonce + 1 : 0;
    return Math.max(chain, next);
  }

  /** Estimated worst-case fee in wei for `gas` at the current base fee (display only). */
  static maxFee(status, gas) {
    const g = BigInt(gas);
    const exec = BigInt(status?.base_fee?.exec ?? '1000000000') * 2n + 1_000_000_000n;
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

  async sendNow(tx, { status: shown } = {}) {
    const info = await this.vault.info();
    if (!info) throw new Error('No wallet yet.');
    const status = shown || (await this.rpc.call('aether_status', []));
    const nonce = await this.nonceFor(info.address);
    const prepared = JSON.parse(this.wasm.prepareTx(hex.dec(info.publicKey), JSON.stringify(status), BigInt(CHAIN_ID), BigInt(nonce), JSON.stringify(tx)));
    const sig = await this.vault.sign(hex.dec(prepared.signing_message));
    const env = JSON.parse(this.wasm.attachSignature(JSON.stringify(prepared.envelope), sig, hex.dec(info.publicKey)));
    const r = await this.rpc.call('aether_sendTransaction', [env]);
    this.lastNonce = { address: info.address, nonce };
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
