// window.aether: an EIP-1193 provider for Aether pages, also announced through
// EIP-6963 so wallet pickers list it. It does not take over window.ethereum:
// Aether accounts are P-256 accounts and sign Aether envelopes.
(() => {
  if (window.aether && window.aether.isAether) return;
  const TO_CONTENT = 'aether:to-content';
  const TO_PAGE = 'aether:to-page';
  const pending = new Map();
  const listeners = new Map();
  let seq = 0;

  function emit(event, data) {
    for (const fn of listeners.get(event) || []) {
      try { fn(data); } catch (e) { setTimeout(() => { throw e; }); }
    }
  }

  window.addEventListener('message', (e) => {
    if (e.source !== window || !e.data || e.data.tag !== TO_PAGE) return;
    const m = e.data;
    if (m.event) return emit(m.event, m.data);
    const p = pending.get(m.id);
    if (!p) return;
    pending.delete(m.id);
    if (m.error) p.reject(Object.assign(new Error(m.error.message), { code: m.error.code, data: m.error.data }));
    else p.resolve(m.result);
  });

  const provider = {
    isAether: true,
    chainId: '0x1e64',
    request({ method, params } = {}) {
      if (typeof method !== 'string') return Promise.reject(Object.assign(new Error('method must be a string'), { code: -32600 }));
      const id = `aether-${Date.now().toString(36)}-${(seq += 1)}`;
      return new Promise((resolve, reject) => {
        pending.set(id, { resolve, reject });
        window.postMessage({ tag: TO_CONTENT, id, method, params: Array.isArray(params) ? params : [] }, window.location.origin);
      });
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
})();
