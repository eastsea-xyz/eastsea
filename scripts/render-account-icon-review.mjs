#!/usr/bin/env node
// Offline Chromium review only: no wallet, live node, signer, or dependencies.
// Usage: node scripts/render-account-icon-review.mjs [--prototype]
import { createServer } from 'node:http';
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { readFileSync, writeFileSync, mkdirSync, mkdtempSync, rmSync, existsSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import {
  ACCOUNT_ICON_VERSION, ACCOUNT_ICON_PALETTES, ACCOUNT_ICON_SILHOUETTES,
  deriveAccountIcon, accountIconSVG, accountIconSilhouette,
} from '../apps/extension/src/lib/accountIcon.js';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
// Reuse the already installed review browser. Overrides make the script usable
// on another development Mac without introducing a package dependency.
const playwrightModule = process.env.PLAYWRIGHT_MODULE || '/Users/kjaylee/.codex/skills/develop-web-game/node_modules/playwright/index.mjs';
const { chromium } = await import(pathToFileURL(path.resolve(root, playwrightModule)).href);
const temporary = path.join(root, 'tmp/account-icon-round2/review-render');
mkdirSync(temporary, { recursive: true });
process.env.TMPDIR = temporary;
const prototype = process.argv.includes('--prototype');
if (process.argv.slice(2).some(arg => arg !== '--prototype')) throw new Error('Usage: node scripts/render-account-icon-review.mjs [--prototype]');
const output = prototype ? path.join(root, 'tmp/account-icon-round2/prototype') : path.join(root, 'docs/design/46-account-icon');
const browserOutput = path.join(output, 'browser');
const surfaceOutput = path.join(output, 'surfaces');
const reviewOutput = path.join(output, 'round2');
for (const dir of [browserOutput, surfaceOutput, reviewOutput]) mkdirSync(dir, { recursive: true });

// These identities are the round-one review's actual fixtures. Vector ordering
// can change with a new specification; the phishing example must not change.
const ADDRESSES = Object.freeze({
  sender: '0x1234567890abcdef1234567890abcdef12345678',
  recipient: '0x1234567890abcdef0000000000abcdef12345678',
  contract: '0xa2521982a17474cb2f8741c85de653b5282d72b0',
});
const fixture = JSON.parse(readFileSync(path.join(root, 'crates/client/tests/account-icon-vectors.json')));
if (fixture.version !== ACCOUNT_ICON_VERSION || fixture.vectors.length !== 16) throw new Error('Canonical module and sixteen shared vectors must be ready before rendering.');
const provenance = {
  canonicalSourceSha256: createHash('sha256').update(readFileSync(path.join(root, 'apps/extension/src/lib/accountIcon.js'))).digest('hex'),
  sharedVectorsSha256: createHash('sha256').update(readFileSync(path.join(root, 'crates/client/tests/account-icon-vectors.json'))).digest('hex'),
};
const baselineModule = path.join(temporary, 'account-icon-v1.mjs');
writeFileSync(baselineModule, execFileSync('git', ['show', '6cd6fee:apps/extension/src/lib/accountIcon.js'], { cwd: root }));
const baseline = await import(pathToFileURL(baselineModule).href);
const savedBaselineSurface = path.join(root, 'tmp/account-icon-round2/before/extension-approval-dark.png');
const baselineSurface = existsSync(savedBaselineSurface) ? savedBaselineSurface : path.join(temporary, 'extension-approval-v1-dark.png');
if (!existsSync(baselineSurface)) writeFileSync(baselineSurface, execFileSync('git', ['show', '6cd6fee:docs/design/46-account-icon/surfaces/extension-approval-dark.png'], { cwd: root, maxBuffer: 16 * 1024 * 1024 }));
const imageData = filename => `data:image/png;base64,${readFileSync(filename).toString('base64')}`;
const escape = value => String(value).replace(/[&<>"']/g, char => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[char]);
const short = address => `${address.slice(0, 10)}…${address.slice(-6)}`;
const icon = (address, size) => accountIconSVG(deriveAccountIcon(address), size);
const font = (name, filename) => `@font-face{font-family:${name};src:url(data:font/woff2;base64,${readFileSync(path.join(root, 'site/fonts', filename)).toString('base64')}) format('woff2');font-weight:100 900;font-style:normal}`;
const fonts = font('Geist', 'geist-latin.woff2') + font('Newsreader', 'newsreader-latin.woff2') + font('GeistMono', 'geist-mono-latin.woff2');
const styles = `${fonts}
  *{box-sizing:border-box}body{margin:0;background:#f4efe6;color:#0d2135;font:15px/1.45 Geist,system-ui,sans-serif}
  h1,h2,h3,p{margin:0}h1{font:500 54px/1.04 Newsreader,Georgia,serif;letter-spacing:-.025em}
  h2{font:500 34px/1.05 Newsreader,Georgia,serif;letter-spacing:-.02em}h3{font:600 16px/1.3 Geist,system-ui,sans-serif}
  code,.mono{font-family:GeistMono,Menlo,monospace}code{font-size:11px}.kicker{font:500 11px/1.4 GeistMono,monospace;text-transform:uppercase;letter-spacing:.12em}
  .muted{color:#4b5b6b}.page{padding:44px 48px}.mast{display:flex;justify-content:space-between;align-items:center;padding-bottom:18px;border-bottom:1px solid #d8cebb}
  .mast b{font:500 25px/1 Newsreader,Georgia,serif;letter-spacing:-.015em}.mast span{font:500 11px/1.4 GeistMono,monospace;color:#4b5b6b;text-transform:uppercase;letter-spacing:.1em}
  .intro{margin:30px 0 34px}.intro p{margin-top:12px;color:#4b5b6b;max-width:920px;font-size:16px;line-height:1.55}
  svg{display:block;flex:none}.night{background:#071320;color:#ece6d9}.night .muted{color:#94a4b5}.night .kicker{color:#94a4b5}
  .foot{border-top:1px solid #d8cebb;margin-top:28px;padding-top:16px;font-size:12px;color:#4b5b6b;display:flex;justify-content:space-between;gap:24px}
  .sizes{display:flex;align-items:end;gap:30px}.size{display:flex;flex-direction:column;align-items:center;gap:10px}.size span{font:500 10px/1.3 GeistMono,monospace;color:#4b5b6b}
  .night .size span{color:#94a4b5}.address{display:flex;align-items:center;gap:10px}.address code{font-size:10px;white-space:nowrap}
`;
const mast = label => `<div class="mast"><b>EastSea</b><span>${escape(label)}</span></div>`;
const sizeRow = (render, address) => `<div class="sizes">${[16, 32, 64].map(size => `<div class="size">${render(address, size)}<span>${size} px</span></div>`).join('')}</div>`;
const server = createServer((request, response) => {
  const url = new URL(request.url, 'http://127.0.0.1');
  if (url.pathname === '/favicon.ico') { response.writeHead(204); response.end(); return; }
  if (url.pathname === '/explorer-fixture') {
    // The real index shell and stylesheet, with startup/RPC replaced by the
    // bounded accountView fixture below. Header classes match js/app.js.
    const shell = readFileSync(path.join(root, 'apps/explorer/index.html'), 'utf8')
      .replace('<head>', '<head><base href="/apps/explorer/">')
      .replace('<header id="top"></header>', '<header id="top"><a class="brand" href="#/"><span class="logo" aria-hidden="true"></span><span class="brand-name">EastSea Explorer</span></a><span class="pill" id="chain">chain 7780</span><span class="pill plain" id="source">offline fixture</span><form id="search" role="search"><input id="q" type="search" placeholder="Height, 0x address or tx hash" aria-label="Search"><button type="submit">Search</button><span id="search-msg" class="small"></span></form><details id="settings"><summary>Settings</summary></details><button class="ghost" title="Switch theme">◐</button></header>')
      .replace('<script type="module" src="js/app.js"></script>', '');
    response.setHeader('Content-Type', 'text/html; charset=utf-8'); response.end(shell); return;
  }
  if (!/^\/apps\/(extension|explorer)\//.test(url.pathname)) { response.writeHead(404); response.end(); return; }
  const filename = path.resolve(root, `.${decodeURIComponent(url.pathname)}`);
  if (!filename.startsWith(path.join(root, 'apps') + path.sep)) { response.writeHead(403); response.end(); return; }
  try {
    response.setHeader('Content-Type', ({ '.js': 'text/javascript', '.css': 'text/css', '.html': 'text/html', '.json': 'application/json', '.png': 'image/png' })[path.extname(filename)] || 'application/octet-stream');
    response.end(readFileSync(filename));
  } catch { response.writeHead(404); response.end(); }
});
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
const origin = `http://127.0.0.1:${server.address().port}`;
const profile = mkdtempSync(path.join(temporary, 'chromium-'));
const context = await chromium.launchPersistentContext(profile, {
  headless: true, executablePath: process.env.CHROME_BINARY || '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',
  viewport: { width: 1440, height: 1080 }, deviceScaleFactor: 1, serviceWorkers: 'block',
}).catch(async error => {
  await new Promise(resolve => server.close(resolve));
  rmSync(profile, { recursive: true, force: true });
  throw error;
});
const checks = [];
const sheets = [];
const externalRequests = [];
await context.route('**/*', route => {
  const url = route.request().url();
  if (url.startsWith(origin + '/') || url.startsWith('data:') || url === 'about:blank') return route.continue();
  externalRequests.push(url); return route.abort();
});

async function sheet(_page, filename, body, css = '', width = 1440) {
  // Isolate large sheets from the repeatedly resized native-icon page. Chrome
  // can otherwise retain a stale compositor tile below the original viewport.
  const page = await context.newPage();
  const errors = [];
  const onError = error => errors.push(error.message);
  page.on('pageerror', onError);
  await page.setViewportSize({ width, height: 1080 });
  await page.emulateMedia({ colorScheme: 'light', reducedMotion: 'reduce' });
  await page.setContent(`<!doctype html><meta charset="utf-8"><style>${styles}${css}</style>${body}`);
  await page.evaluate(() => document.fonts.ready);
  const dimensions = await page.evaluate(() => ({ width: document.documentElement.scrollWidth, height: document.documentElement.scrollHeight }));
  if (dimensions.width > width || errors.length) throw new Error(JSON.stringify({ filename, dimensions, width, errors }));
  await page.setViewportSize({ width, height: dimensions.height });
  await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
  await page.screenshot({ path: filename });
  page.off('pageerror', onError);
  sheets.push({ path: path.relative(root, filename), viewportWidth: width, documentWidth: dimensions.width, height: dimensions.height, pageErrors: errors });
  await page.close();
}

async function snapshot(page, svg, filename, background) {
  await page.setViewportSize({ width: 96, height: 96 });
  await page.setContent(`<style>body{margin:0;background:${background}}svg{display:block}</style>${svg}`);
  await page.locator('svg').screenshot({ path: filename });
}

async function browserSnapshots(page) {
  for (const theme of ['light', 'dark']) {
    const directory = path.join(browserOutput, theme); mkdirSync(directory, { recursive: true });
    for (const [index, vector] of fixture.vectors.entries()) for (const size of [16, 32, 64]) {
      await snapshot(page, icon(vector.address, size), path.join(directory, `vector-${String(index).padStart(2, '0')}-${size}.png`), fixture.backgrounds[theme]);
    }
    const rows = fixture.vectors.map((vector, index) => {
      const spec = deriveAccountIcon(vector.address);
      return `<article>${sizeRow(icon, vector.address)}<code>${String(index).padStart(2, '0')} · ${escape(vector.address)}</code><div class="tuple">${escape(ACCOUNT_ICON_PALETTES[spec.palette].name)} · ${escape(ACCOUNT_ICON_SILHOUETTES[accountIconSilhouette(spec)].name)} · ${spec.rotation * 90}°</div></article>`;
    }).join('');
    await sheet(page, path.join(browserOutput, `browser-review-${theme}.png`), `<main class="page ${theme === 'dark' ? 'night' : ''}">${mast('Account identity / canonical SVG')}<div class="intro"><div class="kicker">Archipelago v${ACCOUNT_ICON_VERSION} · ${theme}</div><h1>Sixteen addresses, native sizes.</h1><p>One broad coastline below 32 px. Two neighboring islands appear at 32 px and above. Frozen shared vectors shown at actual 16 / 32 / 64 px.</p></div><section>${rows}</section><div class="foot"><span>Canonical browser rendering · 1 CSS px = 1 image px</span><span>Color + coastline + rotation carry the first impression.</span></div></main>`, 'section{display:grid;grid-template-columns:repeat(4,1fr);gap:30px 24px}article{min-height:160px;border-top:1px solid #d8cebb;padding-top:20px}article code{display:block;font-size:9px;margin-top:18px}.tuple{font-size:11px;color:#4b5b6b;margin-top:7px}.night .mast,.night article,.night .foot{border-color:#1f3347}.night .intro p,.night .tuple,.night .mast span,.night .foot{color:#94a4b5}.sizes{height:88px;gap:24px}', 1440);
  }
  writeFileSync(path.join(browserOutput, 'render-info.json'), JSON.stringify({ renderer: 'Chromium / Playwright, canonical accountIconSVG', chromium: context.browser().version(), accountIconVersion: ACCOUNT_ICON_VERSION, ...provenance, scale: 1, vectorCount: 16, snapshotCount: 96, sizes: [16, 32, 64], themes: ['light', 'dark'] }, null, 2) + '\n');
}

// Alternative geometry belongs only to this review script; it is never served
// by an extension, explorer, site or native wallet implementation.
function staticDirection(direction, address, size) {
  const spec = deriveAccountIcon(address), palette = ACCOUNT_ICON_PALETTES[spec.palette];
  const id = `review-${direction}-${spec.palette}`;
  const waves = [
    'M 7 39 C 16 39 20 13 35 13 C 46 13 54 20 56 31 C 48 26 40 29 37 36 C 31 50 15 52 7 46 Z',
    'M 7 19 C 16 8 23 9 32 19 C 41 29 49 28 57 17 L 57 34 C 46 44 39 40 31 33 C 23 25 16 28 7 37 Z',
    'M 7 38 C 21 12 38 8 57 22 L 54 35 C 34 18 24 41 7 49 Z',
  ];
  const contours = direction === 'waves'
    ? `<path d="${waves[spec.shape % waves.length]}"/>${size >= 32 ? '<path opacity=".8" d="M 8 53 C 22 40 40 57 56 44 L 56 52 C 37 64 23 47 8 60 Z"/>' : ''}`
    : '<path d="M 32 8 L 48 45 L 32 35 L 17 46 Z"/>' + (size >= 32 ? '<path d="M 9 17 L 18 23 L 12 38 L 5 29 Z"/><path d="M 51 16 L 58 29 L 52 38 L 46 24 Z"/>' : '');
  return `<svg xmlns="http://www.w3.org/2000/svg" width="${size}" height="${size}" viewBox="0 0 64 64" aria-hidden="true"><defs><linearGradient id="${id}" x1="0" y1="0" x2="64" y2="64" gradientUnits="userSpaceOnUse"><stop stop-color="${palette.start}"/><stop offset="1" stop-color="${palette.end}"/></linearGradient></defs><rect width="64" height="64" rx="12" fill="url(#${id})"/><g fill="${palette.ink}" transform="rotate(${spec.rotation * 90} 32 32)">${contours}</g></svg>`;
}

async function directions(page) {
  const directions = [
    { name: 'Islands', render: icon, status: 'Implemented · recommended', title: 'A coastline to remember.', body: 'Sixteen distinct coastlines. At 16 px, a single large landform carries the silhouette. At 32 px, two neighboring islands extend it.', detail: 'Coastal identity / strongest structural variety' },
    { name: 'Waves', render: (a, s) => staticDirection('waves', a, s), status: 'Static alternative', title: 'Broad ocean contours.', body: 'A crest or current becomes the primary shape. One sweeping band at 16 px; a second band joins at larger sizes.', detail: 'Sea motion / softer silhouette vocabulary' },
    { name: 'Navigation', render: (a, s) => staticDirection('navigation', a, s), status: 'Static alternative', title: 'A heading on the sea.', body: 'One broad heading marker at 16 px. At larger sizes, two outer bearings form a three-part navigational emblem.', detail: 'Maritime wayfinding / directional geometry' },
  ];
  const columns = directions.map(({ name, render, status, title, body, detail }, index) => `<article><div class="direction-top"><span class="kicker">0${index + 1}</span><span class="status ${index === 0 ? 'chosen' : ''}">${status}</span></div><h2>${name}</h2><div class="hero-icon">${render(ADDRESSES.sender, 144)}<span class="kicker">Enlarged form · 144 px</span></div>${sizeRow(render, ADDRESSES.sender)}<div class="native-list">${[ADDRESSES.sender, ADDRESSES.contract, ADDRESSES.recipient].map(address => `<div class="address">${render(address, 16)}<code>${escape(short(address))}</code></div>`).join('')}<span class="kicker">Native 16 px · three fixed addresses</span></div><div class="night dark-context">${sizeRow(render, ADDRESSES.contract)}<div class="kicker">Night sea · same colors, new context</div></div><h3>${title}</h3><p class="body">${body}</p><div class="detail">${detail}</div></article>`).join('');
  await sheet(page, path.join(reviewOutput, 'directions.png'), `<main class="page">${mast('Account icon / founder style directions')}<div class="intro"><div class="kicker">Round two · visual direction</div><h1>Addresses as a piece of the sea.</h1><p>Three directions in EastSea’s parchment and night-sea context. All use the same fixed address colors, with native 16 / 32 / 64 px examples for comparison.</p></div><section class="directions">${columns}</section><div class="foot"><span>Islands is the canonical implementation.</span><span>Waves and Navigation are static studies; their recognition and parity have not been measured.</span></div></main>`, '.directions{display:grid;grid-template-columns:repeat(3,1fr)}article{padding:0 32px;border-left:1px solid #d8cebb}article:first-child{padding-left:0;border-left:0}article:last-child{padding-right:0}.direction-top{display:flex;align-items:center;justify-content:space-between;margin-bottom:16px}.status{font-size:11px;color:#4b5b6b}.status.chosen{color:#0f5a75;font-weight:600}article h2{font-size:42px}.hero-icon{display:flex;flex-direction:column;align-items:center;gap:18px;margin:30px 0 32px}.hero-icon .kicker{font-size:9px;color:#4b5b6b}.sizes{justify-content:center;height:90px;gap:34px}.native-list{margin:30px 0 28px;display:flex;flex-direction:column;gap:11px}.native-list .kicker{font-size:9px;color:#4b5b6b;margin-top:5px}.dark-context{padding:22px 10px 20px;margin-bottom:28px;border-radius:12px}.dark-context .sizes{height:80px}.dark-context .kicker{text-align:center;font-size:9px;margin-top:17px}.body{font-size:13px;color:#4b5b6b;line-height:1.6;margin-top:9px;min-height:86px}.detail{font:500 10px/1.5 GeistMono,monospace;color:#0f5a75;margin-top:15px}', 1536);
}

async function atlas(page) {
  const palettes = ACCOUNT_ICON_PALETTES.map((palette, index) => {
    const spec = { version: ACCOUNT_ICON_VERSION, palette: index, shape: 0, layout: 0, rotation: 0 };
    return `<article><div class="palette-icons">${accountIconSVG(spec, 64)}<div class="size">${accountIconSVG(spec, 16)}<span>16 px</span></div></div><h3>${String(index).padStart(2, '0')} ${escape(palette.name)}</h3><code>${palette.start} → ${palette.end}</code><code>Land ${palette.ink}</code></article>`;
  }).join('');
  const shapes = ACCOUNT_ICON_SILHOUETTES.map((silhouette, index) => {
    const spec = { version: ACCOUNT_ICON_VERSION, palette: 7, shape: index >> 2, layout: index & 3, rotation: 0 };
    return `<article><div class="shape-icons">${accountIconSVG(spec, 48)}${accountIconSVG(spec, 16)}</div><div><h3>${String(index).padStart(2, '0')} ${escape(silhouette.name)}</h3><span>One form at 16 px</span></div></article>`;
  }).join('');
  await sheet(page, path.join(reviewOutput, 'atlas.png'), `<main class="page">${mast('Account identity / palette and silhouette atlas')}<div class="intro"><div class="kicker">Archipelago v${ACCOUNT_ICON_VERSION}</div><h1>Color is one cue. Coastline is another.</h1><p>Sixteen hue families, paired with contrasting navy or sand. Sixteen broad silhouettes; each can face four directions. Shown independently here to expose the visual vocabulary.</p></div><div class="section-head"><h2>The coastal palette</h2><span class="kicker">Same silhouette / native 16 + 64 px</span></div><section class="palettes">${palettes}</section><div class="section-head forms-head"><h2>The coastline classes</h2><span class="kicker">Same hue / native 16 + 48 px</span></div><section class="shapes">${shapes}</section><div class="foot"><span>Gradients and land colors are identical in light and dark appearances.</span><span>Detail is additive above 32 px.</span></div></main>`, '.section-head{display:flex;justify-content:space-between;align-items:baseline;margin-bottom:22px}.section-head h2{font-size:30px}.section-head .kicker{font-size:10px;color:#4b5b6b}.palettes{display:grid;grid-template-columns:repeat(8,1fr);gap:25px 22px}.palettes article{border-top:1px solid #d8cebb;padding-top:14px}.palette-icons{display:flex;align-items:center;gap:16px;margin-bottom:12px}.palettes h3{font-size:13px;margin-bottom:5px}.palettes code{display:block;color:#4b5b6b;font-size:9px;line-height:1.6}.forms-head{margin-top:38px}.shapes{display:grid;grid-template-columns:repeat(4,1fr);gap:20px 22px}.shapes article{display:flex;align-items:center;gap:16px;border-top:1px solid #d8cebb;padding-top:15px}.shape-icons{display:flex;align-items:center;gap:12px}.shapes h3{font-size:12px}.shapes span{font-size:10px;color:#4b5b6b}', 1536);
}

async function beforeAfter(page, hasCurrentSurface) {
  const nativeDirectory = path.join(reviewOutput, 'native16'); mkdirSync(nativeDirectory, { recursive: true });
  const roles = [['FROM / sender', ADDRESSES.sender], ['TO / contract', ADDRESSES.contract], ['CALL / recipient', ADDRESSES.recipient]];
  for (const [index, [, address]] of roles.entries()) {
    await snapshot(page, baseline.accountIconSVG(baseline.deriveAccountIcon(address), 16), path.join(nativeDirectory, `before-${index}.png`), '#071320');
    await snapshot(page, icon(address, 16), path.join(nativeDirectory, `after-${index}.png`), '#071320');
  }
  const column = current => {
    const native = roles.map(([role, address], index) => {
      const raster = imageData(path.join(nativeDirectory, `${current ? 'after' : 'before'}-${index}.png`));
      const spec = current ? deriveAccountIcon(address) : baseline.deriveAccountIcon(address);
      const description = current ? `${ACCOUNT_ICON_PALETTES[spec.palette].name} · ${ACCOUNT_ICON_SILHOUETTES[accountIconSilhouette(spec)].name} · ${spec.rotation * 90}°` : `palette ${spec.palette} · glyph ${spec.shape}`;
      return `<div class="comparison-row"><div class="magnified"><img src="${raster}" width="96" height="96"><span>16 px · 6× pixels</span></div><div class="identity"><span class="kicker">${role}</span><div class="address"><img src="${raster}" width="16" height="16"><code>${escape(address)}</code></div><div class="descriptor">${escape(description)}</div></div></div>`;
    }).join('');
    const surfaceFile = current ? path.join(surfaceOutput, 'extension-approval-dark.png') : baselineSurface;
    const surface = existsSync(surfaceFile) && (!current || hasCurrentSurface) ? `<div class="surface-image"><img src="${imageData(surfaceFile)}" width="360"><div><span class="kicker">Product surface / 24 px icons</span><h3>Extension approval</h3><p>The authoritative addresses remain fully visible. Product layout, typography, controls and icon dimensions stay unchanged.</p></div></div>` : '';
    return `<article><div class="column-title"><span class="kicker">${current ? 'After / Archipelago v2' : 'Before / Archipelago v1'}</span><h2>${current ? 'One readable coastline.' : 'Tiny repeated glyphs.'}</h2><p>${current ? 'Sea / dawn, opposing contrast and rotated hook outlines distinguish To / From.' : 'To and From share teal, with small black marks.'}</p></div><div class="night native-rows">${native}</div>${surface}</article>`;
  };
  await sheet(page, path.join(reviewOutput, 'before-after.png'), `<main class="page">${mast('Account identity / same-address comparison')}<div class="intro"><div class="kicker">Lead review example · 6cd6fee → current</div><h1>The distinction lives in the large form.</h1><p>The same three addresses, at actual 16 px and as nearest-neighbor pixel enlargements. The review’s problematic To / From pair is preserved, with its product approval surface below.</p></div><section class="comparison">${column(false)}${column(true)}</section><div class="foot"><span>16 px samples are actual Chromium raster crops, enlarged without smoothing.</span><span>The icon is a recognition aid; the full address remains authoritative.</span></div></main>`, '.comparison{display:grid;grid-template-columns:1fr 1fr;gap:36px}.column-title{margin-bottom:20px}.column-title .kicker{color:#4b5b6b}.column-title h2{margin-top:8px;font-size:34px}.column-title p{margin-top:9px;font-size:13px;color:#4b5b6b}.native-rows{padding:20px;border-radius:12px}.comparison-row{display:flex;align-items:center;gap:24px;margin:0 0 20px}.comparison-row:last-child{margin-bottom:0}.magnified{display:flex;flex-direction:column;gap:8px;flex:none}.magnified img{image-rendering:pixelated}.magnified span{font:500 8px/1.4 GeistMono,monospace;color:#94a4b5}.identity{min-width:0}.identity .kicker{font-size:9px}.identity .address{gap:9px;margin:10px 0}.identity .address code{font-size:9px;white-space:normal;overflow-wrap:anywhere}.descriptor{font-size:11px;color:#94a4b5}.surface-image{display:flex;gap:24px;margin-top:28px;align-items:start}.surface-image>img{display:block;border-radius:12px;flex:none}.surface-image>div{flex:1;padding-top:16px}.surface-image .kicker{font-size:9px;color:#4b5b6b}.surface-image h3{margin-top:12px;font-size:16px}.surface-image p{font-size:12px;line-height:1.6;color:#4b5b6b;margin-top:12px}', 1536);
}

async function verifySurface(page, surface, theme, expectedAddresses, errors, consoleErrors) {
  const details = await page.evaluate(() => ({
    documentWidth: document.documentElement.scrollWidth,
    icons: [...document.querySelectorAll('svg.account-icon')].map(svg => {
      const bounds = svg.getBoundingClientRect();
      return { width: bounds.width, height: bounds.height, requestedWidth: svg.getAttribute('width'), ariaHidden: svg.getAttribute('aria-hidden'), focusable: svg.getAttribute('focusable'), role: svg.getAttribute('role'), titleCount: svg.querySelectorAll('title').length, fullAddress: svg.parentElement.querySelector('.mono')?.textContent?.trim() || null };
    }),
    identities: [...document.querySelectorAll('.account-identity > .mono, .account-heading .mono')].map(el => el.textContent.trim()),
    operations: window.__reviewOperations || [], unexpectedOperations: window.__reviewUnexpectedOperations || [],
    rpcMethods: window.__reviewRPCMethods || [],
  }));
  const viewportWidth = page.viewportSize().width;
  const failed = details.documentWidth > viewportWidth || errors.length || consoleErrors.length
    || details.unexpectedOperations.length || !details.icons.length
    || details.icons.some(svg => svg.ariaHidden !== 'true' || svg.focusable !== 'false' || svg.role === 'img' || svg.titleCount)
    || expectedAddresses.some(address => !details.identities.includes(address));
  const check = { surface, theme, viewportWidth, ...details, pageErrors: errors, consoleErrors, expectedFullAddresses: expectedAddresses, passed: !failed };
  checks.push(check);
  if (failed) throw new Error(JSON.stringify(check));
  await page.screenshot({ path: path.join(surfaceOutput, `${surface}-${theme}.png`), fullPage: true });
}

async function surfaces() {
  for (const theme of ['light', 'dark']) {
    for (const approval of [false, true]) {
      const page = await context.newPage(), errors = [], consoleErrors = [];
      page.on('pageerror', error => errors.push(error.message));
      page.on('console', message => { if (message.type() === 'error') consoleErrors.push(message.text()); });
      await page.setViewportSize({ width: 360, height: 740 });
      await page.emulateMedia({ colorScheme: theme, reducedMotion: 'reduce' });
      await page.addInitScript(({ sender, recipient, contract, approval }) => {
        window.__reviewOperations = []; window.__reviewUnexpectedOperations = [];
        const state = { terms: 4, exists: true, unlocked: true, address: sender, defaultChainId: 7780, developmentNetwork: false, approvals: approval ? [{ id: 'fixture', kind: 'transaction', origin: 'https://trusted.example', what: 'Token approval (allows spending)', value: '0', tx: { to: contract, data: '0x095ea7b3' + recipient.slice(2).padStart(64, '0') + '1'.padStart(64, '0') } }] : [] };
        window.chrome = { runtime: { async sendMessage({ op }) {
          window.__reviewOperations.push(op);
          let result;
          if (op === 'state') result = state;
          else if (op === 'account') result = { balance: '12300000000000000000', height: 42, blockAt: Date.now() };
          else if (op === 'quote') result = '1000000000000';
          else { window.__reviewUnexpectedOperations.push(op); throw new Error('Unexpected offline operation: ' + op); }
          return { ok: true, result };
        } } };
      }, { ...ADDRESSES, approval });
      await page.goto(`${origin}/apps/extension/ui/popup.html${approval ? '?approve=fixture' : ''}`, { waitUntil: 'networkidle' });
      await page.locator('svg.account-icon').first().waitFor();
      await verifySurface(page, approval ? 'extension-approval' : 'extension-home', theme, approval ? Object.values(ADDRESSES) : [ADDRESSES.sender], errors, consoleErrors);
      await page.close();
    }
    for (const width of [360, 1200]) {
      const page = await context.newPage(), errors = [], consoleErrors = [];
      page.on('pageerror', error => errors.push(error.message));
      page.on('console', message => { if (message.type() === 'error') consoleErrors.push(message.text()); });
      await page.setViewportSize({ width, height: 800 });
      await page.emulateMedia({ colorScheme: theme, reducedMotion: 'reduce' });
      await page.goto(origin + '/explorer-fixture', { waitUntil: 'networkidle' });
      await page.evaluate(async address => {
        const { accountView } = await import('/apps/explorer/js/pages.js');
        window.__reviewRPCMethods = [];
        const ctx = { chainId: 7780, node: { url: 'http://offline.test', async call(method) {
          window.__reviewRPCMethods.push(method);
          if (method === 'aether_getAccount') return { balance: '12300000000000000000', nonce: 4, code_size: 0, height: 42, state_root: 'a'.repeat(64) };
          if (method === 'aether_rewards' || method === 'eth_getLogs') return [];
          if (method === 'eth_blockNumber') return '0x2a';
          throw new Error('Unexpected offline RPC: ' + method);
        } }, async read() { throw new Error('Fixture address is not a token.'); } };
        document.getElementById('view').replaceChildren(await accountView(ctx, address));
      }, ADDRESSES.recipient);
      await verifySurface(page, width === 360 ? 'explorer-mobile' : 'explorer-desktop', theme, [ADDRESSES.recipient], errors, consoleErrors);
      await page.close();
    }
  }
  writeFileSync(path.join(surfaceOutput, 'checks.json'), JSON.stringify({ backend: 'Offline bounded RPC and chrome.runtime fixture; real product DOM/CSS; no wallet, live node, signing or external requests', ...provenance, addresses: ADDRESSES, externalRequests, checks }, null, 2) + '\n');
}

try {
  const page = await context.newPage();
  await atlas(page);
  await directions(page);
  if (!prototype) {
    await browserSnapshots(page);
    await surfaces();
  }
  await beforeAfter(page, !prototype);
  if (externalRequests.length) throw new Error('Unexpected external request(s): ' + externalRequests.join(', '));
  writeFileSync(path.join(reviewOutput, 'render-info.json'), JSON.stringify({ accountIconVersion: ACCOUNT_ICON_VERSION, ...provenance, baselineCommit: '6cd6fee', renderer: 'Chromium / Playwright', chromium: context.browser().version(), scale: 1, prototype, addresses: ADDRESSES, staticAlternatives: ['Waves', 'Navigation'], externalRequests, sheets, surfaceCount: checks.length }, null, 2) + '\n');
  console.log(JSON.stringify({ output: path.relative(root, output), sheets: sheets.length, browserSnapshots: prototype ? 0 : 96, surfaces: checks.length, externalRequests: externalRequests.length }));
} finally {
  await context.close();
  await new Promise(resolve => server.close(resolve));
  rmSync(profile, { recursive: true, force: true });
}
