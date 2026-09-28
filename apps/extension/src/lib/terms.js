// The one-time first-run notice, with the same risk points as the app's terms
// (apps/wallet/Sources/Onboarding.swift). Bump `TERMS_VERSION` when the text
// changes materially, and everyone is asked again.

export const TERMS_VERSION = 1;
export const DISCLAIMER_URL = 'https://github.com/kjaylee/aether-node/blob/main/DISCLAIMER.md';

/** Shown one bullet per line; `link` follows them. */
export const NOTICE_POINTS = [
  'Aether is experimental research software on a test network. It is provided as is, without any warranty, has not had an independent security audit, and may have bugs.',
  'Test AETH comes free from the faucet and has no value; it does not carry over to any future network. Nothing here promises a price, a return or a way to cash out.',
  'You use this wallet at your own risk and responsibility, including taxes and following the laws where you live.',
  'Your key is stored encrypted in this browser and never sent anywhere. If you lose this browser profile and have no copy of the private key (Settings → Show private key), nobody can restore the account.',
];
