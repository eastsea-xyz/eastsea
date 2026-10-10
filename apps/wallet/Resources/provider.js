// window.aether inside the Explore tab: the EIP-1193 surface the browser
// extension injects (apps/extension/src/inpage.js), answered by the wallet
// itself over WebKit's message bridge instead of a content script. The
// method set is the extension's (apps/extension/src/lib/methods.js);
// apps/extension/test/wallet-provider.test.mjs asserts the two stay equal.
// It does not take over window.ethereum: EastSea accounts are P-256 accounts
// and sign EastSea envelopes.
(() => {
  if (window.aether && window.aether.isAether) return;
  const SUPPORTED = new Set([
    // methods.js READ_METHODS — reads; nothing here can move funds.
    'eth_blockNumber', 'eth_call', 'eth_estimateGas', 'eth_getBalance', 'eth_getCode',
    'eth_getLogs', 'eth_getStorageAt', 'eth_getTransactionCount', 'eth_gasPrice',
    'net_version', 'aether_status', 'aether_getReceipt', 'aether_getAccount', 'aether_accountHistory',
    // methods.js ACCOUNT_METHODS — the address, only through a sheet.
    'eth_requestAccounts', 'aether_requestAccounts', 'eth_accounts', 'aether_accounts',
    // methods.js SEND_METHODS — a transfer, only through a sheet.
    'eth_sendTransaction', 'aether_sendTransaction',
    'eth_signTypedData_v4',
    // answered directly, as the extension's background does.
    'eth_chainId', 'wallet_disconnect', 'aether_disconnect',
  ]);
  const listeners = new Map();
  const bridge = () => window.webkit && window.webkit.messageHandlers && window.webkit.messageHandlers.aether;
  const err = (code, message) => Object.assign(new Error(message), { code });
  let seq = 0;

  const provider = {
    isAether: true,
    chainId: '0x1e64',
    supportedMethods: [...SUPPORTED],
    request({ method, params } = {}) {
      if (typeof method !== 'string') return Promise.reject(err(-32600, 'method must be a string'));
      if (!SUPPORTED.has(method)) return Promise.reject(err(4200, `EastSea Wallet does not support ${method}.`));
      const handler = bridge();
      if (!handler) return Promise.reject(err(4100, 'This page is not talking to the EastSea wallet provider.'));
      const id = `aether-${Date.now().toString(36)}-${(seq += 1)}`;
      return handler.postMessage({ id, method, params: Array.isArray(params) ? params : [] }).then(
        (m) => {
          if (m && m.error) return Promise.reject(err(m.error.code ?? -32603, m.error.message || 'the wallet refused the request'));
          return m ? m.result : undefined;
        },
        (e) => Promise.reject(err(-32603, (e && e.message) || 'the wallet did not answer')),
      );
    },
    on(event, fn) {
      if (!listeners.has(event)) listeners.set(event, new Set());
      listeners.get(event).add(fn);
      return provider;
    },
    removeListener(event, fn) {
      listeners.get(event)?.delete(fn);
      return provider;
    },
  };
  Object.freeze(provider);
  Object.defineProperty(window, 'aether', { value: provider, writable: false, configurable: false });

  const info = Object.freeze({
    uuid: crypto.randomUUID(),
    name: 'EastSea Wallet',
    rdns: 'com.pipln.aether',
    icon: 'data:image/svg+xml;base64,' + btoa('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32"><defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#7d66f2"/><stop offset="1" stop-color="#ec4899"/></linearGradient></defs><circle cx="16" cy="16" r="15" fill="url(#g)"/></svg>'),
  });
  const announce = () => window.dispatchEvent(new CustomEvent('eip6963:announceProvider', { detail: Object.freeze({ info, provider }) }));
  window.addEventListener('eip6963:requestProvider', announce);
  announce();
  window.dispatchEvent(new Event('aether#initialized'));

  // window.eastsea: the verification surface only the app's own pages get a
  // working answer from. block / account / receipt resolve with the wallet's
  // native verdict {verified, height?, reason} — the Rust verifier, ≈0.68 ms
  // a committee certificate where the wasm module a public page loads takes
  // ≈11.7 (docs/research/wasm-speed-2026-10-05.md). Its own handler name so
  // the provider surface above keeps meaning one thing. The origin rule
  // (VerifyBridge.allows) is enforced on the Swift side: a page that is not
  // bundled and not connected gets a rejection, never a verdict.
  (() => {
    if (window.eastsea && window.eastsea.verify) return;
    const handler = () => window.webkit && window.webkit.messageHandlers && window.webkit.messageHandlers.eastsea;
    const err = (code, message) => Object.assign(new Error(message), { code });
    let seq = 0;
    const ask = (what, param) => {
      const h = handler();
      if (!h) return Promise.reject(err(4100, 'This page is not talking to the EastSea wallet.'));
      const id = `eastsea-${Date.now().toString(36)}-${(seq += 1)}`;
      return h.postMessage({ id, what, param }).then(
        (m) => {
          if (m && m.error) return Promise.reject(err(m.error.code ?? -32603, m.error.message || 'the wallet refused the request'));
          return m ? m.result : undefined;
        },
        (e) => Promise.reject(err(-32603, (e && e.message) || 'the wallet did not answer')),
      );
    };
    const eastsea = Object.freeze({
      verify: Object.freeze({
        block: (height) => ask('block', height),
        account: (address) => ask('account', address),
        receipt: (txHash) => ask('receipt', txHash),
      }),
    });
    Object.defineProperty(window, 'eastsea', { value: eastsea, writable: false, configurable: false });
  })();
})();
