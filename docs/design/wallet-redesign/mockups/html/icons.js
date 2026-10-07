// Stroke icons standing in for SF Symbols in the mockups (spec.md §5.6 maps
// each to its SF Symbol name). Usage: <svg><use href="#i-send"/></svg>
const P = {
  home: '<path d="M4 10.5 12 4l8 6.5V19a1 1 0 0 1-1 1h-4.5v-5.5h-5V20H5a1 1 0 0 1-1-1z"/>',
  activity: '<circle cx="12" cy="12" r="8"/><path d="M12 7.5V12l3 2"/>',
  explore: '<circle cx="12" cy="12" r="8"/><path d="m15.5 8.5-2 5-5 2 2-5z"/>',
  node: '<rect x="4" y="5" width="16" height="10.5" rx="1.6"/><path d="M9 19h6M12 15.5V19"/>',
  shield: '<path d="M12 3.5 5 6v5.5c0 4.3 3 7.6 7 9 4-1.4 7-4.7 7-9V6z"/><path d="m9 12 2.2 2.2L15.5 10"/>',
  agent: '<rect x="4" y="5" width="16" height="13" rx="3"/><path d="m8 10 2.5 2L8 14M12.5 14.5H16"/>',
  send: '<path d="M7 17 17 7M9 7h8v8"/>',
  receive: '<path d="M17 7 7 17M15 17H7V9"/>',
  assets: '<path d="m12 4 8 4-8 4-8-4z"/><path d="m4 12 8 4 8-4M4 16l8 4 8-4"/>',
  touchid: '<path d="M7.5 6.5A7 7 0 0 1 19 12v1.5M5 10a7 7 0 0 0-.2 1.8V14M8.5 19.5c.9-1.6 1.3-3.4 1.3-5.5V12a2.2 2.2 0 0 1 4.4 0v2c0 1.2-.1 2.4-.4 3.5M12 12v2c0 2.6-.6 4.6-1.6 6.2M16.6 16.8c-.2 1.2-.6 2.3-1 3.3M12 7.8A4.2 4.2 0 0 1 16.2 12v.8"/>',
  check: '<path d="m5 12.5 4.5 4.5L19 7.5"/>',
  checkseal: '<path d="m12 3 2.2 1.6 2.7-.1.8 2.6 2.2 1.6-.9 2.6.9 2.6-2.2 1.6-.8 2.6-2.7-.1L12 21l-2.2-1.6-2.7.1-.8-2.6L4.1 15.3l.9-2.6-.9-2.6 2.2-1.6.8-2.6 2.7.1z"/><path d="m8.8 12.2 2.2 2.2 4.2-4.4"/>',
  copy: '<rect x="8" y="8" width="11" height="11" rx="2"/><path d="M5 15V6a1 1 0 0 1 1-1h9"/>',
  chev: '<path d="m9.5 6 6 6-6 6"/>',
  chevdown: '<path d="m6 9.5 6 6 6-6"/>',
  updown: '<path d="m8 9.5 4-4 4 4M8 14.5l4 4 4-4"/>',
  warn: '<path d="M12 4 3 19.5h18z"/><path d="M12 10v4.2M12 17h.01"/>',
  info: '<circle cx="12" cy="12" r="8.5"/><path d="M12 11v5M12 8h.01"/>',
  qr: '<rect x="4" y="4" width="6" height="6" rx="1"/><rect x="14" y="4" width="6" height="6" rx="1"/><rect x="4" y="14" width="6" height="6" rx="1"/><path d="M14 14h2v2h-2zM18 18h2v2h-2zM14 18h2M18 14h2"/>',
  gear: '<circle cx="12" cy="12" r="3"/><path d="M12 3v2.5M12 18.5V21M3 12h2.5M18.5 12H21M5.6 5.6l1.8 1.8M16.6 16.6l1.8 1.8M5.6 18.4l1.8-1.8M16.6 7.4l1.8-1.8"/>',
  search: '<circle cx="11" cy="11" r="6"/><path d="m16 16 4 4"/>',
  refresh: '<path d="M19 12a7 7 0 1 1-2.1-5M19 4.5V8h-3.5"/>',
  lock: '<rect x="5.5" y="10.5" width="13" height="9.5" rx="2"/><path d="M8.5 10.5V8a3.5 3.5 0 0 1 7 0v2.5"/>',
  key: '<circle cx="8.5" cy="12" r="3.5"/><path d="M12 12h8.5M17.5 12v3M20 12v2"/>',
  devices: '<rect x="3" y="5" width="13" height="9" rx="1.5"/><path d="M6 17.5h7"/><rect x="17" y="9" width="4.5" height="9.5" rx="1.2"/>',
  x: '<path d="m7 7 10 10M17 7 7 17"/>',
  plus: '<path d="M12 5v14M5 12h14"/>',
  download: '<path d="M12 4v11M7.5 10.5 12 15l4.5-4.5M5 19.5h14"/>',
  globe: '<circle cx="12" cy="12" r="8.5"/><path d="M3.5 12h17M12 3.5c2.5 2.6 2.5 14.4 0 17M12 3.5c-2.5 2.6-2.5 14.4 0 17"/>',
  bolt: '<path d="M13 3 5.5 13.5H12L11 21l7.5-10.5H12z"/>',
  wifioff: '<path d="M4 9a12 12 0 0 1 4-2.4M11 6a12 12 0 0 1 9 3M7 12.5a7 7 0 0 1 2.5-1.4M14.5 11.2A7 7 0 0 1 17 12.5M10 16a3 3 0 0 1 4 0M4 4l16 16"/>',
  pause: '<circle cx="12" cy="12" r="8.5"/><path d="M10 9v6M14 9v6"/>',
  person: '<circle cx="12" cy="8.5" r="3.5"/><path d="M5 20c.8-3.8 3.6-5.5 7-5.5s6.2 1.7 7 5.5"/>',
  doc: '<path d="M7 3.5h7l4 4V20a.5.5 0 0 1-.5.5h-10.5A.5.5 0 0 1 6.5 20V4a.5.5 0 0 1 .5-.5z"/><path d="M14 3.5V8h4M9.5 12.5h5M9.5 16h5"/>',
  share: '<path d="M12 4v11M8 7.5 12 4l4 3.5M6 11v8.5h12V11"/>',
  stop: '<rect x="6" y="6" width="12" height="12" rx="2.5"/>',
  sun: '<circle cx="12" cy="12" r="3.5"/><path d="M12 3.5v2M12 18.5v2M3.5 12h2M18.5 12h2M6 6l1.4 1.4M16.6 16.6 18 18M6 18l1.4-1.4M16.6 7.4 18 6"/>',
  arrowcw: '<path d="M18.5 8.5A7 7 0 1 0 19 13M19 4v4.5h-4.5"/>',
  calendar: '<rect x="4" y="5.5" width="16" height="14" rx="2"/><path d="M4 10h16M8.5 3.5v4M15.5 3.5v4"/>',
};
const sprite = Object.entries(P).map(([k, v]) =>
  `<symbol id="i-${k}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">${v}</symbol>`).join('');
document.body.insertAdjacentHTML('afterbegin', `<svg width="0" height="0" style="position:absolute">${sprite}</svg>`);

// Engraved wave lines for the balance plate (the coin's sea, as linework).
window.waves = (w = 900, h = 220, n = 7) => {
  let d = '';
  for (let i = 0; i < n; i++) {
    const y = 40 + i * (h - 40) / n, a = 6 + i * 1.2, len = 120 + i * 18;
    let p = `M0 ${y}`;
    for (let x = 0; x <= w; x += len / 2) p += ` Q ${x + len / 4} ${y - a} ${x + len / 2} ${y}`;
    d += `<path d="${p}" />`;
  }
  return `<svg class="waves" viewBox="0 0 ${w} ${h}" preserveAspectRatio="none" fill="none" stroke="rgba(232,191,89,.20)" stroke-width="1">${d}</svg>`;
};
document.querySelectorAll('[data-waves]').forEach(el => el.insertAdjacentHTML('afterbegin', waves()));
