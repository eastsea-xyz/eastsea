import { mountLiveGlobe } from './live-globe/live-globe.js';

const TITLES = { en: 'Network', ko: '네트워크', ja: 'ネットワーク', 'zh-Hans': '网络', 'zh-Hant': '網路', es: 'Red' };

// Inbound-only interface: the wallet calls these functions in WKWebView.
// This document has no network access or JavaScript-to-native message handlers.
export function installWalletHost(root) {
  const view = mountLiveGlobe(root, { host: true });
  const document = root.ownerDocument;
  const api = Object.freeze({
    ready: true,
    update(presence) { return view.update(presence); },
    reset() { return view.reset(); },
    captureFrame() { return view.captureFrame(); },
    configure(options) {
      if (typeof options?.searchHome === 'boolean') {
        document.body.classList.toggle('search-home', options.searchHome);
      }
      const configured = view.configure(options);
      if (configured) {
        document.documentElement.lang = root.lang;
        document.title = `${TITLES[root.lang]} · EastSea`;
      }
      return configured;
    },
    height() {
      const body = document.defaultView.getComputedStyle(document.body);
      return Math.ceil(root.getBoundingClientRect().height
        + (Number.parseFloat(body.paddingTop) || 0) + (Number.parseFloat(body.paddingBottom) || 0));
    },
  });
  root.dataset.ready = 'true';
  return api;
}

if (typeof document !== 'undefined') {
  const root = document.getElementById('globe');
  if (root) globalThis.eastseaGlobe = installWalletHost(root);
}
