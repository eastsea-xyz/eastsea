export const Brand = Object.freeze({
  project: 'EastSea',
  projectKo: '동해',
  coinName: 'Doubloon',
  coinTicker: 'DBLN',
});

/** The chain id of the legacy testnet. Its coin is shown as test DBLN
 * ("Test Doubloon") like every chain's: the label is display only. */
export const LEGACY_TESTNET_CHAIN_ID = 7780;

/** The native coin's ticker on a chain: DBLN everywhere (the native coin
 * has no on-chain symbol; the old "AETH" label is retired). */
export function coinTicker(_chainId) {
  return Brand.coinTicker;
}

/** The native coin's display name on a chain, same rule as coinTicker. */
export function coinName(chainId) {
  return Number(chainId) === LEGACY_TESTNET_CHAIN_ID ? 'Test Doubloon' : Brand.coinName;
}
