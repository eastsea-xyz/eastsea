// Audit R2-2 (docs/research/audit-2-2026-10-03.md): unauthenticated RPC
// metadata must never define the units a transfer signs. Two configured
// endpoints agreeing is not a security quorum — both URLs can belong to one
// operator, and nothing authenticates the answer. This file is the ONLY
// source of "trusted denomination": a shipped, immutable, address-pinned
// list, keyed by chain id + lowercase token address. A token on this list
// sends with these decimals no matter what any node answers; a token not on
// it may be displayed, but its units are "Unverified" and sending them needs
// an explicit confirmation of the exact base-unit count
// (lib/sendIntent.js, ui/popup.js).
//
// How to add an entry: paste the token's chain id and 0x address exactly as
// deployed, with the symbol, name and decimals from the deployment record
// (the repo's deployment JSONs — apps/agent/Resources/dex/*.json,
// token-sources.json — or the contract source), NOT from an RPC read. Every
// entry needs review: a wrong decimals here mis-sizes every send of that
// token, so a human must check it against the deployed contract before it
// ships. Test test/known-tokens.test.mjs checks the shape and that the list
// covers the addresses this repo itself ships as token sources.

/** The native coin of each chain (not an ERC-20; recorded so the trusted
 * denomination table is complete). The extension always shows the native
 * amount at 18 decimals (lib/units.js), independent of any RPC answer.
 * Chain 7780 is the legacy testnet; its coin is shown as DBLN ("Test
 * Doubloon") like every chain's (lib/brand.js). */
export const NATIVE_COINS = Object.freeze({
  7780: Object.freeze({ symbol: 'DBLN', name: 'Test Doubloon', decimals: 18 }),
});

/** Known ERC-20 tokens, by chain id and lowercase address.
 * Chain 7780 (aether-testnet), from apps/agent/Resources/dex/aether-testnet.json
 * and the seed deployment in aether-dex's script/Deploy.s.sol:
 * - WAETH (src/WAETH.sol): name/symbol/decimals are contract constants.
 * - NEB, ORB, CMT (src/Token.sol via TokenFactory): 18 decimals is a contract
 *   constant; names from the seed deployment script. */
export const KNOWN_TOKENS = Object.freeze({
  7780: Object.freeze({
    '0xa2521982a17474cb2f8741c85de653b5282d72b0': Object.freeze({ symbol: 'WAETH', name: 'Wrapped AETH', decimals: 18 }),
    '0x6bc5ded76ccbdc8df35e7cd28b68fed245a74416': Object.freeze({ symbol: 'NEB', name: 'Test Nebula', decimals: 18 }),
    '0x961f8add5ae93ff0700be8abd5f9f8ec69ba4347': Object.freeze({ symbol: 'ORB', name: 'Test Orbit', decimals: 18 }),
    '0xc91367bac92c6de822de8afd0f34ff19fd8f7670': Object.freeze({ symbol: 'CMT', name: 'Test Comet', decimals: 18 }),
  }),
});

/** The shipped entry for `address` on `chainId`, or null. Returns a copy, so
 * callers cannot mutate the shipped table through it. */
export function knownToken(chainId, address) {
  const entry = KNOWN_TOKENS[Number(chainId)]?.[String(address || '').toLowerCase()];
  return entry ? { ...entry } : null;
}

/** Display names for tokens whose on-chain name still carries the retired
 * AETH label. The contract's own symbol and name never change (they live in
 * the contract, and the denomination checks compare against them); only what
 * a person reads does. */
export const DISPLAY_NAMES = Object.freeze({
  7780: Object.freeze({
    '0xa2521982a17474cb2f8741c85de653b5282d72b0': 'Wrapped test DBLN',
  }),
});

/** The name to show for a token: the display name when there is one. */
export function displayTokenName(chainId, address, name) {
  return DISPLAY_NAMES[Number(chainId)]?.[String(address || '').toLowerCase()] ?? name;
}
