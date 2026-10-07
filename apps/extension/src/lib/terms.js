import { Brand, coinTicker } from './brand.js';
// The one-time first-run notice, with the same risk points as the app's terms
// (apps/wallet/Sources/Onboarding.swift). Bump `TERMS_VERSION` when the text
// changes materially, and everyone is asked again.

export const TERMS_VERSION = 4;
export const DISCLAIMER_URL = 'https://github.com/eastsea-xyz/eastsea/blob/main/DISCLAIMER.md';

/** Shown one bullet per line; `link` follows them. `chainId` picks the coin's
 * ticker (DBLN on every chain). */
export function noticePoints(chainId = null) {
  const ticker = coinTicker(chainId);
  return [
    `${Brand.project} is built for production. Mainnet has not launched yet; the network running today is the public testnet, and its ${ticker} does not carry over. It is provided as is, without warranty, and has not had an independent security audit yet.`,
    `There is no token sale. The value of ${ticker} is set by the market; nothing here promises a price, a return, a listing or a way to cash out.`,
    'You use this wallet at your own risk and responsibility, including taxes and following the laws where you live.',
    'Your key is stored encrypted in this browser and never sent anywhere. If you lose this browser profile and have no copy of the private key (Settings → Show private key), nobody can restore the account.',
  ];
}
