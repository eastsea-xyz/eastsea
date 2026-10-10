import { mountLiveGlobe } from './live-globe/live-globe.js';

const root = document.getElementById('live-network-globe');
let config = {};
try {
  config = await (await fetch('./live-network.json', { credentials: 'omit', referrerPolicy: 'no-referrer' })).json();
} catch { /* The component displays an unavailable state and retries. */ }
const fixture = new URLSearchParams(location.search).get('globe') === 'fixture';
const globe = mountLiveGlobe(root, {
  endpoint: config.rpc,
  fixture,
  lang: document.documentElement.lang,
  ...(fixture ? { seed: 'fixture-smoke' } : {}),
});
new MutationObserver(() => globe.setLanguage(document.documentElement.lang))
  .observe(document.documentElement, { attributes: true, attributeFilter: ['lang'] });
window.addEventListener('pagehide', event => { if (!event.persisted) globe.destroy(); });
