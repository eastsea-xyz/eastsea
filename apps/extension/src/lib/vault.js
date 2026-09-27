// The wallet key: a P-256 key made by WebCrypto in this browser. At rest it is
// encrypted with a key derived from the password (PBKDF2-SHA256, AES-GCM).
// While unlocked, the private JWK sits only in memory-only session storage
// and clears when the browser closes or the lock timer runs out.

const PBKDF2_ITERATIONS = 600_000;
const VAULT_KEY = 'vault';
const SESSION_KEY = 'unlocked';
export const DEFAULT_LOCK_MINUTES = 30;
const EC = { name: 'ECDSA', namedCurve: 'P-256' };

const b64 = {
  enc: (u8) => btoa(String.fromCharCode(...u8)),
  dec: (s) => Uint8Array.from(atob(s), (c) => c.charCodeAt(0)),
};
const b64url = {
  enc: (u8) => b64.enc(u8).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, ''),
  dec: (s) => b64.dec(s.replace(/-/g, '+').replace(/_/g, '/') + '='.repeat((4 - (s.length % 4)) % 4)),
};
export const hex = {
  enc: (u8) => Array.from(u8, (b) => b.toString(16).padStart(2, '0')).join(''),
  dec: (s) => {
    const h = s.replace(/^0x/, '');
    if (!/^([0-9a-fA-F]{2})*$/.test(h)) throw new Error('not hex');
    return Uint8Array.from(h.match(/../g) || [], (x) => parseInt(x, 16));
  },
};

async function passwordKey(password, salt) {
  const base = await crypto.subtle.importKey('raw', new TextEncoder().encode(password), 'PBKDF2', false, ['deriveKey']);
  return crypto.subtle.deriveKey({ name: 'PBKDF2', hash: 'SHA-256', salt, iterations: PBKDF2_ITERATIONS }, base, { name: 'AES-GCM', length: 256 }, false, ['encrypt', 'decrypt']);
}

/** Uncompressed SEC1 public key (65 bytes) from a JWK's x and y. */
export function publicKeyOf(jwk) {
  const out = new Uint8Array(65);
  out[0] = 4;
  out.set(b64url.dec(jwk.x), 1);
  out.set(b64url.dec(jwk.y), 33);
  return out;
}

export class Vault {
  /**
   * `local`/`session`: {get(key), set(key, value), remove(key)} (chrome.storage
   * wrappers in the extension, maps in tests). `addressOf(pubkey)` comes from wasm.
   */
  constructor({ local, session, addressOf, now = () => Date.now() }) {
    this.local = local;
    this.session = session;
    this.addressOf = addressOf;
    this.now = now;
  }

  async exists() {
    return Boolean(await this.local.get(VAULT_KEY));
  }

  /** {address, publicKey (hex)} without unlocking, or null. */
  async info() {
    const v = await this.local.get(VAULT_KEY);
    return v ? { address: v.address, publicKey: v.publicKey } : null;
  }

  static checkPassword(password) {
    if (typeof password !== 'string' || password.length < 8) throw new Error('Use a password of at least 8 characters.');
  }

  /** New key. Returns the address. */
  async create(password) {
    Vault.checkPassword(password);
    const pair = await crypto.subtle.generateKey(EC, true, ['sign', 'verify']);
    const jwk = await crypto.subtle.exportKey('jwk', pair.privateKey);
    return this.store(jwk, password);
  }

  /** Existing 32-byte private key (hex). `publicKeyFromSecret` comes from wasm. */
  async importSecret(secretHex, password, publicKeyFromSecret) {
    Vault.checkPassword(password);
    const d = hex.dec(secretHex.trim());
    if (d.length !== 32) throw new Error('A private key is 32 bytes (64 hex characters).');
    const pub = publicKeyFromSecret(d);
    const jwk = { kty: 'EC', crv: 'P-256', d: b64url.enc(d), x: b64url.enc(pub.slice(1, 33)), y: b64url.enc(pub.slice(33, 65)), ext: true };
    await crypto.subtle.importKey('jwk', jwk, EC, false, ['sign']); // rejects a bad key before storing
    return this.store(jwk, password);
  }

  async store(jwk, password) {
    const salt = crypto.getRandomValues(new Uint8Array(16));
    const iv = crypto.getRandomValues(new Uint8Array(12));
    const key = await passwordKey(password, salt);
    const ct = new Uint8Array(await crypto.subtle.encrypt({ name: 'AES-GCM', iv }, key, new TextEncoder().encode(JSON.stringify(jwk))));
    const pub = publicKeyOf(jwk);
    const address = this.addressOf(pub);
    await this.local.set(VAULT_KEY, { v: 1, kdf: 'PBKDF2-SHA256', iterations: PBKDF2_ITERATIONS, salt: b64.enc(salt), iv: b64.enc(iv), ct: b64.enc(ct), publicKey: hex.enc(pub), address });
    await this.keepUnlocked(jwk);
    return address;
  }

  async decrypt(password) {
    const v = await this.local.get(VAULT_KEY);
    if (!v) throw new Error('No wallet yet.');
    const key = await passwordKey(password, b64.dec(v.salt));
    try {
      const pt = await crypto.subtle.decrypt({ name: 'AES-GCM', iv: b64.dec(v.iv) }, key, b64.dec(v.ct));
      return JSON.parse(new TextDecoder().decode(pt));
    } catch {
      throw new Error('Wrong password.');
    }
  }

  async unlock(password, minutes) {
    const jwk = await this.decrypt(password);
    await this.keepUnlocked(jwk, minutes);
    return (await this.info()).address;
  }

  async keepUnlocked(jwk, minutes) {
    const m = minutes ?? (await this.local.get('lockMinutes')) ?? DEFAULT_LOCK_MINUTES;
    await this.session.set(SESSION_KEY, { jwk, until: this.now() + m * 60_000 });
  }

  async lock() {
    await this.session.remove(SESSION_KEY);
  }

  async unlocked() {
    const s = await this.session.get(SESSION_KEY);
    if (!s) return false;
    if (this.now() >= s.until) {
      await this.lock();
      return false;
    }
    return true;
  }

  /** Raw r‖s P-256 signature over SHA-256(message) (what the chain verifies). */
  async sign(message) {
    const s = await this.session.get(SESSION_KEY);
    if (!s || this.now() >= s.until) throw new Error('The wallet is locked.');
    const key = await crypto.subtle.importKey('jwk', s.jwk, EC, false, ['sign']);
    return new Uint8Array(await crypto.subtle.sign({ name: 'ECDSA', hash: 'SHA-256' }, key, message));
  }

  /** The private key as hex, for a backup. Needs the password again. */
  async revealSecret(password) {
    const jwk = await this.decrypt(password);
    return hex.enc(b64url.dec(jwk.d));
  }

  /** Forget this key (after the user has a backup). */
  async erase() {
    await this.lock();
    await this.local.remove(VAULT_KEY);
  }
}
