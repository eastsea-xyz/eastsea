// The five views. Each is one async function: it fetches what the node serves,
// then builds DOM through `h` (never HTML strings). Everything the RPC gives is
// finalized — the node serves no other kind — and none of it is verified here,
// which is why every page carries its "read from the node" line.

import { card, copyButton, dot, kv, message, pill, sourceLine, table, h } from './dom.js';
import { coinTicker, displayTokenName, formatAeth, formatInt, formatRate, formatTokenAmount, localTime, droppedText, notIncludedText, shortHex, timeAgo, toBigInt, txRate } from './format.js';
import { TRANSFER_TOPIC, decodeApproval, decodeTransfer, revertReason, wordAddress } from './abi.js';
import { looksLikeOfficial, officialTokens, originBadge, tokenInfo, tokenOrigin, totalSupply } from './erc20.js';
import { NOT_COMMITTED } from './verify.js';
import { readVerdict } from './peers.js';

// ---- little shared builders ----

export function blockLink(height) {
  return h('a', { href: `#/block/${height}` }, formatInt(height));
}

export function txLink(hash) {
  return h('a', { class: 'mono', href: `#/tx/${hash}`, title: hash }, shortHex(hash, 10, 6));
}

export function addrLink(address) {
  return h('a', { class: 'mono', href: `#/account/${address.toLowerCase()}`, title: address }, shortHex(address, 6, 4));
}

/** "SYMBOL · 0x8a9B…F41c" — a symbol is never shown without its address
 * (the wallet's rule; a symbol alone is trivial to fake). */
export async function tokenLabel(ctx, address) {
  const a = String(address).toLowerCase();
  const info = await ctx.token(a);
  return h('a', { class: 'mono', href: `#/token/${a}`, title: a },
    info ? `${info.symbol === '???' ? '?' : info.symbol} · ${shortHex(a, 6, 4)}` : shortHex(a, 6, 4));
}

function hashValue(full) {
  return h('span', { class: 'row tight' }, h('span', { class: 'mono wrap' }, full), copyButton(full));
}

function withCopy(text) {
  return h('span', { class: 'row tight' }, h('span', { class: 'mono wrap' }, text), copyButton(text));
}

/** `0x` if the node sent a bare hex digest (block summaries do). */
export function ox(hex) {
  if (hex == null) return '—';
  const s = String(hex || '');
  return s.startsWith('0x') ? s : `0x${s}`;
}

/** eth_getLogs only ever scans the newest 2,000 finalized blocks; both the
 * queries and their honest labels below assume that window. */
function logsWindow() {
  return 2000;
}

function hexHeight(n) {
  return `0x${Math.max(0, Number(n)).toString(16)}`;
}

/** The committee-certificate badge. Only a verified answer earns it; a
 * verifier this page has that refused says why; with no verifier at all the
 * row stays quiet — the source line already says the page is not verified. */
function certificateBadge(ctx, v) {
  if (v?.verified) return pill('verified by committee certificate', 'good');
  if (ctx.verifier?.kind === 'none') return null;
  return pill(`not verified${v?.reason ? ` — ${v.reason}` : ''}`, 'plain');
}

async function headHeight(ctx) {
  const r = await ctx.node.call('eth_blockNumber');
  return parseInt(String(r).replace(/^0x/, ''), 16);
}

function sortableLogs(logs) {
  return [...(logs || [])].sort((a, b) => {
    const ba = parseInt(String(a.blockNumber).replace(/^0x/, ''), 16);
    const bb = parseInt(String(b.blockNumber).replace(/^0x/, ''), 16);
    if (ba !== bb) return bb - ba;
    return parseInt(String(b.logIndex).replace(/^0x/, ''), 16) - parseInt(String(a.logIndex).replace(/^0x/, ''), 16);
  });
}

// ---- home ----

export async function homeView(ctx) {
  const [status, blocks, candidates, prover] = await Promise.all([
    ctx.node.call('aether_status'),
    ctx.node.call('aether_recentBlocks', [30]),
    ctx.node.call('aether_candidates').catch(() => null),
    ctx.node.call('aether_proverStatus').catch(() => null),
  ]);
  const verifiedHead = !!readVerdict(status);
  const rate = txRate(blocks);
  const outdated = status.node_protocol < status.newest_scheduled;

  const tiles = h('div', { class: 'tiles' },
    tile('Finalized height', blockLink(status.height), `${timeAgo(status.timestamp_ms)} · finalized`, 'major'),
    tile('Transaction rate', `${formatRate(rate?.perSec)} tx/s`, rate ? `${formatInt(rate.txs)} txs across ${blocks.length} blocks` : `${blocks.length} block${blocks.length === 1 ? '' : 's'} in view`),
    tile('Committee', candidates ? `${candidates.candidates.length} candidates` : '—', candidates ? `registry epoch ${formatInt(candidates.epoch)}` : verifiedHead ? 'registry proof unavailable' : 'registry unreadable'),
    tile('Protocol', `${status.protocol}`, [
      h('span', { class: 'muted' }, verifiedHead ? 'from certified block' : `node ${status.node_protocol} · scheduled ${status.newest_scheduled}`),
      outdated ? pill('update available', 'warn') : null,
    ]),
    tile('Mempool', status.mempool == null ? '—' : formatInt(status.mempool), verifiedHead ? 'uncommitted · unavailable' : 'waiting for a block'),
    tile('Base fee', status.base_fee ? `${formatInt(toBigInt(status.base_fee.exec))} wei` : '—', status.base_fee ? `exec · prove ${formatInt(toBigInt(status.base_fee.prove))} wei` : 'state proof unavailable'),
    verifiedHead ? tile('Prover', '—', 'uncommitted · unavailable') : proverTile(prover),
  );

  const chain = card('Chain', kv([
    ['Chain id', String(status.chain_id)],
    ['Hash function', status.hash_function],
    ['State root', status.state_root == null ? '— · state proof unavailable' : withCopy(ox(status.state_root))],
    ...(verifiedHead ? [['Certified parent state root', withCopy(ox(status.parent_state_root))]] : []),
    ['Prover escrow', status.prover_escrow == null ? '— · state proof unavailable' : `${formatAeth(status.prover_escrow)} ${coinTicker(status.chain_id)}`],
  ]));

  const list = card(`Latest blocks`, table(
    ['Height', 'Hash', 'Proposer', 'Txs', 'Gas used', 'Age'],
    (blocks || []).map((b) => [
      blockLink(b.height),
      h('a', { class: 'mono', href: `#/block/${b.height}`, title: ox(b.hash) }, shortHex(ox(b.hash), 10, 6)),
      addrLink(b.proposer),
      formatInt(b.txs.length),
      formatInt(b.gas_used),
      timeAgo(b.timestamp_ms),
    ])));

  return h('div', { class: 'stack' },
    sourceLine(ctx.node, `chain ${status.chain_id} · finalized height ${formatInt(status.height)}`, status),
    tiles, list, chain);
}

function tile(label, value, sub, extra = '') {
  return h('div', { class: `tile ${extra}` }, h('div', { class: 'tile-label' }, label),
    h('div', { class: 'tile-value' }, value), h('div', { class: 'tile-sub' }, sub));
}

function proverTile(prover) {
  if (!prover?.running) return tile('Prover', 'not running', 'this node proves nothing');
  const last = prover.last_height;
  return tile('Prover', last == null ? 'starting' : `proven ≤ ${formatInt(last)}`,
    `lag ${formatInt(prover.lag ?? 0)} · ${formatInt(prover.proofs)} proofs${prover.error ? ' · error' : ''}`);
}

// ---- block ----

export async function blockView(ctx, height) {
  const [block, prover] = await Promise.all([
    ctx.node.call('aether_getBlock', [height]),
    ctx.node.call('aether_proverStatus').catch(() => null),
  ]);
  if (!block) return unknownBlock(ctx, height);

  const verifiedBlock = !!readVerdict(block);
  const proof = verifiedBlock ? card('Proof', message('plain', 'The block certificate is verified. Prover activity is uncommitted and unavailable from public peers.')) : proofCard(height, prover);
  const certificate = await ctx.verifier?.block(ctx.node, height, block);
  const rows = await blockTxs(ctx, block);
  const prev = h('a', { href: `#/block/${height - 1}` }, `← ${formatInt(height - 1)}`);
  const next = h('a', { href: `#/block/${height + 1}` }, `${formatInt(height + 1)} →`);

  return h('div', { class: 'stack' },
    h('div', { class: 'row spread' },
      h('h2', { class: 'page-title' }, `Block ${formatInt(height)}`),
      h('nav', { class: 'row tight', 'aria-label': 'Neighbour blocks' }, height > 0 ? prev : null, next)),
    block.pruned
      ? message('warn', `Pruned on this node: it keeps summaries from height ${(await ctx.node.call('aether_history')).pruned_below} up, this one only as a record (era ${Math.floor(height / 8192)}).`)
      : null,
    card('Header', kv([
      ['Height', formatInt(block.height)],
      ['Status', h('span', { class: 'row tight' }, pill('finalized', 'good'), h('span', { class: 'muted small' }, verifiedBlock ? 'verified certified bytes' : 'served by this node'))],
      ['Certificate', certificateBadge(ctx, certificate)],
      ['Hash', hashValue(ox(block.hash))],
      ['Parent', hashValue(ox(block.parent))],
      ['Proposer', addrLink(block.proposer)],
      ['Timestamp', `${localTime(block.timestamp_ms)} (${timeAgo(block.timestamp_ms)})`],
      ['State root', block.state_root == null ? '— · state proof unavailable' : withCopy(ox(block.state_root))],
      ['Parent state root', withCopy(ox(block.parent_state_root))],
      ['Transactions', formatInt(block.txs.length)],
      ['Gas used', `${formatInt(block.gas_used)} exec · ${formatInt(block.prove_gas)} prove`],
      ['Base fee paid', block.base_fee ? `${formatInt(toBigInt(block.base_fee.exec))} exec · ${formatInt(toBigInt(block.base_fee.prove))} prove wei` : '—'],
      ['Fee excess after', block.excess ? `${formatInt(toBigInt(block.excess.exec))} exec · ${formatInt(toBigInt(block.excess.prove))} prove` : '—'],
    ])),
    proof,
    card(`Transactions (${block.txs.length})`, rows.length ? table(['Status', 'Hash', 'Gas used', 'Notes'], rows)
      : message('plain', 'No transactions in this block.')),
    sourceLine(ctx.node, null, block));
}

async function unknownBlock(ctx, height) {
  const [status, history] = await Promise.all([
    ctx.node.call('aether_status').catch(() => null),
    ctx.node.call('aether_history').catch(() => null),
  ]);
  const ahead = status && height > status.height;
  return h('div', { class: 'stack' },
    h('h2', { class: 'page-title' }, `Block ${formatInt(height)}`),
    message('warn', ahead
      ? `Not built yet: this node's finalized height is ${formatInt(status.height)}.`
      : `This node knows no block at height ${formatInt(height)}${history && height < history.pruned_below ? ` — it keeps blocks from ${formatInt(history.pruned_below)} up` : ''}.`),
    sourceLine(ctx.node));
}

/** The proof card. Honest by construction: it reports what this node's prover
 * has done, never claiming an on-chain record the RPC does not serve. */
function proofCard(height, prover) {
  if (!prover?.running) {
    return card('Proof', message('plain', 'This node runs no prover; nothing is known here about proofs of this block.'));
  }
  const last = prover.last_height;
  const state = last != null && height <= last
    ? dot('done', `Proven — this node's prover is at ${formatInt(last)} (${formatInt(prover.proofs)} proofs in total)`)
    : prover.proving === height
      ? dot('pending', 'Proving now on this node')
      : dot('pending', `Not proven by this node yet — its prover is at ${last == null ? 'nothing yet' : formatInt(last)}`);
  return card('Proof', state,
    h('p', { class: 'small muted' }, `Program ${shortHex(prover.program, 10, 6)}. Proof status is this node's prover's view, not an on-chain record.`));
}

async function blockTxs(ctx, block) {
  const hashes = block.txs.slice(0, 50);
  // A pruned block's receipts are gone with it; nothing to ask the node for.
  const receipts = block.pruned ? [] : await Promise.all(hashes.map((x) => ctx.node.call('aether_getReceipt', [x]).catch(() => null)));
  const rows = [];
  hashes.forEach((hash, i) => {
    const r = receipts[i];
    const receipt = r?.receipt;
    if (block.pruned || !r) {
      rows.push([dot('pending', block.pruned ? 'pruned' : 'no receipt'), txLink(hash), '—', '']);
      return;
    }
    const notes = [];
    if (receipt.contract_address) notes.push(pill('contract created', 'good'));
    const transfer = (receipt.events || []).find((e) => decodeTransfer(e));
    if (transfer) notes.push('ERC-20 transfer');
    else if ((receipt.events || []).length) notes.push(`${receipt.events.length} event${receipt.events.length === 1 ? '' : 's'}`);
    rows.push([
      receipt.success ? dot('done', 'success') : dot('failed', 'failed'),
      txLink(hash),
      formatInt(receipt.gas_used),
      h('span', { class: 'row tight wrap' }, notes),
    ]);
  });
  if (block.txs.length > hashes.length) rows.push([null, `${block.txs.length - hashes.length} more not shown`, null, null]);
  return rows;
}

// ---- transaction ----

export async function txView(ctx, hash) {
  const r = await ctx.node.call('aether_getReceipt', [hash]);
  if (r == null) {
    return h('div', { class: 'stack' },
      h('h2', { class: 'page-title' }, 'Transaction'),
      withCopy(hash),
      message('warn', 'No transaction with this hash is known to this node — it may never have existed, belong to another chain, or be older than what this node keeps receipts for.'),
      sourceLine(ctx.node));
  }
  if (r.pending) {
    ctx.pollNow = true;
    const why = notIncludedText(r.waiting);
    return h('div', { class: 'stack' },
      h('h2', { class: 'page-title' }, 'Transaction'),
      withCopy(hash),
      card('Status', dot('pending', why
        ? `In the mempool — not in a block yet: ${why}. This page re-checks while it is open.`
        : 'In the mempool — waiting for a block. This page re-checks while it is open.')),
      sourceLine(ctx.node));
  }
  // Left this node's mempool without a block (bug #5): say why. Not final
  // (round 2, finding 5): a later receipt supersedes it, so keep checking.
  if (r.status === 'dropped') {
    ctx.pollNow = true;
    return h('div', { class: 'stack' },
      h('h2', { class: 'page-title' }, 'Transaction'),
      withCopy(hash),
      card('Status', dot('pending', droppedText(r.reason))),
      sourceLine(ctx.node));
  }
  ctx.pollNow = false;
  const receipt = r.receipt;
  const created = receipt.contract_address;
  const failedWhy = !receipt.success && /^0x08c379a0/.test(String(receipt.output)) ? ` — ${revertReason(String(receipt.output))}` : '';
  // Modern receipts have a certified Merkle root. Legacy blocks may still
  // lack that commitment and must never earn a verified badge.
  const proof = await ctx.verifier?.receipt(ctx.node, hash, r);

  return h('div', { class: 'stack' },
    h('h2', { class: 'page-title' }, 'Transaction'),
    withCopy(hash),
    card('Receipt', kv([
      ['Status', receipt.success ? dot('done', 'success') : dot('failed', `failed${failedWhy}`)],
      ['Block', h('span', { class: 'row tight' }, blockLink(r.height), h('span', { class: 'muted small' }, '(finalized)'))],
      ['Certificate', h('span', { class: 'row tight' }, certificateBadge(ctx, proof),
        proof?.reason === NOT_COMMITTED ? h('span', { class: 'muted small' }, 'this block has no receipt commitment') : null)],
      ['Gas used', `${formatInt(receipt.gas_used)} exec · ${formatInt(receipt.prove_gas)} prove`],
      ['Logs', formatInt(receipt.logs)],
      ['Contract created', created ? h('span', { class: 'row tight' }, addrLink(created), pill('creation', 'good')) : '—'],
    ])),
    await eventsCard(ctx, receipt),
    card('Output', receipt.output && receipt.output !== '0x'
      ? h('details', {}, h('summary', {}, `${String(receipt.output).length / 2 - 1} bytes`), h('div', { class: 'mono wrap small' }, receipt.output))
      : h('span', { class: 'muted' }, 'none')),
    sourceLine(ctx.node, `finalized in block ${formatInt(r.height)}`, r));
}

/** Decoded events: ERC-20 `Transfer` and `Approval` by name and amount, every
 * other log as the raw triple (address, topics, data). */
async function eventsCard(ctx, receipt) {
  const events = receipt.events || [];
  if (!events.length) return card('Events', h('span', { class: 'muted' }, 'none'));
  const rows = [];
  for (const e of events) {
    const transfer = decodeTransfer(e);
    const approval = decodeApproval(e);
    if (!transfer && !approval) {
      rows.push(h('div', { class: 'event' },
        h('div', { class: 'row tight' }, pill('event'), h('span', { class: 'mono small' }, shortHex(e.topics[0], 10, 6))),
        h('details', {}, h('summary', { class: 'small muted' }, 'raw log'),
          kv([['Address', addrLink(e.address)], ['Topics', h('span', { class: 'mono small wrap' }, e.topics.join(' '))], ['Data', h('span', { class: 'mono small wrap' }, String(e.data))]]))));
      continue;
    }
    const info = await ctx.token(e.address);
    const kind = transfer ? 'Transfer' : 'Approval';
    const [one, two] = transfer ? [transfer.from, transfer.to] : [approval.owner, approval.spender];
    const amount = (transfer || approval).value;
    rows.push(h('div', { class: 'event' },
      h('div', { class: 'row tight wrap' },
        pill(kind, 'plain'),
        h('strong', {}, info ? `${formatTokenAmount(amount, info.decimals)} ${info.symbol === '???' ? '' : info.symbol}` : `${amount.toString()} (raw)`),
        h('span', { class: 'muted' }, 'of'), await tokenLabel(ctx, e.address)),
      h('div', { class: 'row tight wrap small' },
        transfer ? h('span', {}, 'from ') : h('span', {}, 'owner '),
        addrLink(one),
        transfer ? h('span', {}, ' to ') : h('span', {}, ' approves '),
        addrLink(two))));
  }
  return card(`Events (${events.length})`, ...rows);
}

// ---- account ----

export async function accountView(ctx, address) {
  const a = String(address).toLowerCase();
  const [account, token, rewards] = await Promise.all([
    ctx.node.call('aether_getAccount', [a]),
    tokenInfo(a, ctx.read),
    ctx.node.call('aether_rewards', [a, 10]).catch(() => []),
  ]);
  const certificate = await ctx.verifier?.account(ctx.node, a, account);

  const els = h('div', { class: 'stack' },
    h('h2', { class: 'page-title' }, account.code_size > 0 ? 'Contract' : 'Account'),
    withCopy(a),
    account.code_size > 0 ? pill(`code · ${formatInt(account.code_size)} bytes`, 'plain') : null,
    token ? h('p', { class: 'small' }, h('a', { href: `#/token/${a}` }, `ERC-20 token ${token.symbol} · view the token page →`)) : null,
    card('State (finalized)', kv([
      ['Balance', `${formatAeth(account.balance)} ${coinTicker(ctx.chainId)}`],
      ['Raw balance', `${toBigInt(account.balance).toString()} wei`],
      ['Nonce', formatInt(account.nonce)],
      ['Code', account.code_size == null ? 'code proof unavailable' : account.code_size > 0 ? `${formatInt(account.code_size)} bytes` : 'none'],
      ['At', h('span', { class: 'row tight' }, blockLink(account.height), h('span', { class: 'muted small' }, `state root ${shortHex(ox(account.state_root), 10, 6)}`))],
      ['Certificate', certificateBadge(ctx, certificate)],
    ])),
    sourceLine(ctx.node, `height ${formatInt(account.height)}`, account));

  const rewardsCard = rewardCard(rewards, coinTicker(ctx.chainId));
  if (rewardsCard) els.append(rewardsCard);
  els.append(await transfersCard(ctx, a));
  return els;
}

/** The node keeps reward records per payout address (aether_rewards); an
 * ordinary account simply has none, so the card appears only when there is one. */
function rewardCard(records, ticker = 'DBLN') {
  if (!records || !records.length) return null;
  const rows = [...records].reverse().map((r) => [
    r.kind === 'node' ? pill('node reward', 'plain') : pill('proof reward', 'plain'),
    r.proven != null ? formatInt(r.proven) : '—',
    `${formatAeth(r.amount)} ${ticker}`,
    blockLink(r.height),
    timeAgo(r.timestamp_ms),
  ]);
  return card('Latest rewards', table(['Kind', 'Proven block', 'Amount', 'Paid in block', 'When'], rows));
}

/** ERC-20 transfers to and from the address, from `eth_getLogs` (the newest
 * 2,000 blocks — the whole window the node scans). Native-coin sends emit no
 * log and cannot be searched by address on this node; the card says so. */
async function transfersCard(ctx, address) {
  const word = wordAddress(address);
  let head;
  try {
    head = await headHeight(ctx);
  } catch {
    return card('ERC-20 transfers', message('plain', 'The node would not answer the log scan.'));
  }
  const from = { fromBlock: hexHeight(head - logsWindow() + 1), toBlock: 'latest', topics: [TRANSFER_TOPIC, word] };
  const to = { fromBlock: hexHeight(head - logsWindow() + 1), toBlock: 'latest', topics: [TRANSFER_TOPIC, null, word] };
  const [out, inn] = await Promise.all([
    ctx.node.call('eth_getLogs', [from]).catch(() => []),
    ctx.node.call('eth_getLogs', [to]).catch(() => []),
  ]);
  const seen = new Set();
  const logs = sortableLogs([...out, ...inn]).filter((l) => {
    const k = `${l.transactionHash}:${l.logIndex}`;
    if (seen.has(k)) return false;
    seen.add(k);
    return true;
  }).slice(0, 25);

  return card('ERC-20 transfers', h('p', { class: 'small muted' }, `Newest ${logsWindow().toLocaleString('en-US')} blocks only (the node's eth_getLogs window); native-coin transfers emit no events and cannot be searched by address.`),
    logs.length ? await transferTable(ctx, logs, address) : h('span', { class: 'muted' }, 'none in that window'));
}

async function transferTable(ctx, logs, pov) {
  const tokens = new Map();
  const rows = [];
  for (const log of logs) {
    const t = decodeTransfer(log);
    if (!t) continue;
    const token = String(log.address).toLowerCase();
    if (!tokens.has(token)) tokens.set(token, await ctx.token(token));
    const info = tokens.get(token);
    const outgoing = t.from === pov;
    rows.push([
      outgoing ? pill('sent', 'plain') : pill('received', 'plain'),
      h('a', { class: 'mono', href: `#/token/${token}`, title: token }, info ? `${info.symbol === '???' ? '?' : info.symbol} · ${shortHex(token, 6, 4)}` : shortHex(token, 6, 4)),
      info ? `${formatTokenAmount(t.value, info.decimals)}` : `${t.value.toString()} (raw)`,
      addrLink(outgoing ? t.to : t.from),
      blockLink(parseInt(String(log.blockNumber).replace(/^0x/, ''), 16)),
      txLink(log.transactionHash),
    ]);
  }
  return table(['Direction', 'Token', 'Amount', 'Counterparty', 'Block', 'Transaction'], rows);
}

// ---- token ----

export async function tokenView(ctx, address) {
  const a = String(address).toLowerCase();
  const info = await tokenInfo(a, ctx.read);
  if (!info) {
    return h('div', { class: 'stack' },
      h('h2', { class: 'page-title' }, 'Token'),
      withCopy(a),
      message('warn', 'This address does not answer ERC-20 metadata calls — not a token, or not readable through this node.'),
      h('p', {}, h('a', { href: `#/account/${a}` }, 'View it as an account →')),
      sourceLine(ctx.node));
  }
  const [supply, transfers] = await Promise.all([
    totalSupply(a, ctx.read),
    tokenTransfers(ctx, a),
  ]);
  const badges = h('div', { class: 'row tight wrap', id: 'origin' }, pill('checking origin…', 'plain'));
  fillOrigin(ctx, a, badges);

  return h('div', { class: 'stack' },
    h('div', { class: 'row spread wrap' },
      h('div', { class: 'hero' }, h('div', { class: 'tile-value big' }, info.symbol === '???' ? '?' : info.symbol), h('div', { class: 'tile-sub' }, displayTokenName(a, info.name) || a)),
      badges),
    withCopy(a),
    card('Metadata', kv([
      ['Name', displayTokenName(a, info.name) || '—'],
      ['Symbol', info.symbol],
      ['Decimals', formatInt(info.decimals)],
      ['Total supply', supply != null ? `${formatTokenAmount(supply, info.decimals)} ${info.symbol === '???' ? '' : info.symbol}` : '—'],
      ['Raw supply', supply != null ? `${supply.toString()} base units` : '—'],
    ])),
    await impersonationNote(ctx, info),
    transfers,
    sourceLine(ctx.node, 'metadata via read-only eth_call'));
}

/** The origin scan walks the same lists the wallet walks (launchpad, DEX token
 * factory, pools) and can take a few seconds, so the badge fills in when it
 * lands. `unverified` is the wallet's zero-trust label, not a fraud verdict. */
async function fillOrigin(ctx, address, holder) {
  try {
    const { text, kind } = originBadge(await ctx.origin(address));
    holder.replaceChildren(pill(text, kind));
  } catch {
    holder.replaceChildren(pill('origin unknown', 'plain'));
  }
}

/** The impersonation check from the wallet's send policy: a symbol or name
 * equal to — or one edit away from — an official token's is a warning, not a
 * judgement; the address is what settles it. */
async function impersonationNote(ctx, info) {
  const sources = ctx.sources();
  if (!sources) return null;
  const official = await officialTokens(sources, ctx.read);
  return looksLikeOfficial(info, official)
    ? message('warn', `The symbol or name resembles an official token (${official.map((o) => o.symbol).join(', ')}). Check the address, not the symbol.`)
    : null;
}

async function tokenTransfers(ctx, token) {
  let head;
  try {
    head = await headHeight(ctx);
  } catch {
    return card('Transfers', message('plain', 'The node would not answer the log scan.'));
  }
  const logs = await ctx.node.call('eth_getLogs', [{
    fromBlock: hexHeight(head - logsWindow() + 1), toBlock: 'latest',
    address: token, topics: [TRANSFER_TOPIC],
  }]).catch(() => []);
  const top = sortableLogs(logs).slice(0, 25);
  const info = await ctx.token(token);
  const rows = top.map((log) => {
    const t = decodeTransfer(log);
    if (!t) return null;
    return [
      addrLink(t.from),
      addrLink(t.to),
      info ? formatTokenAmount(t.value, info.decimals) : `${t.value.toString()} (raw)`,
      blockLink(parseInt(String(log.blockNumber).replace(/^0x/, ''), 16)),
      txLink(log.transactionHash),
    ];
  }).filter(Boolean);
  return card('Recent transfers', h('p', { class: 'small muted' }, `Newest ${logsWindow().toLocaleString('en-US')} blocks (the node's eth_getLogs window).`),
    rows.length ? table(['From', 'To', 'Amount', 'Block', 'Transaction'], rows) : h('span', { class: 'muted' }, 'none in that window'));
}

// ---- not found ----

export function notFoundView(ctx, what) {
  return h('div', { class: 'stack' },
    h('h2', { class: 'page-title' }, 'Not found'),
    message('warn', what || 'No such page.'),
    h('p', {}, h('a', { href: '#/' }, '← Home')),
    sourceLine(ctx.node));
}

/** What a failed fetch looks like on any page. */
export function errorView(ctx, err) {
  return h('div', { class: 'stack' },
    h('h2', { class: 'page-title' }, 'No source answered'),
    message('error', err?.message || String(err)),
    message('plain', 'This page tries your node at 127.0.0.1:18545, then verified public peers. Install the EastSea app or allow local network access when Chrome asks. Check that this deployment includes its WASM verifier and peer seeds; Settings also accepts your own optional gateway.'),
    sourceLine(ctx.node));
}
