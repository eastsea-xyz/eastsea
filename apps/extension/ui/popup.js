import { Brand, coinTicker, coinName } from '../src/lib/brand.js';
// Popup (toolbar button) and approval window (?approve=<id>). Every action goes
// to the service worker; this page never holds the key. Page-supplied text
// (origins, call data) is only ever set as text, never as HTML.

import { aethToWei, formatAeth, shortAddress, weiToAeth } from '../src/lib/units.js';
import { erc20TransferCalldata, formatTokenAmount, formatTokenAmountExact, grouped } from '../src/lib/tokens.js';
import { buildSendIntent } from '../src/lib/sendIntent.js';
import { addressRisk, looksLikeOfficial, tokenLabel, tokenShort } from '../src/lib/safety.js';
import { nextPauseState, pausedLine, PAUSE_HELP } from '../src/lib/pause.js';
import { TERMS_VERSION, DISCLAIMER_URL, noticePoints } from '../src/lib/terms.js';
import { mergeHistory } from '../src/lib/history.js';
import { displayTokenName } from '../src/lib/knownTokens.js';
import { tokenArtSource, tokenFallbackAppearance } from './token-art.js';
import { t, language } from '../src/lib/i18n.js';
import { browserInput, externalNameMessage, suggestedHTTPS } from '../src/lib/sea-url.mjs';

const params = new URLSearchParams(location.search);
const approveId = params.get('approve');
if (approveId) document.body.classList.add('window');
if (approveId) document.documentElement.lang = language();
let approvalBusy = false;
const app = document.getElementById('app');
let tab = 'home';
let developmentNetwork = false;
let defaultChainId = 7780;
let pauseState = null; // the network-pause tracker, for as long as this popup is open
let updaters = []; // what the 15 s poll re-runs while the popup is open
setInterval(() => { for (const u of updaters) Promise.resolve().then(u).catch(() => {}); }, 15_000);

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
  if (tag === 'button') el.classList.add('es-control');
  if (tag === 'h2') el.classList.add('es-section-title');
  for (const c of children.flat()) if (c != null && c !== false) el.append(c instanceof Node ? c : String(c));
  return el;
}

function nativeArt(className = 'avatar token-art') {
  return h('img', { class: className, src: new URL('assets/dawn-flat.svg', import.meta.url).href, alt: '', 'aria-hidden': 'true', width: 40, height: 40 });
}

function holdingArt(token) {
  const src = tokenArtSource(defaultChainId, token.address);
  if (src) return h('img', { class: 'avatar token-art', src, alt: '', 'aria-hidden': 'true', width: 40, height: 40 });
  const { fill, letter } = tokenFallbackAppearance(token);
  const disc = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
  disc.setAttribute('viewBox', '0 0 36 36');
  disc.setAttribute('class', 'unverified-disc');
  disc.setAttribute('aria-hidden', 'true');
  const circle = document.createElementNS(disc.namespaceURI, 'circle');
  for (const [key, value] of Object.entries({ cx: 18, cy: 18, r: 18, fill })) circle.setAttribute(key, value);
  disc.append(circle);
  return h('span', { class: 'avatar unverified-art', role: 'img', 'aria-label': 'Unverified token' },
    disc,
    h('span', { class: 'unverified-initial', 'aria-hidden': 'true' }, letter),
    h('span', { class: 'unverified-question', 'aria-hidden': 'true' }, '?'));
}

function actionIcon(name) {
  const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
  for (const [key, value] of Object.entries({ viewBox: '0 0 24 24', fill: 'none', stroke: 'currentColor', 'stroke-width': '1.7', 'stroke-linecap': 'round', 'stroke-linejoin': 'round', 'aria-hidden': 'true', class: 'ico' })) svg.setAttribute(key, value);
  const path = document.createElementNS(svg.namespaceURI, 'path');
  path.setAttribute('d', name === 'receive' ? 'M12 4v16M6 14l6 6 6-6' : 'M6 18 18 6M6 6h12v12');
  svg.append(path);
  return svg;
}

function header(extra) {
  return h('header', {}, nativeArt('logo'), h('h1', { class: 'es-wordmark' }, Brand.project), h('span', { class: 'header-kind' }, 'Wallet'),
    developmentNetwork ? h('span', { class: 'pill warn' }, 'Dev network') : null, extra);
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

/** The one-time notice, with the same risk points as the app's terms. */
function noticeView() {
  const out = h('div');
  const btn = h('button', { class: 'primary' }, 'I understand');
  btn.addEventListener('click', action(btn, out, async () => { await op('acceptTerms'); refresh(); }));
  render(header(), h('div', { class: 'card notice' },
    h('h2', {}, `Before you use ${Brand.project}`),
    ...noticePoints(defaultChainId).map((p) => h('p', { class: 'small' }, p)),
    h('a', { class: 'small', href: DISCLAIMER_URL, target: '_blank', rel: 'noreferrer' }, 'Read the full terms and disclaimer'),
    btn,
    out));
}

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
    h('p', { class: 'muted small' }, `A new key is made in this browser and encrypted with your password. It works without the ${Brand.project} app. ${defaultChainId === 7780 ? `This is the testnet; its ${coinTicker(defaultChainId)} does not carry over to mainnet. ` : ''}Provided as is and not yet independently audited.`),
    h('label', {}, 'Password', pw),
    h('label', {}, 'Password again', pw2),
    create,
    h('details', {}, h('summary', { class: 'small muted' }, 'Import an existing key instead'),
      h('div', { class: 'list import-fields' }, h('label', {}, 'Private key', secret), importBtn)),
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

function readableField(field, domain = false) {
  const labels = { name: 'domainName', chainId: 'chainId', verifyingContract: 'verifyingContract', version: 'domainVersion' };
  const value = field.displayKey ? t(field.displayKey) : field.byteLength != null ? t('byteCount', { count: field.byteLength }) : field.value;
  return h('div', { class: 'signing-field' }, h('div', { class: 'small muted' }, domain && labels[field.path] ? t(labels[field.path]) : field.path),
    h('div', { class: field.type === 'address' ? 'mono' : 'signing-value', dir: 'auto' }, value),
    field.detail ? h('details', {}, h('summary', { class: 'small muted' }, t('callData')), h('div', { class: 'mono' }, field.detail)) : null);
}

function simulatedEffects(simulation) {
  const body = [message(simulation.success ? 'ok' : 'error', t(simulation.success ? 'simulationPassed' : 'simulationReverted'))];
  if (!simulation.success) body.push(h('div', { class: 'kv' }, h('span', {}, t('failureReason')), h('strong', {}, simulation.failureReason)));
  if (simulation.success) {
    body.push(h('h2', {}, t('balanceChanges')));
    if (!simulation.tokenCoverageComplete) body.push(h('div', { class: 'small warn' }, t('incompleteTokenCoverage')));
    if (!simulation.balanceChanges.length) body.push(h('p', { class: 'small muted' }, t('noBalanceChanges')));
    for (const change of simulation.balanceChanges) {
      const sign = BigInt(change.delta) > 0n ? '+' : '';
      const amount = change.baseUnits ? t('baseUnits', { amount: `${sign}${change.amount}` }) : `${sign}${change.amount}${change.kind === 'native' ? ` ${change.symbol}` : ''}`;
      body.push(h('div', { class: 'signing-effect' },
        h('div', { class: 'row' }, h('strong', { class: 'grow' }, change.kind === 'nft' ? t('nftLabel', { id: change.tokenId }) : change.symbol || t('token')), h('strong', { class: BigInt(change.delta) < 0n ? 'warn' : 'positive' }, amount)),
        change.kind !== 'native' ? h('div', { class: 'mono muted' }, change.address) : null,
        change.kind === 'erc20' && !change.trusted ? h('div', { class: 'small warn' }, t('unverifiedUnits')) : null,
        change.source === 'log' ? h('div', { class: 'small warn' }, t('reportedByContract')) : null));
    }
    body.push(h('h2', {}, t('approvalsGranted')));
    if (!simulation.approvals.length) body.push(h('p', { class: 'small muted' }, t('noApprovals')));
    for (const grant of simulation.approvals) {
      const amount = grant.revoked ? t('approvalRevoke') : grant.kind === 'all' ? t('approvalAll') : grant.kind === 'nft' ? t('approvalNFT', { id: grant.tokenId }) : grant.unlimited ? t('allowanceUnlimited') : grant.baseUnits ? t('baseUnits', { amount: grant.amount }) : `${grant.amount} ${grant.symbol || ''}`;
      body.push(h('div', { class: 'signing-effect' }, h('strong', { class: grant.revoked ? '' : 'warn' }, amount),
        h('div', { class: 'small muted' }, t('token')), h('div', { class: 'mono' }, grant.address),
        h('div', { class: 'small muted' }, t('spender')), h('div', { class: 'mono' }, grant.spender),
        grant.kind === 'erc20' && !grant.trusted ? h('div', { class: 'small warn' }, t('unverifiedUnits')) : null));
    }
    if (simulation.unrecognizedLogs) body.push(h('div', { class: 'small warn' }, t('unrecognizedLogs')));
    body.push(h('p', { class: 'small muted' }, t('effectsNotice')));
  }
  body.push(h('p', { class: 'small muted' }, t('simulationNotice')));
  return body;
}

async function approvalView(s) {
  const a = s.approvals.find((x) => x.id === approveId);
  if (!a) {
    render(header(), h('div', { class: 'card' }, h('p', {}, t('expired')), h('button', { onclick: () => window.close() }, t('close'))));
    return;
  }
  const out = h('div');
  let shownPreview = null, loading = a.kind === 'send', submitting = false;
  const yes = h('button', { class: 'primary' }, t(a.kind === 'connect' ? 'connect' : a.kind === 'typed' ? 'signMessage' : 'approve'));
  const no = h('button', {}, t('reject'));
  const extraConfirm = h('input', { type: 'checkbox' });
  const confirmation = h('label', { class: 'confirm-check', hidden: true }, extraConfirm, h('span', {}, t('revertAck')));
  const syncButton = () => { yes.disabled = submitting || (a.kind === 'send' && (loading || !shownPreview || (!shownPreview.simulation.success && !extraConfirm.checked))); };
  extraConfirm.addEventListener('change', syncButton);
  const body = [h('div', { class: 'small muted' }, t(a.kind === 'connect' ? 'connectRequest' : a.kind === 'typed' ? 'typedRequest' : 'sendRequest')), h('div', { class: 'origin' }, a.origin)];
  let loadPreview = null;
  if (a.kind === 'connect') {
    body.push(h('p', { class: 'small muted' }, 'It will see your address and balance, and can ask for transactions. Every transaction still needs your approval.'),
      h('div', { class: 'kv' }, h('span', {}, t('account')), h('span', { class: 'mono' }, s.address)));
  } else if (a.kind === 'typed') {
    body.push(h('div', { class: 'kv' }, h('span', {}, t('account')), h('span', { class: 'mono' }, a.account)),
      ...a.domainFields.map((field) => readableField(field, true)),
      !a.domainFields.some((field) => field.path === 'verifyingContract') ? h('div', { class: 'warn' }, t('noVerifyingContract')) : null,
      h('p', { class: 'small muted' }, t('domainNotice')),
      h('h2', {}, `${t('messageFields')} · ${a.primaryType}`), ...a.fields.map((field) => readableField(field)),
      h('div', { class: 'warn' }, t('typedWarning')));
  } else {
    const fee = h('span', { class: 'muted' }, '…');
    const previewBox = h('div', { class: 'list', role: 'status' }, t('simulationRunning'));
    const retry = h('button', { class: 'link small' }, t('retryPreview'));
    loadPreview = async () => {
      if (submitting) return;
      loading = true; shownPreview = null; extraConfirm.checked = false; confirmation.hidden = true; retry.disabled = true; syncButton();
      previewBox.replaceChildren(t('simulationRunning'));
      try {
        const fresh = await op('preview', { id: approveId });
        shownPreview = fresh;
        fee.replaceChildren(t('maxFee', { amount: formatAeth(fresh.fee, 6), symbol: coinTicker(fresh.chainId) }));
        previewBox.replaceChildren(...simulatedEffects(fresh.simulation));
        confirmation.hidden = fresh.simulation.success;
      } catch (e) {
        fee.replaceChildren('—');
        previewBox.replaceChildren(message('error', e.message || t('simulationUnavailable')));
      } finally { loading = false; retry.disabled = false; syncButton(); }
    };
    retry.addEventListener('click', () => loadPreview());
    body.push(h('div', { class: 'kv' },
      h('span', {}, 'Action'), h('strong', {}, a.what),
      h('span', {}, t('contractCalled')), h('span', { class: 'mono' }, a.tx.to || t('contractCreation')),
      h('span', {}, t('intendedValue')), h('strong', {}, `${a.value} ${coinTicker(a.chainId)}`),
      h('span', {}, t('networkFee')), fee,
      h('span', {}, t('from')), h('span', { class: 'mono' }, a.account)),
      h('h2', {}, t('simulationTitle')), previewBox, retry, confirmation);
    if (a.tx.data !== '0x') body.push(h('details', {}, h('summary', { class: 'small muted' }, t('callData')), h('div', { class: 'mono muted call-data' }, a.tx.data)));
    if (a.what.startsWith('Token approval')) body.push(h('div', { class: 'warn' }, 'An approval lets the contract move your tokens later. Approve only contracts you trust.'));
  }
  yes.addEventListener('click', async () => {
    submitting = true; approvalBusy = true; no.disabled = true; syncButton(); out.replaceChildren();
    try {
      const r = await op('approve', { id: approveId, previewId: shownPreview?.previewId || a.previewId, confirmRevert: extraConfirm.checked });
      render(header(), h('div', { class: 'card' }, message('ok', t(r.hash ? 'transactionSent' : r.signed ? 'signatureSigned' : 'connected')), r.hash ? h('div', { class: 'mono muted' }, r.hash) : null));
      setTimeout(() => window.close(), r.hash ? 1400 : 600);
    } catch (e) {
      out.replaceChildren(message('error', e.message || String(e)));
      submitting = false;
      if (loadPreview) await loadPreview();
    } finally { submitting = false; approvalBusy = false; no.disabled = false; syncButton(); }
  });
  no.addEventListener('click', action(no, out, async () => { await op('reject', { id: approveId }); window.close(); }));
  render(header(h('span', { class: 'pill' }, t('chainLabel', { chain: a.chainId }))), h('div', { class: 'card' }, ...body), h('div', { class: 'row' }, h('div', { class: 'grow' }), no, yes), out);
  syncButton();
  loadPreview?.();
}

// ---- main popup ----

function nav() {
  const b = (id, label) => h('button', { 'aria-current': tab === id ? 'page' : null, onclick: () => { tab = id; refresh(); } }, label);
  return h('nav', { 'aria-label': 'Sections' }, b('home', 'Home'), b('assets', 'Assets'), b('activity', 'Activity'), b('sites', 'Sites'), b('settings', 'Settings'));
}

/** The balance pill: "Block N", or the paused state when no block for 60 s. */
function nodePill() {
  const node = h('span', { class: 'pill es-status' }, 'Connecting…');
  const show = (a) => {
    pauseState = nextPauseState(pauseState, { now: Date.now(), height: a.height, blockAt: a.blockAt });
    if (pauseState.pausedSince != null) {
      node.textContent = pausedLine(pauseState.pausedSince, Date.now());
      node.className = 'pill es-status paused';
      node.title = PAUSE_HELP;
    } else {
      node.textContent = `Block ${a.height}`;
      node.className = 'pill es-status good';
      node.removeAttribute('title');
    }
  };
  return { node, show };
}

async function home(s) {
  const out = h('div');
  const balanceValue = h('span', { class: 'balance-value' }, '…');
  const bal = h('div', { class: 'balance es-amount' }, balanceValue, h('span', { class: 'balance-unit' }, ` ${coinTicker(defaultChainId)}`));
  const proofNote = h('div', { class: 'small plate-proof' }, developmentNetwork ? 'Dev network · read from the node' : `Checking ${coinTicker(defaultChainId)} balance…`);
  const { node, show } = nodePill();
  const addr = h('button', { class: 'link mono', title: 'Copy address', onclick: async () => { await navigator.clipboard.writeText(s.address); addr.textContent = 'Copied'; setTimeout(() => { addr.textContent = shortAddress(s.address); }, 900); } }, shortAddress(s.address));
  const load = () => op('account').then((a) => { balanceValue.textContent = formatAeth(a.balance); show(a); if (!developmentNetwork) proofNote.textContent = `${coinTicker(defaultChainId)} balance verified with a certificate and state proof`; })
    .catch((e) => { balanceValue.textContent = '—'; node.textContent = 'No node'; node.className = 'pill es-status'; proofNote.textContent = `${coinTicker(defaultChainId)} balance unavailable`; out.replaceChildren(message('error', e.message)); });
  load();
  updaters = [load];

  // ---- the send form: AETH or any held token, with the send-flow checks of
  // token-spam-2026.md §6 (look-alike recipient, first send, dry-run) ----
  const to = h('input', { placeholder: '0x… recipient', spellcheck: 'false' });
  const amount = h('input', { placeholder: `Amount in ${coinTicker(defaultChainId)}`, inputmode: 'decimal' });
  const max = h('button', { type: 'button', class: 'link small' }, 'Max');
  const assetPick = h('select');
  const warnBox = h('div');
  const sendBtn = h('button', { class: 'primary', type: 'submit' }, 'Send');
  const sendForm = h('form', { class: 'card', hidden: true }, h('h2', {}, 'Send'),
    h('label', {}, 'Asset', assetPick),
    h('label', {}, 'To', to),
    h('label', {}, 'Amount', h('div', { class: 'row' }, amount, max)),
    warnBox, sendBtn,
    h('p', { class: 'small muted' }, 'Before signing, the recipient is checked against your history and the transfer is tried on the node (an estimate, not a guarantee). These checks read public chain data and settings on this device; nothing new is written on chain.'));

  let holdings = [];            // the picker's tokens
  let officialSymbols = [];     // official symbols, for the look-alike warning
  let asset = null;             // the chosen holding (null: AETH)
  let sent = [];                // addresses this wallet sent to before
  let acked = false;            // the look-alike warning's confirmation

  const ready = () => Boolean(to.value.trim() && amount.value.trim())
    && (!asset || /^0x[0-9a-fA-F]{40}$/.test(to.value.trim()));

  const warnings = () => {
    acked = false;
    const risk = addressRisk(to.value, sent);
    const parts = [];
    if (asset && asset.token.origin === 'launchpad') parts.push(h('span', { class: 'pill warn' }, 'Launchpad · unverified'));
    if (asset && asset.token.unverifiedUnits) parts.push(h('span', { class: 'pill warn' }, 'Unverified units'));
    if (asset && looksLikeOfficial(asset.token, officialSymbols)) parts.push(h('span', { class: 'pill warn' }, 'Mimics an official token'));
    if (risk.poisoningMatch) {
      const box = h('input', { type: 'checkbox' });
      box.addEventListener('change', () => { acked = box.checked; sendBtn.disabled = !ready() || !acked; });
      parts.push(h('div', { class: 'warn' },
        h('strong', {}, 'This address only looks like one you sent to before'),
        h('div', { class: 'small' }, `It shares its first and last characters with ${tokenShort(risk.poisoningMatch)} — a different address. Scammers copy exactly those to catch a quick copy-paste. Compare the whole address before sending.`),
        h('label', { class: 'small' }, box, ' I compared the full address; this is where I want to send')));
    } else if (risk.firstSend && to.value.trim()) {
      parts.push(h('div', { class: 'small muted' }, 'First time sending to this address. Double-check it with whoever gave it to you.'));
    }
    warnBox.replaceChildren(...parts);
    sendBtn.disabled = !ready() || (risk.poisoningMatch != null && !acked);
  };
  to.addEventListener('input', warnings);
  amount.addEventListener('input', warnings);

  const pickAssets = async () => {
    try {
      const t = await op('assets', {});
      holdings = t.tokens || [];
      officialSymbols = t.officialSymbols || [];
    } catch { holdings = []; }
    assetPick.replaceChildren(h('option', { value: '' }, `${coinTicker(defaultChainId)} · ${coinName(defaultChainId)}`),
      ...holdings.map((x) => h('option', { value: x.token.address }, `${tokenLabel(x.token)} · ${formatTokenAmount(x.balance, x.token.decimals)}`)));
  };
  assetPick.addEventListener('change', () => {
    asset = holdings.find((x) => x.token.address === assetPick.value) || null;
    amount.placeholder = asset ? `Amount in ${asset.token.symbol}` : `Amount in ${coinTicker(defaultChainId)}`;
    warnings();
  });
  max.addEventListener('click', () => {
    if (asset) amount.value = formatTokenAmountExact(asset.balance, asset.token.decimals);
    else op('account').then((a) => { amount.value = weiToAeth(BigInt(a.balance) - 10n ** 15n); }).catch(() => {});
    warnings();
  });

  sendForm.addEventListener('submit', action(sendBtn, out, async () => {
    const recipient = to.value.trim();
    const risk = addressRisk(recipient, sent);
    if (risk.poisoningMatch && !acked) throw new Error('Confirm the look-alike address warning first.');
    if (asset) {
      // Audits A3 + R2-2: the units shown and signed come either from the
      // list shipped with the wallet (trusted) or from details pinned on this
      // device — and pinned details are "unverified units" that can only be
      // sent after the exact base-unit count is acknowledged below.
      if (!asset.token.trusted) {
        if (asset.token.metadataChanged) throw new Error('This token now reports different details than the ones saved on this device. Review the change in Assets first.');
        if (asset.token.unconfirmed) throw new Error('This token’s details are not confirmed yet. Open Assets and let the wallet confirm them first.');
      }
      const pin = asset.token;
      // One immutable send intent (audit R2-5): the amount text is parsed
      // ONCE, under the decimals this screen shows, and travels as an exact
      // integer from here on — never re-parsed under whatever is stored later.
      const intent = buildSendIntent({ recipient, amountText: amount.value, token: pin });
      const units = BigInt(intent.baseUnits);
      if (BigInt(asset.balance) < units) throw new Error(`Not enough ${pin.symbol}: the balance is ${formatTokenAmount(asset.balance, pin.decimals)}`);
      const data = erc20TransferCalldata(recipient, units); // for the node simulation only
      const check = await op('sendCheck', { recipient, to: pin.address, value_wei: '0', data });
      if (check.dry.state === 'reverted') throw new Error(`Not sent — this transfer would fail: ${check.dry.message}`);
      // The simulation is never proof of success: it is shown as an estimate,
      // and no warning was skipped because it did not fail.
      const estimate = check.dry.state === 'ok'
        ? 'Node estimate, not a guarantee: the simulated transfer did not fail.'
        : 'The node could not simulate this transfer, so there is no estimate.';
      const back = h('button', { type: 'button' }, 'Back');
      const confirmBtn = h('button', { class: 'primary', type: 'button' }, 'Confirm');
      // Unverified units need the user's explicit acknowledgement of the
      // exact count before Confirm works (audit R2-2).
      let unitsAcked = pin.trusted;
      const ack = pin.trusted ? null : h('input', { type: 'checkbox' });
      if (ack) {
        confirmBtn.disabled = true;
        ack.addEventListener('change', () => { unitsAcked = ack.checked; confirmBtn.disabled = !unitsAcked; });
      }
      back.addEventListener('click', () => out.replaceChildren());
      confirmBtn.addEventListener('click', action(confirmBtn, out, async () => {
        // Exactly what was confirmed: the wallet core signs this integer and
        // refuses if the stored details moved since this card was built.
        const r = await op('send', { to: intent.recipient, token: { ...intent.token, acknowledged: unitsAcked, baseUnits: intent.baseUnits } });
        out.replaceChildren(message('ok', `Sent ${formatTokenAmount(units, pin.decimals)} ${pin.symbol} to ${tokenShort(intent.recipient)} · ${shortAddress(r.hash)}`));
        sendForm.hidden = true;
      }));
      out.replaceChildren(h('div', { class: 'card' },
        h('h2', {}, 'Confirm the send'),
        h('div', { class: 'kv' },
          h('span', {}, 'You will send'), h('strong', {}, `${grouped(units)} units`),
          h('span', {}, 'Shown as'), h('span', { class: 'mono' }, `${formatTokenAmount(units, pin.decimals)} ${pin.symbol}`),
          h('span', {}, 'Decimals'), h('span', { class: 'mono' }, pin.trusted ? `${pin.decimals} (shipped list)` : `${pin.decimals} (unverified claim)`),
          h('span', {}, 'To'), h('span', { class: 'mono' }, intent.recipient),
          h('span', {}, 'From'), h('span', { class: 'mono' }, shortAddress(s.address))),
        pin.trusted ? null : h('div', { class: 'warn' },
          h('strong', {}, 'Unverified units'),
          h('div', { class: 'small' }, `This token is not on the wallet’s trusted list, so the wallet cannot check what one unit is. The count above follows the token contract’s unverified claim of ${pin.decimals} decimals. Compare it with what you expect before sending.`),
          h('label', { class: 'small' }, ack, ` I checked the exact count: ${grouped(units)} units is what I want to send`)),
        h('p', { class: 'small muted' }, estimate),
        h('div', { class: 'row' }, h('div', { class: 'grow' }), back, confirmBtn)));
    } else {
      const wei = aethToWei(amount.value);
      const check = await op('sendCheck', { recipient, to: recipient, value_wei: wei.toString(), data: '0x' });
      if (check.dry.state === 'reverted') throw new Error(`Not sent — this transfer would fail: ${check.dry.message}`);
      const r = await op('send', { to: recipient, value_wei: wei.toString() });
      out.replaceChildren(message('ok', `Sent ${weiToAeth(wei)} ${coinTicker(defaultChainId)} · ${shortAddress(r.hash)}`));
      sendForm.hidden = true;
    }
  }));
  const receive = h('button', { onclick: () => navigator.clipboard.writeText(s.address).then(() => out.replaceChildren(message('ok', 'Address copied.'))) }, actionIcon('receive'), 'Receive');
  const send = h('button', { class: 'primary', onclick: () => { sendForm.hidden = !sendForm.hidden; if (!sendForm.hidden) { to.focus(); pickAssets(); op('activity').then((l) => { sent = l.filter((a) => !a.owner || a.owner.toLowerCase() === s.address.toLowerCase()).map((a) => a.to).filter(Boolean); warnings(); }).catch(() => {}); } } }, actionIcon('send'), 'Send');
  return [h('section', { class: 'es-plate hero', 'aria-label': `${coinTicker(defaultChainId)} balance` },
    h('div', { class: 'row plate-top' }, nativeArt('plate-coin'), h('div', { class: 'grow' }, h('div', { class: 'plate-title' }, coinName(defaultChainId)), addr)),
    bal,
    h('div', { class: 'row plate-network' }, node, h('span', { class: 'small' }, developmentNetwork ? 'Dev network' : `${Brand.project} chain ${defaultChainId}`)),
    proofNote),
    h('div', { class: 'actions' }, receive, send), sendForm, out];
}

// ---- assets ----

function agoLine(ts, now = Date.now()) {
  const s = Math.max(0, Math.floor((now - ts) / 1000));
  if (s < 45) return 'Updated just now';
  const m = Math.max(1, Math.floor(s / 60));
  return m < 120 ? `Updated ${m} min ago` : `Updated ${Math.floor(m / 60)} h ago`;
}

/** A token row: never the symbol alone ("NEB · 0x8a9B…F41c"), the launchpad
 * and look-alike badges, and the user's own show/hide choice. */
function holdingRow(x, officialSymbols, { onHide, onShow } = {}) {
  const badges = [];
  if (x.token.origin === 'launchpad') badges.push(h('span', { class: 'pill warn' }, 'Launchpad · unverified'));
  if (looksLikeOfficial(x.token, officialSymbols)) badges.push(h('span', { class: 'pill warn' }, 'Mimics an official token'));
  if (x.token.nodeDisagrees) badges.push(h('span', { class: 'pill warn' }, 'Node disagrees · units from the shipped list'));
  if (x.token.unverifiedUnits) badges.push(h('span', { class: 'pill warn' }, 'Unverified units'));
  if (!x.token.trusted && x.token.metadataChanged) badges.push(h('span', { class: 'pill warn' }, 'Details changed'));
  else if (x.token.unconfirmed) badges.push(h('span', { class: 'pill warn' }, 'Details not confirmed'));
  const act = onHide ? h('button', { class: 'small', onclick: onHide }, 'Hide')
    : onShow ? h('button', { class: 'small show-token', onclick: onShow }, 'Show in main list') : null;
  return h('div', { class: onShow ? 'item unverified-item' : 'item', title: x.token.address },
    holdingArt(x.token),
    h('div', { class: 'grow' },
      h('div', {}, displayTokenName(defaultChainId, x.token.address, x.token.name) || x.token.symbol),
      h('div', { class: 'small muted mono' }, tokenLabel(x.token)),
      badges.length ? h('div', { class: 'row holding-badges' }, ...badges) : null),
    act,
    h('div', { class: 'holding-amount' }, h('strong', { class: 'es-amount' }, formatTokenAmount(x.balance, x.token.decimals)), h('span', { class: 'small muted' }, ` ${x.token.symbol}`)));
}

/** The audit-A3 review card: the details saved on this device and the changed
 * ones side by side. Sending stays paused until the user answers it. */
async function tokenReviewCard(token, done) {
  const out = h('div');
  const { pinned, observed } = await op('tokenChange', { address: token.address }).catch(() => ({ pinned: null, observed: null }));
  const show = (m) => h('div', { class: 'kv' },
    h('span', {}, 'Decimals'), h('span', { class: 'mono' }, String(m?.decimals ?? '—')),
    h('span', {}, 'Symbol'), h('span', { class: 'mono' }, m?.symbol ?? '—'),
    h('span', {}, 'Name'), h('span', { class: 'mono' }, m?.name ?? '—'));
  const keep = h('button', { type: 'button' }, 'Not now');
  const use = h('button', { class: 'primary', type: 'button' }, 'Use the new details');
  const card = h('div', { class: 'card' },
    h('h2', {}, `${token.symbol} reports different details`),
    h('p', { class: 'small' }, 'The nodes now describe this token differently than the details saved on this device, so sending it is paused. Compare both, then use the new details only if you know why they changed.'),
    h('div', { class: 'small muted' }, 'Saved on this device'), show(pinned),
    h('div', { class: 'small muted' }, 'Reported by the nodes'), show(observed || token),
    h('div', { class: 'row' }, h('div', { class: 'grow' }), keep, use),
    out);
  keep.addEventListener('click', () => card.remove()); // stays paused
  use.addEventListener('click', action(use, out, async () => {
    const r = await op('confirmTokenChange', { address: token.address });
    out.replaceChildren(message('ok', r.accepted ? 'The new details are saved. Sending works again.' : 'The nodes describe this token differently again; the saved details stand and sending stays paused.'));
    done();
  }));
  return card;
}

async function assetsView(s) {
  const aethAmt = h('strong', { class: 'es-amount' }, '…');
  const proofNote = h('div', { class: 'small muted' }, developmentNetwork ? 'Dev network · read from the node' : `Checking ${coinTicker(defaultChainId)} balance…`);
  const rows = h('div', { class: 'list' });
  const review = h('div', { class: 'list token-reviews' });
  const unverified = h('div', { class: 'list' });
  const unverifiedBox = h('details', { hidden: true },
    h('summary', {}, 'Unverified'),
    h('p', { class: 'small muted' }, 'Someone sent these to you. Nothing you signed ever touched them — anyone can create a token, so check the contract address before trusting one.'),
    unverified);
  const note = h('div', { class: 'small muted' }, 'Looking for tokens…');
  const updated = h('div', { class: 'small muted' });
  const load = async (force) => {
    const [acct, assets] = await Promise.allSettled([op('account'), op('assets', { force })]);
    if (acct.status === 'fulfilled') {
      aethAmt.replaceChildren(formatAeth(acct.value.balance));
      if (!developmentNetwork) proofNote.textContent = `${coinTicker(defaultChainId)} verified · token balances read from the node`;
    } else {
      aethAmt.replaceChildren('—');
      proofNote.textContent = `${coinTicker(defaultChainId)} balance unavailable · token balances read from the node`;
    }
    if (assets.status === 'rejected') {
      note.textContent = 'Could not read tokens from the node. It tries again shortly.';
      return;
    }
    const t = assets.value;
    const officialSymbols = t.officialSymbols || [];
    // A "details changed" review is for pinned, unverified tokens only: a
    // token on the shipped list never pauses for node noise (audit R2-2) —
    // it just carries the nodeDisagrees badge.
    const flagged = [...(t.tokens || []), ...(t.unverified || [])].filter((x) => x.token.metadataChanged && !x.token.trusted);
    review.replaceChildren();
    for (const x of flagged) review.append(await tokenReviewCard(x.token, () => load(true)));
    rows.replaceChildren(...(t.tokens || []).map((x) => holdingRow(x, officialSymbols, {
      onHide: async () => { await op('hideToken', { address: x.token.address }); load(false); },
    })));
    unverified.replaceChildren(...(t.unverified || []).map((x) => holdingRow(x, officialSymbols, {
      onShow: async () => { await op('showToken', { address: x.token.address }); load(false); },
    })));
    unverifiedBox.hidden = !(t.unverified || []).length;
    unverifiedBox.querySelector('summary').textContent = `Unverified (${(t.unverified || []).length})`;
    const any = (t.tokens || []).length + (t.unverified || []).length;
    note.replaceChildren(any ? '' : t.error ? 'Could not read tokens from the node. It tries again shortly.' : t.updated != null ? 'No other tokens in this wallet.' : 'Looking for tokens…');
    updated.textContent = t.updated != null ? agoLine(t.updated) : '';
  };
  load(true);
  updaters = [() => load(false)];
  // The AETH account is proof checked; token calls remain node-sourced.
  return [h('div', { class: 'card' },
    h('div', { class: 'row' }, h('h2', { class: 'grow' }, 'Assets'), h('span', { class: 'small muted nowrap' }, developmentNetwork ? 'Dev network' : `${Brand.project} network`)),
    proofNote,
    h('div', { class: 'item', title: s.address },
      nativeArt(),
      h('div', { class: 'grow' }, h('div', {}, coinName(defaultChainId)), h('div', { class: 'small muted mono' }, shortAddress(s.address))),
      h('div', { class: 'holding-amount' }, aethAmt, h('span', { class: 'small muted' }, ` ${coinTicker(defaultChainId)}`))),
    h('h2', {}, 'Tokens'),
    review, rows, unverifiedBox, note, updated)];
}

async function activity() {
  let { items, cursors, starts } = await op('activityPage');
  const linked = await op('linkedWallets');
  const rows = h('div', { class: 'card list' });
  const draw = () => rows.replaceChildren(...(items.length ? items.map((a) => h('div', { class: 'item' },
    h('span', { class: `dot ${a.state}`, title: a.state }),
    h('div', { class: 'grow' }, h('div', {}, a.title), h('div', { class: 'small muted' }, `${a.origin} · ${new Date(a.at).toLocaleString()}`), h('div', { class: 'mono muted', title: a.hash }, shortAddress(a.hash))),
    a.value && a.value !== '0' && !a.title.startsWith('Swapped') ? h('div', { class: 'small' }, `${formatAeth(a.value)} ${coinTicker(defaultChainId)}`) : null))
    : [h('p', { class: 'muted' }, 'No transactions yet.')]));
  draw();
  const more = h('button', { type: 'button' }, 'Load older');
  more.hidden = !Object.keys(cursors).length;
  more.addEventListener('click', async () => {
    more.disabled = true;
    try {
      const page = await op('activityPage', { cursors });
      items = mergeHistory(items, page.items);
      cursors = page.cursors;
      starts = { ...starts, ...page.starts };
      draw();
      more.hidden = !Object.keys(cursors).length;
    } finally { more.disabled = false; }
  });
  const address = h('input', { placeholder: '0x… address (view only)', spellcheck: 'false', 'aria-label': 'Linked wallet address' });
  const add = h('button', { type: 'submit' }, 'Add');
  const output = h('div');
  const form = h('form', { class: 'row' }, address, add);
  form.addEventListener('submit', action(add, output, async () => { await op('linkWallet', { address: address.value }); refresh(); }));
  const links = h('div', { class: 'card' }, h('h2', {}, 'Linked wallets'),
    h('p', { class: 'small muted' }, 'View activity for up to 8 other addresses. Their signing keys stay in their own wallets.'),
    ...linked.map((a) => h('div', { class: 'item' }, h('span', { class: 'mono grow', title: a }, shortAddress(a)),
      h('button', { onclick: async () => { await op('unlinkWallet', { address: a }); refresh(); } }, 'Remove'))), form, output);
  const first = Math.max(0, ...Object.values(starts || {}));
  return [links, first ? h('p', { class: 'small muted' }, `This node's retained history starts at block #${first}.`) : null, rows, more];
}

async function sitesView() {
  const s = await op('sites');
  const entries = Object.entries(s);
  const input = h('input', { placeholder: 'harbor.sea or sea://harbor', spellcheck: 'false' });
  const out = h('div');
  const names = h('form', { class: 'card' }, h('h2', {}, 'Open an EastSea name'),
    h('label', {}, 'Name or link', input), h('button', { type: 'submit' }, 'Check link'), out);
  names.addEventListener('submit', (event) => {
    event.preventDefault();
    out.replaceChildren();
    try {
      const link = browserInput(input.value, defaultChainId);
      const href = link.kind === 'name' ? link.canonicalURL : link.kind === 'action' ? link.raw : link.url;
      out.append(h('a', { href, target: '_blank', rel: 'noopener noreferrer' }, href));
      if (link.kind !== 'web') out.append(h('p', { class: 'small muted' },
        'Open this link in the installed wallet. Payments require its approval screen. Content delivery comes next.'));
    } catch (error) {
      out.append(message('error', error.code === 'externalTLD' ? externalNameMessage(navigator.language) : error.message));
      const https = suggestedHTTPS(input.value);
      if (https) out.append(h('a', { href: https, target: '_blank', rel: 'noopener noreferrer' }, 'Open with https://'));
    }
  });
  if (!entries.length) return [names, h('div', { class: 'card' }, h('p', { class: 'muted' }, 'No sites are connected.'))];
  return [names, h('div', { class: 'card list' }, entries.map(([origin, e]) => h('div', { class: 'item' },
    h('div', { class: 'grow' }, h('div', { class: 'mono' }, origin), h('div', { class: 'small muted' }, `since ${new Date(e.at).toLocaleDateString()}`)),
    h('button', { class: 'danger', onclick: async () => { await op('disconnect', { origin }); refresh(); } }, 'Disconnect'))))];
}

function settingsView(s) {
  const out = h('div');
  const minutes = h('input', { type: 'number', min: 1, max: 1440, value: s.lockMinutes });
  const developerMode = h('input', { type: 'checkbox' });
  developerMode.checked = Boolean(s.developerMode);
  const network = h('select', {}, h('option', { value: 'default' }, 'Default'), h('option', { value: 'development' }, 'Local development network'));
  network.value = s.developmentNetwork ? 'development' : 'default';
  const port = h('input', { type: 'number', min: 1024, max: 65535, value: s.developmentPort });
  const networkFields = h('div', { class: 'network-fields' }, h('label', {}, 'Network', network), h('label', {}, 'Local RPC port (127.0.0.1)', port));
  networkFields.hidden = !developerMode.checked;
  developerMode.addEventListener('change', () => { networkFields.hidden = !developerMode.checked; });
  const rpcs = h('textarea', { placeholder: 'https://node.example (one per line)', spellcheck: 'false' }, s.rpcs.join('\n'));
  const save = h('button', { type: 'submit' }, 'Save');
  const form = h('form', { class: 'card' }, h('h2', {}, 'Settings'), h('label', {}, 'Lock after (minutes)', minutes),
    h('label', {}, 'Extra default-network nodes', rpcs), h('p', { class: 'small muted' }, `Default: the ${Brand.project} app\'s node on this computer (127.0.0.1:18545).`),
    h('label', {}, developerMode, ' Developer mode'), networkFields, save);
  form.addEventListener('submit', action(save, out, async () => {
    const list = rpcs.value.split(/\s+/).filter(Boolean);
    if (network.value === 'development' && !developerMode.checked) throw new Error('Turn on Developer mode to use a local development network.');
    const extra = list.filter((u) => !/^http:\/\/(127\.0\.0\.1|localhost)[:/]/.test(u));
    if (extra.length && !(await chrome.permissions.request({ origins: extra.map((u) => `${new URL(u).origin}/*`) }))) throw new Error('The browser did not allow those nodes.');
    await op('settings', { lockMinutes: minutes.value, rpcs: list, developerMode: developerMode.checked,
      developmentNetwork: network.value === 'development', developmentPort: Number(port.value) });
    out.replaceChildren(message('ok', 'Saved.'));
    if (developerMode.checked !== s.developerMode || (network.value === 'development') !== s.developmentNetwork) refresh();
  }));
  const developerOut = h('div');
  const faucet = h('button', { type: 'button' }, `Get test ${coinTicker(defaultChainId)}`);
  faucet.addEventListener('click', action(faucet, developerOut, async () => {
    const r = await op('faucet');
    developerOut.replaceChildren(message('ok', `Faucet sent · ${shortAddress(r.hash)}. The balance updates when it is final.`));
  }));
  const developer = h('details', { class: 'card' }, h('summary', {}, 'Developer mode'), faucet, developerOut);

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
  return [form, out, s.developerMode && s.developmentNetwork ? developer : null, backup];
}

async function refresh() {
  updaters = [];
  let s;
  try {
    s = await op('state');
  } catch (e) {
    render(header(), message('error', e.message));
    return;
  }
  developmentNetwork = s.developmentNetwork;
  defaultChainId = s.defaultChainId;
  if (s.terms < TERMS_VERSION) return noticeView();
  if (!s.exists) return onboarding();
  if (!s.unlocked) return unlockView(s.address);
  if (approveId) return approvalView(s);
  const lock = h('button', { class: 'link small', onclick: async () => { await op('lock'); refresh(); } }, 'Lock');
  const pendingNote = s.approvals.length ? h('div', { class: 'warn' }, `${s.approvals.length} request(s) waiting in their approval window.`) : null;
  const views = { home, assets: assetsView, activity, sites: sitesView, settings: settingsView };
  render(header(lock), nav(), pendingNote, ...(await views[tab](s)));
}

chrome.storage.onChanged.addListener((changes, area) => {
  if (!approveId || approvalBusy) return;
  if ((area === 'session' && changes.approvals && !(changes.approvals.newValue || []).some((a) => a.id === approveId))
    || (area === 'session' && changes.unlocked) || (area === 'local' && changes.vault)) refresh();
});

refresh();
