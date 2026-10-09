import { continentTotals, normalizePresence, presenceRegions, regionKey, sessionJitter } from './data.js';
import { qualityMean, qualityColor } from './quality.js';
import { COUNTRY_CENTROIDS } from './countries.js';
import { SUBREGION_CENTROIDS, SUBREGION_NAMES } from './subregions.js';
import { LAND_POINTS, COASTLINE_POINTS } from './land.js';

// These are bundled artwork anchors, never locations supplied by a node.
const CENTROIDS = {
  africa: [20, 1], asia: [92, 35], europe: [20, 49],
  north_america: [-102, 45], south_america: [-60, -16],
  oceania: [138, -25], antarctica: [20, -79],
};
const RADIANS = Math.PI / 180;
const ARC_STEPS = 48;
const MAX_ARCS = 7;
const INITIAL_YAW = -92 * RADIANS;
const INITIAL_PITCH = 35 * RADIANS;
const IDLE_DELAY = 10_000;
const COLORS = {
  sphere: '#071320', land: '#7CC4DC', coast: '#B7E2EF',
  'quality-start': '#7CC4DC', 'quality-end': '#5CCB98',
  'quality-unknown': '#94A4B5',
};

function vector(longitude, latitude) {
  return [Math.cos(latitude) * Math.sin(longitude), Math.sin(latitude), Math.cos(latitude) * Math.cos(longitude)];
}

// Faint, locally generated 30-degree graticule. No network location input.
function graticule() {
  const points = [];
  for (let lon = -180; lon < 180; lon += 30) {
    for (let lat = -90; lat < 90; lat += 2) points.push(...vector(lon * RADIANS, lat * RADIANS), ...vector(lon * RADIANS, (lat + 2) * RADIANS));
  }
  for (let lat = -60; lat <= 60; lat += 30) {
    for (let lon = -180; lon < 180; lon += 2) points.push(...vector(lon * RADIANS, lat * RADIANS), ...vector((lon + 2) * RADIANS, lat * RADIANS));
  }
  return new Float32Array(points);
}

const SPHERE_VERTEX = `
attribute vec2 a_position;
uniform vec2 u_scale;
varying vec2 v_position;
void main() {
  v_position = a_position;
  gl_Position = vec4(a_position * u_scale, 0.0, 1.0);
}`;
const SPHERE_FRAGMENT = `
precision mediump float;
uniform vec3 u_color;
uniform vec3 u_land;
varying vec2 v_position;
void main() {
  float r = length(v_position);
  if (r > 1.12) discard;
  if (r > 1.0) {
    float halo = exp(-(r - 1.0) * 38.0) * 0.24;
    gl_FragColor = vec4(u_land, halo * (1.0 - smoothstep(1.04, 1.12, r)));
    return;
  }
  vec3 normal = vec3(v_position, sqrt(max(0.0, 1.0 - r * r)));
  float light = max(0.0, dot(normal, normalize(vec3(-0.6, 0.7, 0.8))));
  vec3 color = u_color * (0.7 + light * 1.25);
  color += u_land * pow(r, 16.0) * 0.15;
  gl_FragColor = vec4(color, 1.0 - smoothstep(0.994, 1.0, r));
}`;
const POINT_VERTEX = `
attribute vec3 a_position;
uniform mat3 u_rotation;
uniform vec2 u_scale;
uniform float u_dpr;
uniform float u_point_cap;
uniform float u_land_size;
varying float v_facing;
void main() {
  vec3 position = u_rotation * a_position;
  v_facing = position.z;
  gl_Position = vec4(position.xy * u_scale, 0.0, 1.0);
  gl_PointSize = min(u_point_cap, u_land_size * u_dpr);
}`;
const POINT_FRAGMENT = `
precision mediump float;
uniform vec3 u_color;
varying float v_facing;
void main() {
  if (v_facing < 0.025) discard;
  float r = length(gl_PointCoord - 0.5);
  if (r > 0.5) discard;
  float edge = smoothstep(0.025, 0.22, v_facing);
  float alpha = (1.0 - smoothstep(0.32, 0.5, r)) * (0.65 + v_facing * 0.30);
  gl_FragColor = vec4(u_color, alpha * edge);
}`;
const LINE_VERTEX = `
attribute vec3 a_position;
uniform mat3 u_rotation;
uniform vec2 u_scale;
varying float v_facing;
void main() {
  vec3 position = u_rotation * a_position;
  v_facing = position.z;
  gl_Position = vec4(position.xy * u_scale, 0.0, 1.0);
}`;
const LINE_FRAGMENT = `
precision mediump float;
uniform vec3 u_color;
uniform float u_alpha;
varying float v_facing;
void main() {
  if (v_facing < 0.0) discard;
  gl_FragColor = vec4(u_color, u_alpha * smoothstep(0.0, 0.12, v_facing));
}`;
const ARC_VERTEX = `
attribute vec3 a_position;
attribute float a_progress;
attribute float a_phase;
uniform mat3 u_rotation;
uniform vec2 u_scale;
varying float v_facing;
varying float v_progress;
varying float v_phase;
void main() {
  vec3 position = u_rotation * a_position;
  v_facing = position.z;
  v_progress = a_progress;
  v_phase = a_phase;
  gl_Position = vec4(position.xy * u_scale, 0.0, 1.0);
}`;
const ARC_FRAGMENT = `
precision mediump float;
uniform vec3 u_color;
uniform float u_time;
varying float v_facing;
varying float v_progress;
varying float v_phase;
void main() {
  if (v_facing < 0.025) discard;
  float distance = abs(v_progress - fract(u_time * 0.22 + v_phase));
  float highlight = 1.0 - smoothstep(0.0, 0.16, distance);
  float edge = smoothstep(0.025, 0.2, v_facing);
  gl_FragColor = vec4(u_color, (0.26 + highlight * 0.62) * edge);
}`;

function program(gl, vertexSource, fragmentSource, attributes, uniforms) {
  const shaders = [];
  let handle;
  try {
    for (const [type, source] of [[gl.VERTEX_SHADER, vertexSource], [gl.FRAGMENT_SHADER, fragmentSource]]) {
      const shader = gl.createShader(type);
      if (!shader) throw new Error('WebGL shader unavailable');
      shaders.push(shader);
      gl.shaderSource(shader, source);
      gl.compileShader(shader);
      if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) throw new Error('WebGL shader unsupported');
    }
    handle = gl.createProgram();
    if (!handle) throw new Error('WebGL program unavailable');
    for (const shader of shaders) gl.attachShader(handle, shader);
    gl.linkProgram(handle);
    if (!gl.getProgramParameter(handle, gl.LINK_STATUS)) throw new Error('WebGL program unsupported');
    const result = { handle };
    for (const name of attributes) result[name] = gl.getAttribLocation(handle, name);
    for (const name of uniforms) result[name] = gl.getUniformLocation(handle, name);
    return result;
  } catch (error) {
    if (handle) gl.deleteProgram(handle);
    throw error;
  } finally {
    for (const shader of shaders) gl.deleteShader(shader);
  }
}

function readColor(element, name) {
  const value = element.ownerDocument.defaultView.getComputedStyle(element)
    .getPropertyValue(`--lg-${name}`).trim();
  const hex = /^#[0-9a-f]{6}$/i.test(value) ? value : COLORS[name];
  return {
    css: hex,
    rgb: new Float32Array([1, 3, 5].map(offset => parseInt(hex.slice(offset, offset + 2), 16) / 255)),
  };
}

function clamp(value, min, max) { return Math.max(min, Math.min(max, value)); }

export function createGlobe(canvas, {
  seed, onSelect = () => {}, onVisibility = () => {},
  paused: initiallyPaused = false, reducedMotion = false,
} = {}) {
  const document = canvas.ownerDocument;
  const window = document.defaultView;
  const stage = canvas.parentElement || canvas;
  const sessionSeed = seed ?? Math.floor(Math.random() * 0xffffffff);
  const motion = window.matchMedia('(prefers-reduced-motion: reduce)');
  const original = { hidden: canvas.hidden, display: canvas.style.display, touchAction: canvas.style.touchAction, renderer: canvas.getAttribute('data-renderer') };
  const map = document.createElement('canvas');
  map.className = canvas.className;
  map.setAttribute('aria-hidden', 'true');
  map.dataset.renderer = 'map';
  map.hidden = true;
  canvas.after(map);
  canvas.style.touchAction = 'pan-y';
  const context = map.getContext('2d');
  const overlay = document.createElement('div');
  overlay.className = 'lg-markers';
  stage.append(overlay);
  const svgElement = (tag, attributes) => {
    const el = document.createElementNS('http://www.w3.org/2000/svg', tag);
    for (const [name, value] of Object.entries(attributes)) el.setAttribute(name, String(value));
    return el;
  };
  function makeMarker(region, anchor) {
    const key = regionKey(region);
    const jitter = sessionJitter(key, sessionSeed);
    const longitude = anchor[0] * RADIANS + clamp(Number(jitter[0]) || 0, -0.06, 0.06);
    const latitude = anchor[1] * RADIANS + clamp(Number(jitter[1]) || 0, -0.06, 0.06);
    const button = document.createElement('button');
    button.type = 'button'; button.className = 'lg-marker'; button.dataset.continent = region.continent;
    button.dataset.region = key;
    if (region.country) button.dataset.country = region.country;
    button.hidden = true;
    const orb = svgElement('svg', { viewBox: '-22 -22 44 44', 'aria-hidden': 'true' });
    orb.classList.add('lg-marker-orb');
    const core = svgElement('circle', { cx: 0, cy: 0, r: 15, class: 'lg-marker-core' });
    const ring = svgElement('circle', { cx: 0, cy: 0, r: 18, fill: 'none', 'stroke-width': 2, class: 'lg-marker-ring' });
    const outline = svgElement('circle', { cx: 0, cy: 0, r: 21, fill: 'none', 'stroke-width': 1, class: 'lg-marker-outline' });
    orb.append(outline, core, ring);
    const label = document.createElement('span'); label.className = 'lg-marker-label';
    label.style.right = 'auto'; label.style.transform = 'none';
    const leader = document.createElement('span'); leader.className = 'lg-marker-leader';
    leader.setAttribute('aria-hidden', 'true');
    button.append(orb, label, leader); overlay.append(button);
    const select = () => { interact(); onSelect(key); };
    const deselect = () => { if (document.activeElement !== button) onSelect(null); };
    button.addEventListener('pointerenter', select); button.addEventListener('pointerleave', deselect);
    button.addEventListener('focus', select); button.addEventListener('blur', () => onSelect(null));
    button.addEventListener('click', select);
    return {
      ...region, key, count: 0, score: 0, button, core, ring, label, leader,
      position: new Float32Array([Math.cos(latitude) * Math.sin(longitude), Math.sin(latitude), Math.cos(latitude) * Math.cos(longitude)]),
    };
  }
  const markers = Object.entries({ ...CENTROIDS, ...SUBREGION_CENTROIDS })
    .map(([continent, anchor]) => makeMarker({ continent }, anchor));
  const byRegion = new Map(markers.map(marker => [marker.key, marker]));
  const byContinent = new Map(markers.map(marker => [marker.continent, marker]));
  const land = new Float32Array(LAND_POINTS);
  const coast = new Float32Array(COASTLINE_POINTS);
  const grid = graticule();
  const arcData = new Float32Array(MAX_ARCS * ARC_STEPS * 2 * 5);
  const rotation = new Float32Array(9);
  const scale = new Float32Array(2);
  let colors, gl, resources, glAttempted = false, contextLost = false;
  let arcCount = 0;
  let width = 1, height = 1, dpr = 1, radius = 1;
  let yaw = INITIAL_YAW, pitch = INITIAL_PITCH, time = 0;
  let homeYaw = yaw, homePitch = pitch, centered = false, idleUntil = 0;
  let labels = { continents: {}, regions: {}, quality: 'Operator quality', qualityUnavailable: 'Operation quality unavailable' }, visibilityKey = '';
  let sourceEvidence = true;
  const unplaced = new Map([['unknown', 0], ['world', 0]]);
  let frame = 0, lastFrame = 0, paused = Boolean(initiallyPaused), visible = true, destroyed = false;
  let hostReducedMotion = Boolean(reducedMotion);
  let staticMode = true, pointer = null;

  function releaseResources() {
    if (!gl || !resources) return;
    for (const buffer of resources.buffers) gl.deleteBuffer(buffer);
    for (const item of resources.programs) gl.deleteProgram(item.handle);
    resources = null;
  }

  function ensureGL() {
    if (resources || glAttempted || contextLost) return;
    glAttempted = true;
    try {
      gl = canvas.getContext('webgl', { alpha: true, antialias: true, depth: false, stencil: false, powerPreference: 'low-power' });
      if (!gl) return;
      resources = { buffers: [], programs: [] };
      const makeProgram = (vertex, fragment, attributes, uniforms) => {
        const item = program(gl, vertex, fragment, attributes, uniforms);
        resources.programs.push(item);
        return item;
      };
      const makeBuffer = (data, usage) => {
        const buffer = gl.createBuffer();
        if (!buffer) throw new Error('WebGL buffer unavailable');
        resources.buffers.push(buffer);
        gl.bindBuffer(gl.ARRAY_BUFFER, buffer);
        gl.bufferData(gl.ARRAY_BUFFER, data, usage);
        return buffer;
      };
      resources.sphere = makeProgram(SPHERE_VERTEX, SPHERE_FRAGMENT, ['a_position'], ['u_scale', 'u_color', 'u_land']);
      resources.points = makeProgram(POINT_VERTEX, POINT_FRAGMENT, ['a_position'],
        ['u_rotation', 'u_scale', 'u_dpr', 'u_point_cap', 'u_land_size', 'u_color']);
      resources.outlines = makeProgram(LINE_VERTEX, LINE_FRAGMENT, ['a_position'], ['u_rotation', 'u_scale', 'u_color', 'u_alpha']);
      resources.arcs = makeProgram(ARC_VERTEX, ARC_FRAGMENT, ['a_position', 'a_progress', 'a_phase'], ['u_rotation', 'u_scale', 'u_color', 'u_time']);
      resources.quad = makeBuffer(new Float32Array([-1.12, -1.12, 1.12, -1.12, -1.12, 1.12, -1.12, 1.12, 1.12, -1.12, 1.12, 1.12]), gl.STATIC_DRAW);
      resources.land = makeBuffer(land, gl.STATIC_DRAW);
      resources.coast = makeBuffer(coast, gl.STATIC_DRAW);
      resources.grid = makeBuffer(grid, gl.STATIC_DRAW);
      resources.lines = makeBuffer(arcData, gl.DYNAMIC_DRAW);
      resources.pointCap = gl.getParameter(gl.ALIASED_POINT_SIZE_RANGE)[1];
      gl.disable(gl.DEPTH_TEST);
      gl.enable(gl.BLEND);
      gl.clearColor(0, 0, 0, 0);
    } catch {
      releaseResources();
    }
  }

  function stopFrame() {
    if (frame) window.cancelAnimationFrame(frame);
    frame = 0;
    lastFrame = 0;
  }

  function active() {
    return !destroyed && !paused && !staticMode && visible && !document.hidden;
  }

  function interact() { idleUntil = window.performance.now() + IDLE_DELAY; }

  function animate(now) {
    frame = 0;
    if (!active()) { lastFrame = 0; return; }
    const delta = lastFrame ? Math.min((now - lastFrame) / 1000, 0.05) : 0;
    lastFrame = now;
    time += delta;
    if (!pointer && now >= idleUntil) yaw += delta * 0.025;
    drawWebGL();
    frame = window.requestAnimationFrame(animate);
  }

  function reconcile() {
    if (destroyed) return;
    const reduced = motion.matches || hostReducedMotion;
    if (!reduced && !contextLost) ensureGL();
    staticMode = reduced || contextLost || !resources;
    canvas.hidden = staticMode;
    map.hidden = !staticMode;
    canvas.style.display = staticMode ? 'none' : original.display;
    map.style.display = staticMode ? '' : 'none';
    canvas.dataset.renderer = staticMode ? 'map' : 'webgl';
    stage.dataset.crowdedMap = String(staticMode && markers.filter(marker => marker.count).length > 2);
    overlay.dataset.animated = String(active());
    stopFrame();
    resize();
    if (active()) frame = window.requestAnimationFrame(animate);
  }

  function drawWebGL(force = false) {
    if (!resources || contextLost || destroyed || (!force && (!visible || document.hidden))) return false;
    const cy = Math.cos(yaw), sy = Math.sin(yaw), cp = Math.cos(pitch), sp = Math.sin(pitch);
    rotation[0] = cy; rotation[1] = sp * sy; rotation[2] = -cp * sy;
    rotation[3] = 0; rotation[4] = cp; rotation[5] = sp;
    rotation[6] = sy; rotation[7] = -sp * cy; rotation[8] = cp * cy;
    gl.viewport(0, 0, canvas.width, canvas.height);
    gl.clear(gl.COLOR_BUFFER_BIT);
    // Shaders emit straight RGB; retain source-over coverage alpha so the
    // canvas compositor receives valid premultiplied pixels, not alpha squared.
    gl.blendFuncSeparate(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA, gl.ONE, gl.ONE_MINUS_SRC_ALPHA);
    const sphere = resources.sphere;
    gl.useProgram(sphere.handle);
    gl.uniform2fv(sphere.u_scale, scale);
    gl.uniform3fv(sphere.u_color, colors.sphere.rgb);
    gl.uniform3fv(sphere.u_land, colors.land.rgb);
    gl.bindBuffer(gl.ARRAY_BUFFER, resources.quad);
    gl.enableVertexAttribArray(sphere.a_position);
    gl.vertexAttribPointer(sphere.a_position, 2, gl.FLOAT, false, 0, 0);
    gl.drawArrays(gl.TRIANGLES, 0, 6);
    gl.disableVertexAttribArray(sphere.a_position);

    const points = resources.points;
    gl.useProgram(points.handle);
    gl.uniformMatrix3fv(points.u_rotation, false, rotation);
    gl.uniform2fv(points.u_scale, scale);
    gl.uniform1f(points.u_dpr, dpr);
    gl.uniform1f(points.u_point_cap, resources.pointCap);
    gl.uniform1f(points.u_land_size, clamp(radius / 185, 1.25, 1.85));
    gl.uniform3fv(points.u_color, colors.land.rgb);
    gl.bindBuffer(gl.ARRAY_BUFFER, resources.land);
    gl.enableVertexAttribArray(points.a_position);
    gl.vertexAttribPointer(points.a_position, 3, gl.FLOAT, false, 0, 0);
    gl.drawArrays(gl.POINTS, 0, land.length / 3);
    gl.disableVertexAttribArray(points.a_position);

    const outlines = resources.outlines;
    gl.useProgram(outlines.handle);
    gl.uniformMatrix3fv(outlines.u_rotation, false, rotation);
    gl.uniform2fv(outlines.u_scale, scale);
    gl.enableVertexAttribArray(outlines.a_position);
    for (const [buffer, count, color, alpha] of [[resources.grid, grid.length / 3, colors.land.rgb, 0.16], [resources.coast, coast.length / 3, colors.coast.rgb, 0.8]]) {
      gl.uniform3fv(outlines.u_color, color);
      gl.uniform1f(outlines.u_alpha, alpha);
      gl.bindBuffer(gl.ARRAY_BUFFER, buffer);
      gl.vertexAttribPointer(outlines.a_position, 3, gl.FLOAT, false, 0, 0);
      gl.drawArrays(gl.LINES, 0, count);
    }
    gl.disableVertexAttribArray(outlines.a_position);

    if (arcCount) {
      const arcs = resources.arcs;
      gl.useProgram(arcs.handle);
      gl.uniformMatrix3fv(arcs.u_rotation, false, rotation);
      gl.uniform2fv(arcs.u_scale, scale);
      gl.uniform3fv(arcs.u_color, colors.land.rgb);
      gl.uniform1f(arcs.u_time, time);
      gl.bindBuffer(gl.ARRAY_BUFFER, resources.lines);
      gl.enableVertexAttribArray(arcs.a_position);
      gl.enableVertexAttribArray(arcs.a_progress);
      gl.enableVertexAttribArray(arcs.a_phase);
      gl.vertexAttribPointer(arcs.a_position, 3, gl.FLOAT, false, 20, 0);
      gl.vertexAttribPointer(arcs.a_progress, 1, gl.FLOAT, false, 20, 12);
      gl.vertexAttribPointer(arcs.a_phase, 1, gl.FLOAT, false, 20, 16);
      gl.drawArrays(gl.LINES, 0, arcCount);
      gl.disableVertexAttribArray(arcs.a_position);
      gl.disableVertexAttribArray(arcs.a_progress);
      gl.disableVertexAttribArray(arcs.a_phase);
    }

    positionMarkers();
    if (force) gl.flush();
    return true;
  }

  function drawMap(force = false) {
    if (!context || destroyed || (!force && (!visible || document.hidden))) return false;
    const mapWidth = Math.min(width * 0.92, height * 1.5);
    const mapHeight = mapWidth / 2;
    const left = (width - mapWidth) / 2, top = (height - mapHeight) / 2;
    const toX = (x, z) => left + (Math.atan2(x, z) / (2 * Math.PI) + 0.5) * mapWidth;
    const toY = y => top + (0.5 - Math.asin(clamp(y, -1, 1)) / Math.PI) * mapHeight;
    context.setTransform(dpr, 0, 0, dpr, 0, 0);
    context.clearRect(0, 0, width, height);
    context.fillStyle = colors.sphere.css;
    context.fillRect(left - 12, top - 12, mapWidth + 24, mapHeight + 24);
    context.strokeStyle = colors.land.css;
    context.globalAlpha = 0.16;
    context.lineWidth = 1;
    context.beginPath();
    for (let column = 1; column < 12; column++) {
      const x = left + column * mapWidth / 12;
      context.moveTo(x, top); context.lineTo(x, top + mapHeight);
    }
    for (let row = 1; row < 6; row++) {
      const y = top + row * mapHeight / 6;
      context.moveTo(left, y); context.lineTo(left + mapWidth, y);
    }
    context.stroke();
    context.fillStyle = colors.land.css;
    context.globalAlpha = 0.85;
    context.beginPath();
    for (let i = 0; i < land.length; i += 3) {
      const x = toX(land[i], land[i + 2]), y = toY(land[i + 1]);
      context.moveTo(x + 0.8, y); context.arc(x, y, 0.8, 0, Math.PI * 2);
    }
    context.fill();
    context.strokeStyle = colors.coast.css;
    context.globalAlpha = 0.9;
    context.beginPath();
    for (let i = 0; i < coast.length; i += 6) {
      const x1 = toX(coast[i], coast[i + 2]), x2 = toX(coast[i + 3], coast[i + 5]);
      if (Math.abs(x1 - x2) > mapWidth / 2) continue;
      context.moveTo(x1, toY(coast[i + 1])); context.lineTo(x2, toY(coast[i + 4]));
    }
    context.stroke();
    context.strokeStyle = colors.land.css;
    context.globalAlpha = 0.62;
    context.beginPath();
    for (let i = 0; i < arcCount * 5; i += 10) {
      const r1 = Math.hypot(arcData[i], arcData[i + 1], arcData[i + 2]);
      const r2 = Math.hypot(arcData[i + 5], arcData[i + 6], arcData[i + 7]);
      const x1 = toX(arcData[i], arcData[i + 2]), x2 = toX(arcData[i + 5], arcData[i + 7]);
      if (Math.abs(x1 - x2) > mapWidth / 2) continue;
      context.moveTo(x1, toY(arcData[i + 1] / r1));
      context.lineTo(x2, toY(arcData[i + 6] / r2));
    }
    context.stroke();
    context.globalAlpha = 1;
    positionMarkers();
    return true;
  }

  function positionMarkers() {
    const states = [], front = [];
    const mapWidth = Math.min(width * 0.92, height * 1.5), mapHeight = mapWidth / 2;
    const left = (width - mapWidth) / 2, top = (height - mapHeight) / 2;
    for (const marker of markers) {
      const p = marker.position;
      const facing = rotation[2] * p[0] + rotation[5] * p[1] + rotation[8] * p[2];
      const visibility = !marker.count ? 'empty' : staticMode || facing >= 0.10 ? 'front' : 'back';
      states.push({ key: marker.key, continent: marker.continent, country: marker.country, visibility });
      marker.button.hidden = visibility !== 'front';
      if (visibility !== 'front') continue;
      const x = staticMode ? left + (Math.atan2(p[0], p[2]) / (2 * Math.PI) + 0.5) * mapWidth : width / 2 + radius * (rotation[0] * p[0] + rotation[3] * p[1] + rotation[6] * p[2]);
      const y = staticMode ? top + (0.5 - Math.asin(clamp(p[1], -1, 1)) / Math.PI) * mapHeight : height / 2 - radius * (rotation[1] * p[0] + rotation[4] * p[1] + rotation[7] * p[2]);
      marker.button.style.transform = `translate3d(${x.toFixed(2)}px,${y.toFixed(2)}px,0)`;
      // Include pulse animation, outline and shadow in the exclusion radius.
      const pulseRadius = parseFloat(marker.button.style.getPropertyValue('--marker-size')) / 2 * 1.035 + 5;
      front.push({ marker, x, y, pulseRadius });
    }
    // Text metrics are cached outside animation. Labels avoid every visible
    // pulse as well as previous labels; pulses retain their geographic anchors.
    const occupied = [];
    const overlaps = (a, b) => a.x < b.x + b.w + 4 && a.x + a.w + 4 > b.x && a.y < b.y + b.h + 4 && a.y + a.h + 4 > b.y;
    const coversPulse = rect => front.some(pulse => {
      const dx = pulse.x - clamp(pulse.x, rect.x, rect.x + rect.w);
      const dy = pulse.y - clamp(pulse.y, rect.y, rect.y + rect.h);
      return dx * dx + dy * dy < pulse.pulseRadius * pulse.pulseRadius;
    });
    front.sort((a, b) => Number(b.marker.button.dataset.active === 'true') - Number(a.marker.button.dataset.active === 'true') || a.y - b.y || a.x - b.x);
    for (const { marker, x, y, pulseRadius } of front) {
      const w = marker.labelWidth, h = marker.labelHeight;
      const gap = pulseRadius + 4;
      const sides = x > width * 0.58 ? [-1, 1] : [1, -1];
      let placement;
      const rightEdge = Math.max(4, width - w - 4);
      const candidates = [...new Set([...sides.map(side => clamp(side === 1 ? x + gap : x - gap - w, 4, rightEdge)), 4, rightEdge, (width - w) / 2])];
      for (let step = 0; step <= Math.ceil(height / 8) * 2; step++) {
        const shift = step ? Math.ceil(step / 2) * (step % 2 ? 1 : -1) : 0;
        for (const labelX of candidates) {
          const candidate = { x: labelX, y: y - h / 2 + shift * 8, w, h };
          if (candidate.x < 4 || candidate.x + w > width - 4 || candidate.y < 4 || candidate.y + h > height - 4
            || occupied.some(rect => overlaps(candidate, rect)) || coversPulse(candidate)) continue;
          placement = candidate; break;
        }
        if (placement) break;
      }
      // Dense views can run out of room. Keep the accessible pulse and list,
      // rather than drawing a label over a pulse or another label.
      marker.label.hidden = !placement;
      marker.leader.hidden = !placement;
      if (!placement) continue;
      occupied.push(placement);
      marker.label.style.left = `${(placement.x - x + 22).toFixed(2)}px`;
      marker.label.style.top = `${(placement.y - y + 22).toFixed(2)}px`;
      const dx = clamp(x, placement.x, placement.x + w) - x;
      const dy = clamp(y, placement.y, placement.y + h) - y;
      const length = Math.hypot(dx, dy);
      marker.leader.style.left = `${(22 + dx / length * pulseRadius).toFixed(2)}px`;
      marker.leader.style.top = `${(22 + dy / length * pulseRadius).toFixed(2)}px`;
      marker.leader.style.width = `${Math.max(0, length - pulseRadius).toFixed(2)}px`;
      marker.leader.style.transform = `rotate(${Math.atan2(dy, dx)}rad)`;
    }
    for (const [code, count] of unplaced) {
      states.push({ key: code, continent: code, visibility: count ? 'unknown' : 'empty' });
    }
    const key = states.map(region => `${region.key}:${region.visibility}`).join(',');
    if (key !== visibilityKey) { visibilityKey = key; onVisibility(states); }
  }

  function labelMarkers() {
    for (const marker of markers) {
      const name = labels.regions?.[marker.key] || labels.continents[marker.continent]
        || SUBREGION_NAMES[marker.continent] || marker.continent;
      const text = `${name} ${marker.count.toLocaleString()}`;
      marker.label.textContent = text;
      const quality = !sourceEvidence || labels.qualityAvailable === false ? labels.qualityUnavailable
        : `${labels.quality} ${(marker.score * 100).toFixed(1)} / 100`;
      marker.button.setAttribute('aria-label', `${text} · ${quality}`);
    }
    measureLabels();
  }

  function measureLabels() {
    for (const marker of markers) {
      const hidden = marker.button.hidden;
      const labelHidden = marker.label.hidden;
      marker.button.hidden = false;
      marker.label.hidden = false;
      const compact = width <= 680;
      const maxWidth = compact ? 150 : 190;
      const measured = [...marker.label.textContent].reduce((sum, character) => sum + (character.charCodeAt(0) > 127 ? 13 : 7), 14);
      marker.labelWidth = marker.label.offsetWidth || Math.min(maxWidth, measured);
      marker.labelHeight = marker.label.offsetHeight || Math.ceil(measured / maxWidth) * (compact ? 17 : 19) + 8;
      marker.button.hidden = hidden;
      marker.label.hidden = labelHidden;
    }
  }

  function draw(force = false) { return staticMode ? drawMap(force) : drawWebGL(force); }

  function resize({ force = false } = {}) {
    if (destroyed) return false;
    const bounds = (staticMode ? map : canvas).getBoundingClientRect();
    width = Math.max(1, bounds.width || stage.clientWidth || 1);
    height = Math.max(1, bounds.height || stage.clientHeight || width);
    dpr = Math.min(2, window.devicePixelRatio || 1);
    // The sphere, its 12% halo and elevated arcs all fit with breathing room.
    radius = Math.min(width, height) * 0.40;
    scale[0] = radius * 2 / width; scale[1] = radius * 2 / height;
    const pixelWidth = Math.round(width * dpr), pixelHeight = Math.round(height * dpr);
    for (const surface of [canvas, map]) {
      if (surface.width !== pixelWidth) surface.width = pixelWidth;
      if (surface.height !== pixelHeight) surface.height = pixelHeight;
    }
    colors = { sphere: readColor(canvas, 'sphere'), land: readColor(canvas, 'land'), coast: readColor(canvas, 'coast') };
    const start = readColor(canvas, 'quality-start').css;
    const end = readColor(canvas, 'quality-end').css;
    const unknown = readColor(canvas, 'quality-unknown').css;
    for (const marker of markers) {
      const measured = sourceEvidence && labels.qualityAvailable !== false;
      marker.button.style.setProperty('--marker-color', measured ? qualityColor(marker.score, start, end) : unknown);
      marker.button.style.setProperty('--marker-intensity', String(measured ? .65 + .35 * marker.score : .8));
    }
    measureLabels();
    return draw(force);
  }

  function writeArcPoint(offset, from, to, angle, denominator, progress, phase) {
    const a = denominator < 0.0001 ? 1 - progress : Math.sin((1 - progress) * angle) / denominator;
    const b = denominator < 0.0001 ? progress : Math.sin(progress * angle) / denominator;
    const x = from[0] * a + to[0] * b, y = from[1] * a + to[1] * b, z = from[2] * a + to[2] * b;
    const length = Math.hypot(x, y, z);
    const lift = (1.006 + Math.sin(progress * Math.PI) * 0.16) / length;
    arcData[offset] = x * lift; arcData[offset + 1] = y * lift; arcData[offset + 2] = z * lift;
    arcData[offset + 3] = progress; arcData[offset + 4] = phase;
  }

  function update(model, { reset = false } = {}) {
    if (destroyed) return;
    const clean = normalizePresence(model);
    sourceEvidence = clean.schema !== 2;
    const regions = presenceRegions(clean);
    if (reset) {
      yaw = homeYaw = INITIAL_YAW;
      pitch = homePitch = INITIAL_PITCH;
      centered = false;
      time = lastFrame = idleUntil = 0;
      visibilityKey = '';
      if (pointer && canvas.hasPointerCapture(pointer.id)) canvas.releasePointerCapture(pointer.id);
      pointer = null;
    }
    const activeKeys = new Set(regions.map(regionKey));
    for (let i = markers.length - 1; i >= 0; i--) {
      const marker = markers[i];
      if (marker.country && !activeKeys.has(marker.key)) {
        marker.button.remove(); byRegion.delete(marker.key); markers.splice(i, 1);
      } else { marker.count = 0; marker.score = 0; }
    }
    for (const code of unplaced.keys()) unplaced.set(code, 0);
    for (const region of regions) {
      const key = regionKey(region);
      let marker = byRegion.get(key);
      if (!marker && region.country) {
        marker = makeMarker(region, COUNTRY_CENTROIDS[region.country]);
        markers.push(marker); byRegion.set(key, marker);
      }
      if (marker && region.country) { marker.count = region.count; marker.score = qualityMean(region.quality, region.count); }
      else if (unplaced.has(region.continent)) unplaced.set(region.continent, region.count);
    }
    const totals = new Map();
    for (const total of continentTotals(clean)) {
      totals.set(total.continent, total.count);
      const marker = byContinent.get(total.continent);
      if (marker) {
        marker.count = total.count ?? 0;
        marker.score = total.quality ? qualityMean(total.quality, total.count) : 0;
      }
    }
    const home = [...totals.keys()].reduce((best, code) =>
      markers.some(marker => marker.continent === code && marker.count) && totals.get(code) > (totals.get(best) || 0) ? code : best, null);
    const largest = markers.reduce((best, marker) => marker.continent === home && marker.count > (best?.count || 0) ? marker : best, null);
    if (!centered && largest) {
      yaw = homeYaw = -Math.atan2(largest.position[0], largest.position[2]);
      pitch = homePitch = Math.asin(largest.position[1]);
      centered = true;
    }
    for (const marker of markers) {
      if (!marker.count) marker.button.hidden = true;
      marker.button.style.setProperty('--marker-size', `${Math.min(60, 16 + Math.sqrt(marker.count) * 8)}px`);
      marker.button.dataset.count = String(marker.count);
      marker.button.dataset.quality = sourceEvidence ? String(marker.score) : '';
    }
    labelMarkers();
    arcCount = 0;
    const blocks = clean.recent_blocks || [];
    for (let i = 1; i < Math.min(blocks.length, MAX_ARCS + 1); i++) {
      const from = byContinent.get(blocks[i - 1].continent), to = byContinent.get(blocks[i].continent);
      if (!from || !to || from === to) continue;
      const a = from.position, b = to.position;
      const angle = Math.acos(clamp(a[0] * b[0] + a[1] * b[1] + a[2] * b[2], -1, 1));
      const denominator = Math.sin(angle);
      for (let step = 0; step < ARC_STEPS; step++) {
        writeArcPoint(arcCount++ * 5, a, b, angle, denominator, step / ARC_STEPS, i / 8);
        writeArcPoint(arcCount++ * 5, a, b, angle, denominator, (step + 1) / ARC_STEPS, i / 8);
      }
    }
    if (resources && !contextLost) {
      gl.bindBuffer(gl.ARRAY_BUFFER, resources.lines); gl.bufferSubData(gl.ARRAY_BUFFER, 0, arcData);
    }
    stage.dataset.crowdedMap = String(staticMode && markers.filter(marker => marker.count).length > 2);
    resize();
  }

  function pointerDown(event) {
    if (staticMode || event.button !== 0 || event.isPrimary === false) return;
    interact();
    pointer = { id: event.pointerId, x: event.clientX, y: event.clientY, startX: event.clientX, startY: event.clientY, touch: event.pointerType === 'touch', dragging: false };
    if (!pointer.touch) canvas.setPointerCapture(event.pointerId);
  }
  function pointerMove(event) {
    interact();
    if (!pointer || event.pointerId !== pointer.id) return;
    if (!pointer.dragging) {
      const x = event.clientX - pointer.startX, y = event.clientY - pointer.startY;
      if (pointer.touch && Math.abs(y) > Math.abs(x) && Math.abs(y) > 7) { pointer = null; return; }
      if (Math.hypot(x, y) < 4) return;
      pointer.dragging = true;
      canvas.setPointerCapture(event.pointerId);
    }
    yaw += (event.clientX - pointer.x) * 0.007;
    if (!pointer.touch) pitch = clamp(pitch + (event.clientY - pointer.y) * 0.005, -1.1, 1.1);
    pointer.x = event.clientX; pointer.y = event.clientY;
    draw();
  }
  function pointerEnd(event) {
    if (!pointer || event.pointerId !== pointer.id) return;
    interact();
    if (canvas.hasPointerCapture(event.pointerId)) canvas.releasePointerCapture(event.pointerId);
    pointer = null;
  }
  function keyDown(event) {
    if (staticMode) return;
    if (event.key === 'ArrowLeft') yaw -= 0.13;
    else if (event.key === 'ArrowRight') yaw += 0.13;
    else if (event.key === 'ArrowUp') pitch = clamp(pitch + 0.1, -1.1, 1.1);
    else if (event.key === 'ArrowDown') pitch = clamp(pitch - 0.1, -1.1, 1.1);
    else if (event.key === 'Home') { yaw = homeYaw; pitch = homePitch; }
    else return;
    event.preventDefault();
    interact();
    draw();
  }
  function lost(event) { event.preventDefault(); contextLost = true; reconcile(); }
  function restored() { releaseResources(); contextLost = false; glAttempted = false; reconcile(); }
  function visibilityChanged() { reconcile(); }

  canvas.addEventListener('pointerdown', pointerDown);
  canvas.addEventListener('pointermove', pointerMove);
  canvas.addEventListener('pointerup', pointerEnd);
  canvas.addEventListener('pointercancel', pointerEnd);
  canvas.addEventListener('lostpointercapture', pointerEnd);
  canvas.addEventListener('keydown', keyDown);
  canvas.addEventListener('webglcontextlost', lost);
  canvas.addEventListener('webglcontextrestored', restored);
  document.addEventListener('visibilitychange', visibilityChanged);
  window.addEventListener('resize', resize);
  if (motion.addEventListener) motion.addEventListener('change', reconcile);
  else motion.addListener(reconcile);
  const intersection = window.IntersectionObserver ? new window.IntersectionObserver(entries => {
    visible = entries[0].isIntersecting;
    reconcile();
  }) : null;
  intersection?.observe(stage);
  const sizing = window.ResizeObserver ? new window.ResizeObserver(resize) : null;
  sizing?.observe(stage);
  const theming = window.MutationObserver ? new window.MutationObserver(resize) : null;
  theming?.observe(document.documentElement, { attributes: true, attributeFilter: ['class', 'style', 'data-theme'] });
  reconcile();
  void document.fonts?.ready.then(resize);

  return {
    update,
    setLabels(next) { labels = next; labelMarkers(); resize(); },
    setHighlight(code, { interaction = true } = {}) {
      if (interaction) interact();
      for (const marker of markers) marker.button.dataset.active = String(marker.key === code || marker.continent === code);
    },
    setPaused(value) { paused = Boolean(value); reconcile(); },
    setReducedMotion(value) { hostReducedMotion = Boolean(value); reconcile(); },
    resize,
    // Screenshot fixtures can draw one still frame without changing lifecycle
    // state. This does not start animation or make an offscreen view visible.
    captureFrame() { return resize({ force: true }); },
    destroy() {
      if (destroyed) return;
      destroyed = true;
      stopFrame();
      intersection?.disconnect(); sizing?.disconnect(); theming?.disconnect();
      canvas.removeEventListener('pointerdown', pointerDown);
      canvas.removeEventListener('pointermove', pointerMove);
      canvas.removeEventListener('pointerup', pointerEnd);
      canvas.removeEventListener('pointercancel', pointerEnd);
      canvas.removeEventListener('lostpointercapture', pointerEnd);
      canvas.removeEventListener('keydown', keyDown);
      canvas.removeEventListener('webglcontextlost', lost);
      canvas.removeEventListener('webglcontextrestored', restored);
      document.removeEventListener('visibilitychange', visibilityChanged);
      window.removeEventListener('resize', resize);
      if (motion.removeEventListener) motion.removeEventListener('change', reconcile);
      else motion.removeListener(reconcile);
      if (pointer && canvas.hasPointerCapture(pointer.id)) canvas.releasePointerCapture(pointer.id);
      pointer = null;
      releaseResources();
      overlay.remove();
      map.remove();
      canvas.hidden = original.hidden;
      canvas.style.display = original.display;
      canvas.style.touchAction = original.touchAction;
      if (original.renderer === null) canvas.removeAttribute('data-renderer');
      else canvas.setAttribute('data-renderer', original.renderer);
    },
  };
}
