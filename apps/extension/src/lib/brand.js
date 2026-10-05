export const Brand = Object.freeze({
  project: 'EastSea',
  projectKo: '동해',
  coinName: 'Doubloon',
  coinTicker: 'DBLN',
});

/** The chain id of the legacy testnet, which kept its own coin: test AETH. */
export const LEGACY_TESTNET_CHAIN_ID = 7780;

/** The native coin's ticker on a chain: AETH on the legacy 7780 testnet,
 * Doubloon (DBLN) on every new-genesis chain. */
export function coinTicker(chainId) {
  return Number(chainId) === LEGACY_TESTNET_CHAIN_ID ? 'AETH' : Brand.coinTicker;
}

/** The native coin's display name on a chain, same rule as coinTicker. */
export function coinName(chainId) {
  return Number(chainId) === LEGACY_TESTNET_CHAIN_ID ? 'Test AETH' : Brand.coinName;
}
