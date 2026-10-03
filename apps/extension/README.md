# EastSea Wallet browser extension

A Manifest V3 extension for Chromium browsers (Chrome, Edge, Brave, Arc). It works **without the EastSea app**.

- **Key:** a P-256 key made by WebCrypto in the browser. At rest it is encrypted with your password (PBKDF2-SHA256, 600k iterations, AES-GCM). While unlocked it lives only in memory-only session storage, and it locks after 30 minutes by default.
- **Transactions:** built by `crates/wasm`, the same Rust rules the app and the chain use. The signature is checked against the key before anything is sent.
- **Pages:** get `window.aether`, an EIP-1193 provider, also announced through EIP-6963 (`rdns: com.pipln.aether`). It does not take over `window.ethereum`.
- **Approvals:** connecting a site and every transaction open an approval window. The site's origin comes from the browser, not from the page. Only https pages and pages served from this computer can connect.
- **Assets:** AETH plus the ERC-20 tokens the app also finds (`token-sources.json`: the DEX token factory, its pools and the launchpad, read with `eth_call`s), cached and re-read on open and every 30 s. AETH account balance and nonce are checked against a pinned committee certificate and state proof on the bundled network. Token reads are not proof checked.
- **Network paused:** the popup header shows it when the chain has made no new block for 60 s (the app's rule); a new block clears it.
- **First run:** a one-time notice with the app's terms risk points (experimental, as-is, key loss, testnet tokens have no value), kept per `TERMS_VERSION`.
- **Nodes:** nodes added in Settings are tried first, followed by the EastSea app's node on this computer (`127.0.0.1:18545`). There is no built-in remote browser seed yet.

## Build and load

```bash
scripts/build-extension.sh          # builds apps/extension/wasm
# chrome://extensions -> Developer mode -> Load unpacked -> apps/extension
scripts/build-extension.sh --zip    # dist/aether-extension-<version>.zip for the stores
```

The light-client's `blst` and `zstd-sys` C code needs a compiler with a wasm32 target. Apple's system clang cannot build this target; use a wasm-capable clang or Zig C compiler when building on macOS.

## Page API

```js
const [account] = await window.aether.request({ method: 'eth_requestAccounts' });
const hash = await window.aether.request({
  method: 'eth_sendTransaction',
  params: [{ to, data, value: '0x…' /* wei, hex */, gas: '0x…' /* optional */ }],
});
const r = await window.aether.request({ method: 'aether_getReceipt', params: [hash] }); // r.receipt.success when final
window.aether.on('accountsChanged', (accounts) => { /* [] after a disconnect */ });
```

Reads (`eth_call`, `eth_getBalance`, `eth_getLogs`, `aether_status`, …) go to the node. Errors use EIP-1193 codes: 4001 rejected, 4100 not connected, 4200 unsupported, 4900 no node.

## Tests

```bash
cd apps/extension && npm test                  # units, vault, wasm signing (no browser)
node apps/extension/test/live.mjs              # a real faucet, send and WAETH deposit on the testnet
node apps/extension/test/e2e.mjs               # the extension in Chromium (needs playwright)
node apps/extension/test/dapps-e2e.mjs         # DEX and launchpad UIs through the extension
```

## Not yet

- Other reads, including `eth_call`, status, history, and receipts, are not light-client verified in the extension. Their answers are still labeled as node data. Transactions are still signed only by you and checked by the chain.
- Browser-native follower transport and discovery require a WebTransport server and published certificate hashes; they are not yet available in this build. Without the app's node, a manually configured HTTPS RPC is required.
- Using the app's Secure Enclave key from the extension, through native messaging, is a later step.
