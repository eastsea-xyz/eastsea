---
name: aether-wallet
description: Pay and check balances on the Aether network from this Mac. Use when the user asks the agent to send AETH, pay several people, check a balance, confirm a payment, or see what the agent has spent, or asks about Aether DEX pools, tokens or swap prices (read-only quotes). The key sits in the Secure Enclave, and the account contract enforces the limits the owner set with Touch ID.
---

# Aether wallet for agents

This agent has its own Aether account on this Mac.
- **Key:** created in the Mac's Secure Enclave. It cannot be exported or copied, even by the agent.
- **Balances:** checked on this Mac against the validators' threshold signature, not taken from a server.
- **Limits:** the account contract checks per-payment and 24-hour caps, approved recipients, and session expiry. New sessions require an owner-approved payee and expire after seven days by default. The owner may explicitly allow any recipient. A tricked agent can still spend within these limits; Secure Enclave does not verify the agent's intent.
- **Gas:** paid from a separate small balance, so gas costs are capped too.

## When to Use

- The user asks to pay, tip, refund, or split a payment in AETH.
- The user asks for a balance (the agent's or any address), or whether a payment went through.
- Before a paid action, to check that the budget allows it.
- The user asks about the Aether DEX: which pools exist, a token's details, or what a swap would return.

## How It Works

Prefer the MCP tools (server `aether`). Without MCP, run the same commands with `aether-agent`, which prints JSON.

| Goal | MCP tool | CLI |
|---|---|---|
| Network, fee | `aether_status` | `aether-agent status` |
| Own address, balance, how much the limits still allow now (24-hour window) | `aether_wallet` | `aether-agent wallet` |
| Any address's balance | `aether_balance` | `aether-agent balance --address 0x…` |
| Check a payment without sending | `aether_send` with `dry_run: true` | `aether-agent send --to 0x… --amount 1 --purpose "reason" --dry-run` |
| Pay one recipient | `aether_send` | `aether-agent send --to 0x… --amount 1 --purpose "reason"` |
| Pay several in one transaction | `aether_pay_many` | `aether-agent pay-many --to 0xA,0xB --amount 0.1 --purpose "reason"` |
| Pay an owner-listed ERC-20 (new genesis) | `aether_pay_token` | `aether-agent pay-token --token 0x… --to 0x… --amount 1 --purpose "reason"` |
| Did it go through? | `aether_receipt` | `aether-agent receipt --hash 0x…` |
| What the agent spent | `aether_history` | `aether-agent history` |
| Test tokens (testnet only) | `aether_get_test_tokens` | `aether-agent get-test-tokens` |
| DEX pools: tokens, reserves, prices | `dex_pools` | `aether-agent dex-pools` |
| A DEX token: symbol, name, decimals, supply, the agent's balance | `dex_token_info {token}` | `aether-agent dex-token-info --token NEB` |
| What a swap would return (route, price impact, minimum received) | `dex_quote {from, to, amount}` | `aether-agent dex-quote --from AETH --to NEB --amount 1` |

Rules:
1. Amounts are decimal AETH strings (`"0.25"`), never wei.
2. If you are unsure, do a dry run first and tell the user the amount and the limit left.
3. If a payment is **refused by policy**, do not split it or retry around the limit. Tell the user the limit and that they can change it with `aether-agent policy set --per-tx X --per-day Y`, which asks for their Touch ID.
4. A payment is done only when the result says `"final": true` and `"success": true`. If it is not final, the result says `"status"`: `"pending"` (still waiting; `why` says for what) or `"dropped"` (not included and nothing was paid; `why` says why). Tell the user `why_ko` or `why`. Do not resend a dropped payment on your own; ask the user first.
5. Every payment needs a truthful `purpose`. An unapproved payee is refused and appears as an approval request in the Mac wallet; only the owner may add it with Touch ID. Do not retry or route around the refusal.
6. The owner can run `aether-agent stop` or use **비서 멈추기** in the Mac wallet to revoke the session. This does not reverse already submitted transactions.
   Token policies also need renewed owner approval after any session replacement (`policy set`, `policy renew`, or `payee add`).
7. Never ask for or handle a seed phrase or private key. There is none to handle.
8. The `dex_*` tools only read; they sign nothing. A quote is an estimate from the pools' current reserves, and the real result can differ. Say so when you give one. Agents cannot swap yet: if the user wants to trade, tell them to use the Aether DEX app.
9. DEX tokens are named by symbol or 0x address. `AETH` is the native coin (pools hold it as WAETH, 1:1). If a symbol is ambiguous, the tool lists the addresses; ask the user which one.

## Examples

- "Send 0.5 AETH to 0x1234…": call `aether_send {to, amount: "0.5", purpose: "the owner's requested payment"}`, then report the block and hash.
- "Pay Alice and Bob 1 AETH each": call `aether_pay_many {payments: [{to: A, amount: "1"}, {to: B, amount: "1"}], purpose: "the owner's requested split"}`. It is one transaction: both succeed or neither does.
- "How much can you still spend today?": call `aether_wallet` and read `left_now_aeth`.
- "How much NEB would 1 AETH get me?": call `dex_quote {from: "AETH", to: "NEB", amount: "1"}`, then report `expected_out`, `minimum_received` and `price_impact_percent`, and say it is an estimate.
