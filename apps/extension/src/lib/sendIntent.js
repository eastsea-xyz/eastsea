// Audit R2-5 (docs/research/audit-2-2026-10-03.md): the extension must not be
// able to sign different token units than its open confirmation showed. The
// confirmation produces one immutable send intent — the token, the recipient,
// the exact base-unit integer, the decimals that were shown, and the pin
// generation the display was built from — and the wallet core executes
// exactly that. It never re-parses an amount text under whatever decimals are
// stored later, and it refuses when the stored pin generation (or the token's
// change flag) moved since the confirmation (lib/pinStore.js bumps the
// generation on every content change).
//
// Audit R2-2 rides the same intent: for a token on the shipped list
// (lib/knownTokens.js) the units come from that list and no RPC answer can
// move them; for any other token the intent is only valid when the user
// acknowledged the exact base-unit count on the confirmation screen.

import { erc20TransferCalldata, parseTokenAmount } from './tokens.js';
import { sendBlocker } from './tokenPin.js';

/** A plain non-negative decimal integer, the only acceptable `baseUnits`. */
const BASE_UNITS = /^\d+$/;

/**
 * Build the intent from what the user saw: `amountText` is parsed ONCE, under
 * `token.decimals` — the decimals the confirmation displayed — and every later
 * step uses the resulting integer, never the text again. `token`:
 * `{ address, decimals, trusted?, acknowledged?, pinGeneration }`.
 */
export function buildSendIntent({ recipient, amountText, token }) {
  const address = String(token?.address || '').toLowerCase();
  if (!/^0x[0-9a-f]{40}$/.test(address)) throw new Error('the token is not an address');
  const to = String(recipient || '').trim();
  if (!/^0x[0-9a-fA-F]{40}$/.test(to)) throw new Error('the recipient is not an address');
  const decimals = Number(token.decimals);
  if (!Number.isSafeInteger(decimals) || decimals < 0 || decimals > 77) throw new Error('the token’s decimals are not usable');
  const units = parseTokenAmount(amountText, decimals);
  return {
    token: {
      address,
      decimals,
      trusted: Boolean(token.trusted),
      acknowledged: Boolean(token.acknowledged),
      pinGeneration: Number(token.pinGeneration) || 0,
    },
    recipient: to,
    baseUnits: units.toString(),
  };
}

/**
 * Check an intent against the pins stored NOW and derive the calldata to sign.
 * `pins`: the stored pin state (with its generation); `known`: the shipped
 * list entry for this token (lib/knownTokens.js), or null. Throws one plain
 * sentence when the intent no longer matches what is stored; the caller must
 * not fall back to any re-derived amount.
 */
export function checkSendIntent(intent, { pins, known = null }) {
  const i = intent ?? {};
  const t = i.token ?? {};
  const baseUnits = String(i.baseUnits ?? '');
  if (!BASE_UNITS.test(baseUnits) || BigInt(baseUnits) > 2n ** 256n - 1n) throw new Error('The confirmed amount is not a usable number of base units.');
  if (!/^0x[0-9a-fA-F]{40}$/.test(String(i.recipient || ''))) throw new Error('the recipient is not an address');
  // The pin generation moved since the confirmation: whatever the popup showed
  // may no longer be what storage says. Sign nothing; the user starts again.
  const generation = Number(pins?.generation) || 0;
  if (generation !== (Number(t.pinGeneration) || 0)) throw new Error('This token’s details changed since the send was confirmed. Close this and confirm the send again.');
  if (known) {
    // Trusted denomination: the shipped list decides the units. A node answer
    // that disagrees is display noise, never a reason to change them.
    if (Number(t.decimals) !== known.decimals) throw new Error('This token’s units come from the list shipped with the wallet; confirm the send again.');
    return erc20TransferCalldata(i.recipient, BigInt(baseUnits));
  }
  // Unverified units: only an explicit acknowledgement of the exact base-unit
  // count authorizes signing, and a flagged, unreviewed metadata change still
  // blocks the send (audit A3/R2-2).
  if (t.acknowledged !== true) throw new Error('This token is not on the wallet’s trusted list, so sending it needs your confirmation of the exact number of units.');
  const block = sendBlocker(pins, t.address);
  if (block) throw new Error(block);
  const pin = pins?.tokens?.[t.address];
  if (!pin) throw new Error('This token’s details are not confirmed yet. Open Assets and let the wallet confirm them first.');
  if (pin.decimals !== Number(t.decimals)) throw new Error('This token’s details changed since the send was confirmed. Close this and confirm the send again.');
  return erc20TransferCalldata(i.recipient, BigInt(baseUnits));
}
