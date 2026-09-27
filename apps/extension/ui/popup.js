// Popup (toolbar button) and approval window (?approve=<id>). Every action goes
// to the service worker; this page never holds the key. Page-supplied text
// (origins, call data) is only ever set as text, never as HTML.

import { aethToWei, formatAeth, shortAddress, weiToAeth } from '../src/lib/units.js';

const params = new URLSearchParams(location.search);
const approveId = params.get('approve');
if (approveId) document.body.classList.add('window');
const app = document.getElementById('app');
let tab = 'home';

async function op(name, args) {
  const r = await chrome.runtime.sendMessage({ op: name, args });
  if (!r) throw new Error('The wallet did not answer. Try again.');
  if (!r.ok) throw new Error(r.error);
  return r.result;
}

function h(tag, props = {}, ...children) {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(props || {})) {
    if (v == null || v === false) continue;
    if (k.startsWith('on')) el.addEventListener(k.slice(2).toLowerCase(), v);
    else if (k === 'class') el.className = v;
    else el.setAttribute(k, v === true ? '' : v);
  }
  for (const c of children.flat()) if (c != null && c !== false) el.append(c instanceof Node ? c : String(c));
  return el;
}

function header(extra) {
  return h('header', {}, h('div', { class: 'logo', 'aria-hidden': 'true' }), h('h1', {}, 'Aether Wallet'), extra);
}

function message(kind, text) {
  return h('div', { class: `msg ${kind}`, role: kind === 'error' ? 'alert' : 'status' }, text);
}

/** Run `fn` from a form button: disable it, show errors under the form. */
function action(button, out, fn) {
  return async (e) => {
    e?.preventDefault();
    button.disabled = true;
    out.replaceChildren();
    try {
      await fn();
    } catch (err) {
      out.replaceChildren(message('error', err.message || String(err)));
    } finally {
      button.disabled = false;
    }
  };
}

function render(...nodes) {
  app.replaceChildren(...nodes.flat().filter((n) => n != null && n !== false));
}

// ---- onboarding ----

function onboarding() {
  const out = h('div');
  const pw = h('input', { type: 'password', autocomplete: 'new-password', placeholder: 'At least 8 characters', required: true });
  const pw2 = h('input', { type: 'password', autocomplete: 'new-password', required: true });
  const create = h('button', { class: 'primary', type: 'submit' }, 'Create wallet');
  const secret = h('textarea', { placeholder: '64 hex characters', spellcheck: 'false' });
  const importBtn = h('button', { type: 'button' }, 'Import a private key');
  const check = () => {
    if (pw.value !== pw2.value) throw new Error('The passwords do not match.');
  };
  const form = h('form', { class: 'card' },
    h('h2', {}, 'Create a wallet'),
    h('p', { class: 'muted small' }, 'A new key is made in this browser and encrypted with your password. It works without the Aether app. This is the testnet: test AETH has no value.'),
    h('label', {}, 'Password', pw),
    h('label', {}, 'Password again', pw2),
    create,
    h('details', {}, h('summary', { class: 'small muted' }, 'Import an existing key instead'),
      h('div', { class: 'list', style: 'margin-top:8px' }, h('label', {}, 'Private key', secret), importBtn)),
    out,
  );
  form.addEventListener('submit', action(create, out, async () => { check(); await op('create', { password: pw.value }); refresh(); }));
  importBtn.addEventListener('click', action(importBtn, out, async () => { check(); await op('importKey', { secret: secret.value, password: pw.value }); refresh(); }));
  render(header(), form);
  pw.focus();
}

function unlockView(address) {
  const out = h('div');
  const pw = h('input', { type: 'password', autocomplete: 'current-password', required: true });
  const btn = h('button', { class: 'primary', type: 'submit' }, 'Unlock');
  const form = h('form', { class: 'card' }, h('h2', {}, 'Unlock'), h('div', { class: 'mono muted' }, address), h('label', {}, 'Password', pw), btn, out);
  form.addEventListener('submit', action(btn, out, async () => { await op('unlock', { password: pw.value }); refresh(); }));
  render(header(h('span', { class: 'pill' }, 'Locked')), form);
  pw.focus();
}

// ---- approval window ----

async function approvalView(s) {
  const a = s.approvals.find((x) => x.id === approveId);
  if (!a) {
    render(header(), h('div', { class: 'card' }, h('p', {}, 'This request was already answered or has expired.'), h('button', { onclick: () => window.close() }, 'Close')));
    return;
  }
  const out = h('div');
  let retryFee = null;
  const yes = h('button', { class: 'primary' }, a.kind === 'connect' ? 'Connect' : 'Approve');
  const no = h('button', {}, 'Reject');
  const body = [h('div', { class: 'small muted' }, a.kind === 'connect' ? 'This site asks to see your address' : 'This site asks you to send a transaction'), h('div', { class: 'origin' }, a.origin)];
  if (a.kind === 'connect') {
    body.push(h('p', { class: 'small muted' }, 'It will see your address and balance, and can ask for transactions. Every transaction still needs your approval.'),
      h('div', { class: 'kv' }, h('span', {}, 'Account'), h('span', { class: 'mono' }, s.address)));
  } else {
    const fee = h('span', { class: 'muted' }, '…');
    // The same status snapshot is used for signing, so this is the cap that gets signed.
    const loadFee = () => op('quote', { id: approveId }).then((w) => fee.replaceChildren(`up to ${formatAeth(w, 6)} AETH`)).catch((e) => fee.replaceChildren(`unknown (${e.message})`));
    loadFee();
    retryFee = loadFee;
    body.push(h('div', { class: 'kv' },
      h('span', {}, 'Action'), h('strong', {}, a.what),
      h('span', {}, 'To'), h('span', { class: 'mono' }, a.tx.to || '(new contract)'),
      h('span', {}, 'Sends'), h('strong', {}, `${a.value} AETH`),
      h('span', {}, 'Network fee'), fee,
      h('span', {}, 'From'), h('span', { class: 'mono' }, s.address)));
    if (a.tx.data !== '0x') body.push(h('details', {}, h('summary', { class: 'small muted' }, 'Call data'), h('div', { class: 'mono muted', style: 'max-height:120px;overflow:auto' }, a.tx.data)));
    if (a.what.startsWith('Token approval')) body.push(h('div', { class: 'warn' }, 'An approval lets the contract move your tokens later. Approve only contracts you trust.'));
  }
  yes.addEventListener('click', action(yes, out, async () => {
    const r = await op('approve', { id: approveId }).catch((e) => { retryFee?.(); throw e; });
    render(header(), h('div', { class: 'card' }, message('ok', r.hash ? `Sent. Transaction ${shortAddress(r.hash)}` : 'Connected.'), r.hash ? h('div', { class: 'mono muted' }, r.hash) : null));
    setTimeout(() => window.close(), r.hash ? 1400 : 600);
  }));
  no.addEventListener('click', action(no, out, async () => { await op('reject', { id: approveId }); window.close(); }));
  render(header(h('span', { class: 'pill' }, 'Testnet')), h('div', { class: 'card' }, ...body), h('div', { class: 'row' }, h('div', { class: 'grow' }), no, yes), out);
}

// ---- main popup ----

function nav() {
  const b = (id, label) => h('button', { 'aria-current': tab === id ? 'page' : null, onclick: () => { tab = id; refresh(); } }, label);
  return h('nav', { 'aria-label': 'Sections' }, b('home', 'Home'), b('activity', 'Activity'), b('sites', 'Sites'), b('settings', 'Settings'));
}

async function home(s) {
  const out = h('div');
  const bal = h('div', { class: 'balance' }, '…');
  const node = h('span', { class: 'pill' }, 'Connecting…');
  const addr = h('button', { class: 'link mono', title: 'Copy address', onclick: async () => { await navigator.clipboard.writeText(s.address); addr.textContent = 'Copied'; setTimeout(() => { addr.textContent = shortAddress(s.address); }, 900); } }, shortAddress(s.address));
  op('account').then((a) => { bal.textContent = `${formatAeth(a.balance)} AETH`; node.textContent = `Block ${a.height}`; node.classList.add('good'); })
    .catch((e) => { bal.textContent = '—'; node.textContent = 'No node'; out.replaceChildren(message('error', e.message)); });

  const to = h('input', { placeholder: '0x… recipient', spellcheck: 'false' });
  const amount = h('input', { placeholder: 'Amount in AETH', inputmode: 'decimal' });
  const sendBtn = h('button', { class: 'primary', type: 'submit' }, 'Send');
  const sendForm = h('form', { class: 'card', hidden: true }, h('h2', {}, 'Send AETH'), h('label', {}, 'To', to), h('label', {}, 'Amount', amount), sendBtn);
  sendForm.addEventListener('submit', action(sendBtn, out, async () => {
    const wei = aethToWei(amount.value);
    const r = await op('send', { to: to.value.trim(), value_wei: wei.toString() });
    out.replaceChildren(message('ok', `Sent ${weiToAeth(wei)} AETH · ${shortAddress(r.hash)}`));
    sendForm.hidden = true;
  }));
  const faucet = h('button', {}, h('span', { class: 'ico' }, '💧'), 'Get test AETH');
  faucet.addEventListener('click', action(faucet, out, async () => {
    const r = await op('faucet');
    out.replaceChildren(message('ok', `Faucet sent · ${shortAddress(r.hash)}. The balance updates when it is final.`));
    setTimeout(refresh, 2500);
  }));
  const receive = h('button', { onclick: () => navigator.clipboard.writeText(s.address).then(() => out.replaceChildren(message('ok', 'Address copied.'))) }, h('span', { class: 'ico' }, '⬇'), 'Receive');
  const send = h('button', { onclick: () => { sendForm.hidden = !sendForm.hidden; if (!sendForm.hidden) to.focus(); } }, h('span', { class: 'ico' }, '↗'), 'Send');
  return [h('div', { class: 'card hero' }, h('div', { class: 'row', style: 'justify-content:center' }, addr, node), bal, h('div', { class: 'small muted' }, 'Aether testnet')),
    h('div', { class: 'actions' }, receive, send, faucet), sendForm, out];
}

async function activity() {
  const list = await op('activity');
  if (!list.length) return [h('div', { class: 'card' }, h('p', { class: 'muted' }, 'No transactions yet.'))];
  return [h('div', { class: 'card list' }, list.map((a) => h('div', { class: 'item' },
    h('span', { class: `dot ${a.state}`, title: a.state }),
    h('div', { class: 'grow' }, h('div', {}, a.title), h('div', { class: 'small muted' }, `${a.origin} · ${new Date(a.at).toLocaleString()}`), h('div', { class: 'mono muted', title: a.hash }, shortAddress(a.hash))),
    a.value && a.value !== '0' ? h('div', { class: 'small' }, `${formatAeth(a.value)} AETH`) : null)))];
}

async function sitesView() {
  const s = await op('sites');
  const entries = Object.entries(s);
  if (!entries.length) return [h('div', { class: 'card' }, h('p', { class: 'muted' }, 'No sites are connected.'))];
  return [h('div', { class: 'card list' }, entries.map(([origin, e]) => h('div', { class: 'item' },
    h('div', { class: 'grow' }, h('div', { class: 'mono' }, origin), h('div', { class: 'small muted' }, `since ${new Date(e.at).toLocaleDateString()}`)),
    h('button', { class: 'danger', onclick: async () => { await op('disconnect', { origin }); refresh(); } }, 'Disconnect'))))];
}

function settingsView(s) {
  const out = h('div');
  const minutes = h('input', { type: 'number', min: 1, max: 1440, value: s.lockMinutes });
  const rpcs = h('textarea', { placeholder: 'https://node.example (one per line)', spellcheck: 'false' }, s.rpcs.join('\n'));
  const save = h('button', { type: 'submit' }, 'Save');
  const form = h('form', { class: 'card' }, h('h2', {}, 'Settings'), h('label', {}, 'Lock after (minutes)', minutes),
    h('label', {}, 'Extra nodes, tried before the defaults', rpcs), h('p', { class: 'small muted' }, 'Defaults: the Aether app\'s node on this computer (127.0.0.1:18545), then this Mac\'s testnet validators.'), save);
  form.addEventListener('submit', action(save, out, async () => {
    const list = rpcs.value.split(/\s+/).filter(Boolean);
    const extra = list.filter((u) => !/^http:\/\/(127\.0\.0\.1|localhost)[:/]/.test(u));
    if (extra.length && !(await chrome.permissions.request({ origins: extra.map((u) => `${new URL(u).origin}/*`) }))) throw new Error('The browser did not allow those nodes.');
    await op('settings', { lockMinutes: minutes.value, rpcs: list });
    out.replaceChildren(message('ok', 'Saved.'));
  }));

  const pw = h('input', { type: 'password', autocomplete: 'current-password', placeholder: 'Password' });
  const reveal = h('button', { type: 'button' }, 'Show private key');
  const erase = h('button', { type: 'button', class: 'danger' }, 'Remove wallet from this browser');
  const keyOut = h('div');
  reveal.addEventListener('click', action(reveal, keyOut, async () => {
    const k = await op('reveal', { password: pw.value });
    keyOut.replaceChildren(h('div', { class: 'warn' }, 'Anyone with this key controls the account. Store it offline.'), h('div', { class: 'mono' }, k));
  }));
  erase.addEventListener('click', action(erase, keyOut, async () => {
    if (!confirm('Remove the key from this browser? Without a backup of the private key, the account is lost.')) return;
    await op('erase', { password: pw.value });
    refresh();
  }));
  const backup = h('div', { class: 'card' }, h('h2', {}, 'Backup'), h('p', { class: 'small muted' }, 'The key lives only in this browser. Keep a copy of the private key somewhere safe.'), pw, h('div', { class: 'row' }, reveal, erase), keyOut);
  return [form, out, backup];
}

async function refresh() {
  let s;
  try {
    s = await op('state');
  } catch (e) {
    render(header(), message('error', e.message));
    return;
  }
  if (!s.exists) return onboarding();
  if (!s.unlocked) return unlockView(s.address);
  if (approveId) return approvalView(s);
  const lock = h('button', { class: 'link small', onclick: async () => { await op('lock'); refresh(); } }, 'Lock');
  const pendingNote = s.approvals.length ? h('div', { class: 'warn' }, `${s.approvals.length} request(s) waiting in their approval window.`) : null;
  const views = { home, activity, sites: sitesView, settings: settingsView };
  render(header(lock), nav(), pendingNote, ...(await views[tab](s)));
}

refresh();
