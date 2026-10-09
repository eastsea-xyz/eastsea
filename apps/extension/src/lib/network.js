import { DEFAULT_RPCS } from './rpc.js';

export const DEV_CHAIN_ID = 7777;

export function networkSettings(defaultNetwork, { developerMode = false, developmentNetwork = false, developmentPort = 18546, rpcs = [] } = {}) {
  const chainId = Number(defaultNetwork?.chain_id);
  if (!Number.isSafeInteger(chainId) || chainId <= 0) throw new Error('Bundled network.json has no valid chain id.');
  if (developerMode && developmentNetwork) {
    const port = Number(developmentPort);
    if (!Number.isInteger(port) || port < 1024 || port > 65535) throw new Error('The local development port must be 1024–65535.');
    return { chainId: DEV_CHAIN_ID, urls: [`http://127.0.0.1:${port}`], development: true, port };
  }
  return { chainId, urls: [...DEFAULT_RPCS, ...rpcs], development: false, port: null };
}
