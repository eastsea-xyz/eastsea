import { readFile } from 'node:fs/promises';
import init, * as wasm from '../wasm/aether_wasm.js';

let loaded;
export async function loadWasm() {
  loaded ??= init({ module_or_path: await readFile(new URL('../wasm/aether_wasm_bg.wasm', import.meta.url)) });
  await loaded;
  return wasm;
}

/** chrome.storage-like area backed by a Map. */
export function memoryArea() {
  const m = new Map();
  return {
    get: async (k) => structuredClone(m.get(k)),
    set: async (k, v) => { m.set(k, structuredClone(v)); },
    remove: async (k) => { m.delete(k); },
    map: m,
  };
}
