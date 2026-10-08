/* Design-only harness. It loads the real explorer app and replaces fetch
   before boot. No production code knows about demo data, and unhandled
   requests fail closed instead of touching a real node or gateway.
   Serve the repository root; use ?theme=light or ?theme=dark. */
(() => {
  const query = new URLSearchParams(location.search);
  const theme = query.get('theme');
  const now = Date.parse('2026-10-08T04:15:00Z');
  const height = 842196;
  const hash = (seed) => `0x${seed.toString(16).padStart(64, '0')}`;
  const address = (seed) => `0x${seed.toString(16).padStart(40, '0')}`;
  const root = hash(0x8f2742);
  const endpoint = 'https://demo.eastsea.invalid';
  Date.now = () => now;
  if (theme === 'light' || theme === 'dark') document.documentElement.dataset.theme = theme;
  try {
    localStorage.setItem('aether-explorer.theme', ['light', 'dark'].includes(theme) ? theme : '');
    localStorage.setItem('aether-explorer.node', endpoint);
    localStorage.setItem('aether-explorer.gateway', '');
  } catch { /* An unavailable store keeps the real app's system-theme path. */ }

  const blocks = Array.from({ length: 7 }, (_, i) => ({
    height: height - i,
    hash: hash(0x6da83c + i),
    parent: hash(0x6da83d + i),
    proposer: address(0x46a910 + i),
    timestamp_ms: now - 4000 - i * 12000,
    state_root: root,
    parent_state_root: hash(0x8f2743 + i),
    txs: Array.from({ length: [4, 2, 7, 3, 5, 2, 1][i] }, (_, j) => hash(0xab1200 + i * 10 + j)),
    gas_used: [126840, 84200, 213480, 96240, 164820, 73120, 42600][i],
    prove_gas: 18400 + i * 300,
    base_fee: { exec: '1200', prove: '300' },
    excess: { exec: '0', prove: '0' },
  }));
  const status = {
    chain_id: 7777,
    height,
    timestamp_ms: now - 4000,
    protocol: 7,
    node_protocol: 7,
    newest_scheduled: 7,
    mempool: 0,
    base_fee: { exec: '1200', prove: '300' },
    hash_function: 'BLAKE3',
    state_root: root,
    prover_escrow: '12000000000000000000000',
  };
  const requests = [];
  window.explorerDemo = { requests, height, blocks, status };

  // This bridge only refuses verification; synthetic data must never earn a
  // committee certificate. It also keeps the demo from loading a wasm bundle.
  const unverified = async () => ({ verified: false, reason: 'Demo data · not verified' });
  window.eastsea = { verify: { block: unverified, account: unverified, receipt: unverified } };

  const reply = (value) => new Response(JSON.stringify(value), {
    status: 200,
    headers: { 'Content-Type': 'application/json' },
  });
  window.fetch = async (input, init = {}) => {
    const url = new URL(typeof input === 'string' ? input : input.url, document.baseURI);
    if (!init.method || init.method === 'GET') {
      if (url.pathname.endsWith('/token-sources.json')) {
        return reply({ chains: { '7777': { network: 'demo', seed: [] } } });
      }
      throw new Error(`The design demo does not fetch ${url.pathname}`);
    }
    const { id, method, params = [] } = JSON.parse(init.body);
    requests.push({ method, params });
    let result;
    switch (method) {
      case 'aether_status': result = status; break;
      case 'aether_recentBlocks': result = blocks.slice(0, params[0] ?? 30); break;
      case 'aether_candidates': result = { epoch: 42, candidates: Array.from({ length: 12 }, (_, i) => ({ address: address(i + 1) })) }; break;
      case 'aether_proverStatus': result = { running: true, last_height: height - 1, lag: 1, proofs: 24020 }; break;
      case 'aether_getBlock': result = blocks.find((block) => block.height === params[0]) ?? null; break;
      case 'aether_history': result = { pruned_below: 0 }; break;
      case 'aether_getAccount': result = { balance: '128420000000000000000', nonce: 3, code_size: 0, height, state_root: root }; break;
      case 'aether_rewards': result = []; break;
      case 'eth_getLogs': result = []; break;
      case 'eth_blockNumber': result = `0x${height.toString(16)}`; break;
      case 'eth_call': result = '0x'; break;
      case 'eth_getTransactionReceipt': result = null; break;
      default: return reply({ jsonrpc: '2.0', id, error: { code: -32601, message: `This method is outside the design demo: ${method}` } });
    }
    return reply({ jsonrpc: '2.0', id, result });
  };
})();
