// Relay between the page's provider (inpage.js, page world) and the service
// worker. Only messages from this window with our tag pass.
(() => {
  const TO_CONTENT = 'aether:to-content';
  const TO_PAGE = 'aether:to-page';
  let port = null;
  const waiting = new Set();

  function connect() {
    port = chrome.runtime.connect({ name: 'aether-page' });
    port.onMessage.addListener((m) => {
      if (m && m.id) waiting.delete(m.id);
      window.postMessage({ tag: TO_PAGE, ...m }, window.location.origin);
    });
    port.onDisconnect.addListener(() => {
      port = null;
      // The worker restarted mid-request: tell the page instead of hanging.
      for (const id of waiting) window.postMessage({ tag: TO_PAGE, id, error: { code: 4900, message: 'EastSea Wallet restarted; please try again.' } }, window.location.origin);
      waiting.clear();
    });
  }

  window.addEventListener('message', (e) => {
    if (e.source !== window || !e.data || e.data.tag !== TO_CONTENT) return;
    const { id, method, params } = e.data;
    if (typeof id !== 'string' || typeof method !== 'string') return;
    try {
      if (!port) connect();
      waiting.add(id);
      port.postMessage({ id, method, params });
    } catch {
      window.postMessage({ tag: TO_PAGE, id, error: { code: 4900, message: 'EastSea Wallet is not available (was it updated? reload the page).' } }, window.location.origin);
    }
  });

  // Connect once so the page hears accountsChanged even before it asks anything.
  try { connect(); } catch { /* extension context gone */ }
})();
