// Live check against a running testnet (not part of `npm test`):
//   node test/live.mjs [rpc-url] [WAETH address]
// Makes a fresh browser-style key, takes faucet AETH, sends AETH, then calls a
// contract with value (WAETH.deposit), all through the extension's own code.
import { Vault } from '../src/lib/vault.js';
import { Wallet } from '../src/lib/wallet.js';
import { Rpc } from '../src/lib/rpc.js';
import { weiToAeth } from '../src/lib/units.js';
import { loadWasm, memoryArea } from './helpers.mjs';

const [url = 'http://127.0.0.1:8601', waeth] = process.argv.slice(2);
const wasm = await loadWasm();
const vault = new Vault({ local: memoryArea(), session: memoryArea(), addressOf: wasm.accountAddress });
const rpc = new Rpc([url]);
const w = new Wallet({ wasm, rpc, vault });
const me = await vault.create('live-test-password');
console.log('account', me);

const step = async (label, hash) => {
  const r = await w.receipt(hash, { timeoutMs: 60_000 });
  console.log(label, hash, r ? (r.ok ? `ok at ${r.height}` : 'FAILED') : 'not final');
  if (!r || !r.ok) process.exit(1);
};

await step('faucet', await w.faucet(me));
console.log('balance', weiToAeth(await w.balance(me)));
await step('send 0.1 AETH', await w.send({ to: '0x000000000000000000000000000000000000dEaD', value_wei: '100000000000000000', data: '0x', gas: 0 }));
if (waeth) {
  await step('WAETH.deposit 0.2', await w.send({ to: waeth, value_wei: '200000000000000000', data: '0xd0e30db0', gas: 100000 }));
  const bal = await rpc.call('eth_call', [{ to: waeth, data: `0x70a08231${me.slice(2).toLowerCase().padStart(64, '0')}` }, 'latest']);
  console.log('WAETH balance', weiToAeth(BigInt(bal)));
  if (BigInt(bal) !== 200000000000000000n) process.exit(1);
}
console.log('balance', weiToAeth(await w.balance(me)));
