import { continentTotals, sessionJitter } from './data.js';
import { LAND_POINTS } from './land.js';

// These are bundled artwork anchors, never locations supplied by a node.
const CENTROIDS = {
  africa: [20, 1], asia: [92, 35], europe: [20, 49],
  north_america: [-102, 45], south_america: [-60, -16],
  oceania: [138, -25], antarctica: [20, -79],
};
const RADIANS = Math.PI / 180;
const ARC_STEPS = 48;
const MAX_ARCS = 7;
const INITIAL_YAW = -1.08;
const INITIAL_PITCH = 0.19;
const COLORS = { sphere: '#071320', land: '#7CC4DC', pulse: '#E8BF59' };

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
    float halo = exp(-(r - 1.0) * 38.0) * 0.12;
    gl_FragColor = vec4(u_land, halo * (1.0 - smoothstep(1.04, 1.12, r)));
    return;
  }
  vec3 normal = vec3(v_position, sqrt(max(0.0, 1.0 - r * r)));
  float light = max(0.0, dot(normal, normalize(vec3(-0.6, 0.7, 0.8))));
  vec3 color = u_color * (0.7 + light * 1.25);
  color += u_land * pow(r, 8.0) * 0.045;
  gl_FragColor = vec4(color, 1.0 - smoothstep(0.994, 1.0, r));
}`;
const POINT_VERTEX = `
attribute vec3 a_position;
attribute float a_size;
attribute float a_phase;
uniform mat3 u_rotation;
uniform vec2 u_scale;
uniform float u_dpr;
uniform float u_point_cap;
uniform float u_land_size;
uniform mediump float u_marker;
varying float v_facing;
varying float v_phase;
void main() {
  vec3 position = u_rotation * a_position;
  v_facing = position.z;
  v_phase = a_phase;
  gl_Position = vec4(position.xy * u_scale, 0.0, 1.0);
  float size = mix(u_land_size, a_size, u_marker);
  gl_PointSize = min(u_point_cap, size * u_dpr);
}`;
const POINT_FRAGMENT = `
precision mediump float;
uniform vec3 u_color;
uniform mediump float u_marker;
uniform float u_time;
varying float v_facing;
varying float v_phase;
void main() {
  if (v_facing < 0.025) discard;
  float r = length(gl_PointCoord - 0.5);
  if (r > 0.5) discard;
  float edge = smoothstep(0.025, 0.22, v_facing);
  if (u_marker < 0.5) {
    float alpha = (1.0 - smoothstep(0.25, 0.5, r)) * (0.4 + v_facing * 0.45);
    gl_FragColor = vec4(u_color, alpha * edge);
    return;
  }
  float phase = fract(u_time * 0.32 + v_phase);
  float ring = 1.0 - smoothstep(0.014, 0.038, abs(r - (0.16 + phase * 0.3)));
  float alpha = exp(-r * r * 24.0) * 0.24;
  alpha += ring * (1.0 - phase) * 0.4;
  alpha += (1.0 - smoothstep(0.055, 0.105, r)) * 0.92;
  gl_FragColor = vec4(u_color, alpha * edge);
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

export function createGlobe(canvas, { seed } = {}) {
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
  const markers = Object.entries(CENTROIDS).map(([continent, anchor], index) => {
    const jitter = sessionJitter(continent, sessionSeed);
    const longitude = anchor[0] * RADIANS + clamp(Number(jitter[0]) || 0, -0.06, 0.06);
    const latitude = anchor[1] * RADIANS + clamp(Number(jitter[1]) || 0, -0.06, 0.06);
    return {
      continent, count: 0, phase: index / 7,
      position: new Float32Array([Math.cos(latitude) * Math.sin(longitude), Math.sin(latitude), Math.cos(latitude) * Math.cos(longitude)]),
    };
  });
  const byContinent = new Map(markers.map(marker => [marker.continent, marker]));
  const land = new Float32Array(LAND_POINTS);
  const markerData = new Float32Array(markers.length * 5);
  const arcData = new Float32Array(MAX_ARCS * ARC_STEPS * 2 * 5);
  const rotation = new Float32Array(9);
  const scale = new Float32Array(2);
  let colors, gl, resources, glAttempted = false, contextLost = false;
  let markerCount = 0, arcCount = 0;
  let width = 1, height = 1, dpr = 1, radius = 1;
  let yaw = INITIAL_YAW, pitch = INITIAL_PITCH, time = 0;
  let frame = 0, lastFrame = 0, paused = false, visible = true, destroyed = false;
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
      resources.points = makeProgram(POINT_VERTEX, POINT_FRAGMENT, ['a_position', 'a_size', 'a_phase'],
        ['u_rotation', 'u_scale', 'u_dpr', 'u_point_cap', 'u_land_size', 'u_marker', 'u_color', 'u_time']);
      resources.arcs = makeProgram(ARC_VERTEX, ARC_FRAGMENT, ['a_position', 'a_progress', 'a_phase'], ['u_rotation', 'u_scale', 'u_color', 'u_time']);
      resources.quad = makeBuffer(new Float32Array([-1.12, -1.12, 1.12, -1.12, -1.12, 1.12, -1.12, 1.12, 1.12, -1.12, 1.12, 1.12]), gl.STATIC_DRAW);
      resources.land = makeBuffer(land, gl.STATIC_DRAW);
      resources.markers = makeBuffer(markerData, gl.DYNAMIC_DRAW);
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

  function animate(now) {
    frame = 0;
    if (!active()) { lastFrame = 0; return; }
    const delta = lastFrame ? Math.min((now - lastFrame) / 1000, 0.05) : 0;
    lastFrame = now;
    time += delta;
    if (!pointer) yaw += delta * 0.035;
    drawWebGL();
    frame = window.requestAnimationFrame(animate);
  }

  function reconcile() {
    if (destroyed) return;
    if (!motion.matches && !contextLost) ensureGL();
    staticMode = motion.matches || contextLost || !resources;
    canvas.hidden = staticMode;
    map.hidden = !staticMode;
    canvas.style.display = staticMode ? 'none' : original.display;
    map.style.display = staticMode ? '' : 'none';
    canvas.dataset.renderer = staticMode ? 'map' : 'webgl';
    stopFrame();
    resize();
    if (active()) frame = window.requestAnimationFrame(animate);
  }

  function drawWebGL() {
    if (!resources || contextLost || destroyed || !visible || document.hidden) return;
    const cy = Math.cos(yaw), sy = Math.sin(yaw), cp = Math.cos(pitch), sp = Math.sin(pitch);
    rotation[0] = cy; rotation[1] = sp * sy; rotation[2] = -cp * sy;
    rotation[3] = 0; rotation[4] = cp; rotation[5] = sp;
    rotation[6] = sy; rotation[7] = -sp * cy; rotation[8] = cp * cy;
    gl.viewport(0, 0, canvas.width, canvas.height);
    gl.clear(gl.COLOR_BUFFER_BIT);
    gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);
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
    gl.uniform1f(points.u_land_size, clamp(radius / 160, 1.2, 2));
    gl.uniform1f(points.u_time, time);
    gl.uniform1f(points.u_marker, 0);
    gl.uniform3fv(points.u_color, colors.land.rgb);
    gl.bindBuffer(gl.ARRAY_BUFFER, resources.land);
    gl.enableVertexAttribArray(points.a_position);
    gl.vertexAttribPointer(points.a_position, 3, gl.FLOAT, false, 0, 0);
    gl.vertexAttrib1f(points.a_size, 1);
    gl.vertexAttrib1f(points.a_phase, 0);
    gl.drawArrays(gl.POINTS, 0, land.length / 3);
    gl.disableVertexAttribArray(points.a_position);

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

    if (markerCount) {
      gl.useProgram(points.handle);
      gl.uniform1f(points.u_marker, 1);
      gl.uniform3fv(points.u_color, colors.pulse.rgb);
      gl.bindBuffer(gl.ARRAY_BUFFER, resources.markers);
      gl.enableVertexAttribArray(points.a_position);
      gl.enableVertexAttribArray(points.a_size);
      gl.enableVertexAttribArray(points.a_phase);
      gl.vertexAttribPointer(points.a_position, 3, gl.FLOAT, false, 20, 0);
      gl.vertexAttribPointer(points.a_size, 1, gl.FLOAT, false, 20, 12);
      gl.vertexAttribPointer(points.a_phase, 1, gl.FLOAT, false, 20, 16);
      gl.blendFunc(gl.SRC_ALPHA, gl.ONE);
      gl.drawArrays(gl.POINTS, 0, markerCount);
      gl.disableVertexAttribArray(points.a_position);
      gl.disableVertexAttribArray(points.a_size);
      gl.disableVertexAttribArray(points.a_phase);
    }
  }

  function drawMap() {
    if (!context || destroyed || !visible || document.hidden) return;
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
    context.globalAlpha = 0.09;
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
    context.globalAlpha = 0.65;
    context.beginPath();
    for (let i = 0; i < land.length; i += 3) {
      const x = toX(land[i], land[i + 2]), y = toY(land[i + 1]);
      context.moveTo(x + 0.8, y); context.arc(x, y, 0.8, 0, Math.PI * 2);
    }
    context.fill();
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
    for (let i = 0; i < markerCount * 5; i += 5) {
      const x = toX(markerData[i], markerData[i + 2]), y = toY(markerData[i + 1]);
      const size = markerData[i + 3] / 2;
      const glow = context.createRadialGradient(x, y, 1, x, y, size);
      glow.addColorStop(0, colors.pulse.css); glow.addColorStop(1, `${colors.pulse.css}00`);
      context.fillStyle = glow;
      context.beginPath(); context.arc(x, y, size, 0, Math.PI * 2); context.fill();
      context.fillStyle = colors.pulse.css;
      context.beginPath(); context.arc(x, y, 2.4, 0, Math.PI * 2); context.fill();
    }
  }

  function draw() { if (staticMode) drawMap(); else drawWebGL(); }

  function resize() {
    if (destroyed) return;
    const bounds = (staticMode ? map : canvas).getBoundingClientRect();
    width = Math.max(1, bounds.width || stage.clientWidth || 1);
    height = Math.max(1, bounds.height || stage.clientHeight || width);
    dpr = Math.min(2, window.devicePixelRatio || 1);
    radius = Math.min(width, height) * 0.425;
    scale[0] = radius * 2 / width; scale[1] = radius * 2 / height;
    const pixelWidth = Math.round(width * dpr), pixelHeight = Math.round(height * dpr);
    for (const surface of [canvas, map]) {
      if (surface.width !== pixelWidth) surface.width = pixelWidth;
      if (surface.height !== pixelHeight) surface.height = pixelHeight;
    }
    colors = { sphere: readColor(canvas, 'sphere'), land: readColor(canvas, 'land'), pulse: readColor(canvas, 'pulse') };
    draw();
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

  function update(model) {
    if (destroyed) return;
    for (const marker of markers) marker.count = 0;
    for (const region of continentTotals(model)) {
      const marker = byContinent.get(region.continent);
      if (marker && Number.isFinite(region.count)) marker.count = Math.max(0, region.count);
    }
    markerCount = 0;
    for (const marker of markers) {
      if (!marker.count) continue;
      const offset = markerCount++ * 5;
      markerData.set(marker.position, offset);
      markerData[offset + 3] = 30 + Math.min(28, Math.sqrt(marker.count) * 4);
      markerData[offset + 4] = marker.phase;
    }
    arcCount = 0;
    const blocks = Array.isArray(model?.recent_blocks) ? model.recent_blocks : [];
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
      gl.bindBuffer(gl.ARRAY_BUFFER, resources.markers); gl.bufferSubData(gl.ARRAY_BUFFER, 0, markerData);
      gl.bindBuffer(gl.ARRAY_BUFFER, resources.lines); gl.bufferSubData(gl.ARRAY_BUFFER, 0, arcData);
    }
    draw();
  }

  function pointerDown(event) {
    if (staticMode || event.button !== 0 || event.isPrimary === false) return;
    pointer = { id: event.pointerId, x: event.clientX, y: event.clientY, startX: event.clientX, startY: event.clientY, touch: event.pointerType === 'touch', dragging: false };
    if (!pointer.touch) canvas.setPointerCapture(event.pointerId);
  }
  function pointerMove(event) {
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
    if (canvas.hasPointerCapture(event.pointerId)) canvas.releasePointerCapture(event.pointerId);
    pointer = null;
  }
  function keyDown(event) {
    if (staticMode) return;
    if (event.key === 'ArrowLeft') yaw -= 0.13;
    else if (event.key === 'ArrowRight') yaw += 0.13;
    else if (event.key === 'ArrowUp') pitch = clamp(pitch + 0.1, -1.1, 1.1);
    else if (event.key === 'ArrowDown') pitch = clamp(pitch - 0.1, -1.1, 1.1);
    else if (event.key === 'Home') { yaw = INITIAL_YAW; pitch = INITIAL_PITCH; }
    else return;
    event.preventDefault();
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

  return {
    update,
    setPaused(value) { paused = Boolean(value); reconcile(); },
    resize,
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
      map.remove();
      canvas.hidden = original.hidden;
      canvas.style.display = original.display;
      canvas.style.touchAction = original.touchAction;
      if (original.renderer === null) canvas.removeAttribute('data-renderer');
      else canvas.setAttribute('data-renderer', original.renderer);
    },
  };
}
