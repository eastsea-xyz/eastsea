// Color measurements for the account-icon reports. Inputs are sRGB hex colors
// or normalized [r, g, b] triples; Lab uses the D65 reference white.
function srgb(color) {
  if (typeof color === 'string' && /^#[0-9a-f]{6}$/i.test(color)) {
    return [1, 3, 5].map((offset) => parseInt(color.slice(offset, offset + 2), 16) / 255);
  }
  if (Array.isArray(color) && color.length === 3 && color.every((channel) => Number.isFinite(channel) && channel >= 0 && channel <= 1)) {
    return color;
  }
  throw new TypeError('color must be #rrggbb or three normalized sRGB channels');
}

function linear(channel) {
  return channel <= 0.04045 ? channel / 12.92 : ((channel + 0.055) / 1.055) ** 2.4;
}

export function luminance(color) {
  const [r, g, b] = srgb(color).map(linear);
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

export function contrast(first, second) {
  const a = luminance(first), b = luminance(second);
  return (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05);
}

export function interpolateColor(first, second, amount = 0.5) {
  if (!Number.isFinite(amount) || amount < 0 || amount > 1) throw new RangeError('interpolation amount must be 0..1');
  const a = srgb(first).map((channel) => channel * 255), b = srgb(second).map((channel) => channel * 255);
  return '#' + a.map((channel, index) => Math.round(channel + amount * (b[index] - channel)).toString(16).padStart(2, '0')).join('');
}

export function srgbToLab(color) {
  const [r, g, b] = srgb(color).map(linear);
  const x = (0.4124564 * r + 0.3575761 * g + 0.1804375 * b) / 0.95047;
  const y = 0.2126729 * r + 0.7151522 * g + 0.0721750 * b;
  const z = (0.0193339 * r + 0.1191920 * g + 0.9503041 * b) / 1.08883;
  const f = (value) => value > 216 / 24389 ? Math.cbrt(value) : (24389 / 27 * value + 16) / 116;
  const fx = f(x), fy = f(y), fz = f(z);
  return [116 * fy - 16, 500 * (fx - fy), 200 * (fy - fz)];
}

// CIEDE2000 with the reference viewing-condition weights kL = kC = kH = 1.
// Sharma, Wu & Dalal (2005): https://hajim.rochester.edu/ece/sites/gsharma/ciede2000/
export function deltaE2000(first, second) {
  for (const lab of [first, second]) {
    if (!Array.isArray(lab) || lab.length !== 3 || !lab.every(Number.isFinite)) throw new TypeError('Lab must contain three finite numbers');
  }
  const [l1, a1, b1] = first, [l2, a2, b2] = second;
  const radians = Math.PI / 180;
  const c1 = Math.hypot(a1, b1), c2 = Math.hypot(a2, b2);
  const meanC7 = ((c1 + c2) / 2) ** 7;
  const g = 0.5 * (1 - Math.sqrt(meanC7 / (meanC7 + 25 ** 7)));
  const ap1 = (1 + g) * a1, ap2 = (1 + g) * a2;
  const cp1 = Math.hypot(ap1, b1), cp2 = Math.hypot(ap2, b2);
  const hue = (a, b) => a === 0 && b === 0 ? 0 : (Math.atan2(b, a) / radians + 360) % 360;
  const h1 = hue(ap1, b1), h2 = hue(ap2, b2);
  const dl = l2 - l1, dc = cp2 - cp1;
  let dh = h2 - h1;
  if (cp1 * cp2 === 0) dh = 0;
  else if (dh > 180) dh -= 360;
  else if (dh < -180) dh += 360;
  const dH = 2 * Math.sqrt(cp1 * cp2) * Math.sin(dh / 2 * radians);
  const meanL = (l1 + l2) / 2, meanC = (cp1 + cp2) / 2;
  let meanH = (h1 + h2) / 2;
  if (cp1 * cp2 === 0) meanH = h1 + h2;
  else if (Math.abs(h1 - h2) > 180) meanH += h1 + h2 < 360 ? 180 : -180;
  const t = 1 - 0.17 * Math.cos((meanH - 30) * radians) + 0.24 * Math.cos(2 * meanH * radians)
    + 0.32 * Math.cos((3 * meanH + 6) * radians) - 0.20 * Math.cos((4 * meanH - 63) * radians);
  const sl = 1 + 0.015 * (meanL - 50) ** 2 / Math.sqrt(20 + (meanL - 50) ** 2);
  const sc = 1 + 0.045 * meanC, sh = 1 + 0.015 * meanC * t;
  const rt = -2 * Math.sqrt(meanC ** 7 / (meanC ** 7 + 25 ** 7))
    * Math.sin(60 * Math.exp(-(((meanH - 275) / 25) ** 2)) * radians);
  const lightness = dl / sl, chroma = dc / sc, hueDifference = dH / sh;
  return Math.sqrt(lightness ** 2 + chroma ** 2 + hueDifference ** 2 + rt * chroma * hueDifference);
}
