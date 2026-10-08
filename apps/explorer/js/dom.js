// DOM helpers. Everything renders through `h`, whose children are set as text
// nodes only — nothing from the chain is ever interpreted as HTML.

/** `h('a', {href:'#/block/1'}, 'Block 1')` — attributes via setAttribute,
 * `on*` props as listeners, `class` as className; children flattened, text
 * unless already a Node. */
export function h(tag, props = {}, ...children) {
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

/** A card: the box every section of a page sits in. */
export function card(title, ...children) {
  return h('section', { class: 'card' }, title ? h('h2', { class: 'es-section-title' }, title) : null, ...children);
}

/** Status pill. `kind`: good | warn | bad | plain. Never color alone — always text. */
export function pill(text, kind = 'plain') {
  return h('span', { class: `pill es-status ${kind}` }, text);
}

/** A status dot with a text label beside it. */
export function dot(kind, label) {
  return h('span', { class: 'row tight' }, h('span', { class: `dot ${kind}`, 'aria-hidden': 'true' }), h('span', {}, label));
}

/** `label → value` rows, the field list at the top of every detail page. */
export function kv(pairs) {
  return h('dl', { class: 'kv' }, ...pairs.flatMap(([k, v]) => [h('dt', {}, k), h('dd', {}, wrap(v))]));
}

function wrap(v) {
  return v instanceof Node ? v : h('span', { class: 'mono' }, v == null ? '—' : String(v));
}

/** A table that scrolls sideways on narrow screens instead of breaking. */
export function table(headings, rows) {
  return h('div', { class: 'tablewrap' },
    h('table', {},
      h('thead', {}, h('tr', {}, ...headings.map((t) => h('th', { scope: 'col' }, t)))),
      h('tbody', {}, ...rows.map((r) => h('tr', {}, ...r.map((c) => h('td', {}, c == null ? '—' : c)))))));
}

/** A message line (errors, notices). */
export function message(kind, text) {
  return h('div', { class: `msg ${kind}`, role: kind === 'error' ? 'alert' : 'status' }, text);
}

/** Copy button for a full hash/address. Clipboard needs a secure context
 * (localhost or https); on failure it says so instead of pretending. */
export function copyButton(text) {
  const b = h('button', { class: 'copy es-control', title: 'Copy', 'aria-label': 'Copy to clipboard' }, 'copy');
  b.addEventListener('click', async () => {
    try {
      await navigator.clipboard.writeText(text);
      b.textContent = 'copied ✓';
    } catch {
      b.textContent = 'copy failed';
    }
    setTimeout(() => { b.textContent = 'copy'; }, 1500);
  });
  return b;
}

/** The line every page carries: where its data came from. */
export function sourceLine(node, extra) {
  return h('p', { class: 'small muted source' }, 'Data read from the node at ', h('span', { class: 'mono' }, node.url), extra ? ` · ${extra}` : null, '. Not light-client verified.');
}

/** Placeholder while a page fetches. */
export function loading(text = 'Loading…') {
  return h('div', { class: 'loading', role: 'status' }, h('span', { class: 'spin', 'aria-hidden': 'true' }), h('span', {}, text));
}
