import { KNOWN_TOKENS } from '../../../../../apps/extension/src/lib/knownTokens.js';

const params = new URLSearchParams(location.search);
document.documentElement.dataset.theme = params.get('theme') === 'dark' ? 'dark' : 'light';
const view = ['home', 'assets', 'activity', 'sites', 'settings'].includes(params.get('view')) ? params.get('view') : 'home';
const address = '0x111122223333444455556666777788889999AaAa';
const balance = '12480320000000000000000';
const hidden = new Set();
const shown = new Set();
let unlocked = true;
const amounts = { WAETH: '175000000000000000000', NEB: '4200000000000000000000', ORB: '60000000000000000000', CMT: '800000000000000000000' };
const holdings = Object.entries(KNOWN_TOKENS[7780]).map(([contract, token]) => ({ token: { ...token, address: contract, trusted: true, origin: 'official' }, balance: amounts[token.symbol] }));
const unseen = [{ token: { address: '0x777e11112222333344445555666677778888a40b', symbol: 'MOON', name: 'Sample Moon', decimals: 18, trusted: false, unverifiedUnits: true }, balance: '1000000000000000000000' }];
const history = [{ state: 'done', title: 'Received DBLN', origin: 'Sample transfer', at: Date.UTC(2026, 9, 8, 3, 20), hash: `0x${'a'.repeat(64)}`, value: '1250000000000000000' }];

async function respond(op, args = {}) {
  switch (op) {
    case 'state': return { terms: 4, exists: true, unlocked, address, approvals: [], developmentNetwork: false, defaultChainId: 7780, lockMinutes: 15, developerMode: false, developmentPort: 18545, rpcs: [] };
    case 'account': return { balance, height: 841372, blockAt: Date.now() };
    case 'assets': return { tokens: [...holdings.filter((x) => !hidden.has(x.token.address)), ...unseen.filter((x) => shown.has(x.token.address))], unverified: [...holdings.filter((x) => hidden.has(x.token.address)), ...unseen.filter((x) => !shown.has(x.token.address))], officialSymbols: [{ symbol: 'DBLN', name: 'Doubloon' }], updated: Date.now() };
    case 'hideToken': hidden.add(args.address); shown.delete(args.address); return {};
    case 'showToken': hidden.delete(args.address); shown.add(args.address); return {};
    case 'activity': return history;
    case 'activityPage': return { items: history, cursors: {}, starts: {} };
    case 'linkedWallets': return [];
    case 'sites': return {};
    case 'settings': return {};
    case 'lock': unlocked = false; return {};
    case 'unlock': unlocked = true; return {};
    default: throw new Error('This is a demo preview. It cannot sign, reveal a key, or change a real wallet.');
  }
}

// There is no service worker behind this fixture. All results remain local.
globalThis.chrome = {
  runtime: { async sendMessage({ op, args }) {
    try { return { ok: true, result: await respond(op, args) }; }
    catch (error) { return { ok: false, error: error.message }; }
  } },
  permissions: { async request() { return false; } },
};
Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { async writeText(text) { globalThis.eastSeaDemoCopied = text; } } });

// Navigate with the real popup's controls once its first render is available.
const app = document.getElementById('app');
let navigated = view === 'home';
const observer = new MutationObserver(() => {
  const buttons = [...app.querySelectorAll('nav button')];
  if (!buttons.length) return;
  if (!navigated) {
    navigated = true;
    buttons.find((button) => button.textContent.toLowerCase() === view)?.click();
    return;
  }
  if (view === 'assets' && params.get('unverified') === 'open') {
    const disclosure = [...app.querySelectorAll('details')].find((element) => element.querySelector('summary')?.textContent.startsWith('Unverified'));
    if (!disclosure || disclosure.hidden) return;
    disclosure.open = true;
  }
  observer.disconnect();
});
observer.observe(app, { childList: true, subtree: true });
await import('../../../../../apps/extension/ui/popup.js');
