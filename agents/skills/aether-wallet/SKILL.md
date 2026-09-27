---
name: aether-wallet
description: Pay and check balances on the Aether network from this Mac. Use when the user asks the agent to send AETH, pay several people, check a balance, confirm a payment, or see what the agent has spent. The key sits in the Secure Enclave, and the account contract enforces the limits the owner set with Touch ID.
---

# Aether wallet for agents

This agent has its own Aether account on this Mac.
- **Key:** created in the Mac's Secure Enclave. It cannot be exported or copied, even by the agent.
- **Balances:** checked on this Mac against the validators' threshold signature, not taken from a server.
- **Limits:** the account contract enforces them on chain: a per-payment limit, a 24-hour limit, and optionally a list of allowed recipients and an expiry. Only the owner can change them, with Touch ID. Nothing the agent runs can get past them.
- **Gas:** paid from a separate small balance, so gas costs are capped too.

## When to Use

- The user asks to pay, tip, refund, or split a payment in AETH.
- The user asks for a balance (the agent's or any address), or whether a payment went through.
- Before a paid action, to check that the budget allows it.

## How It Works

Prefer the MCP tools (server `aether`). Without MCP, run the same commands with `aether-agent`, which prints JSON.

| Goal | MCP tool | CLI |
|---|---|---|
| Network, fee | `aether_status` | `aether-agent status` |
| Own address, balance, how much the limits still allow now (24-hour window) | `aether_wallet` | `aether-agent wallet` |
| Any address's balance | `aether_balance` | `aether-agent balance --address 0x…` |
| Check a payment without sending | `aether_send` with `dry_run: true` | `aether-agent send --to 0x… --amount 1 --dry-run` |
| Pay one recipient | `aether_send` | `aether-agent send --to 0x… --amount 1` |
| Pay several in one transaction | `aether_pay_many` | `aether-agent pay-many --to 0xA,0xB --amount 0.1` |
| Did it go through? | `aether_receipt` | `aether-agent receipt --hash 0x…` |
| What the agent spent | `aether_history` | `aether-agent history` |
| Test tokens (testnet only) | `aether_get_test_tokens` | `aether-agent get-test-tokens` |

Rules:
1. Amounts are decimal AETH strings (`"0.25"`), never wei.
2. If you are unsure, do a dry run first and tell the user the amount and the limit left.
3. If a payment is **refused by policy**, do not split it or retry around the limit. Tell the user the limit and that they can change it with `aether-agent policy set --per-tx X --per-day Y`, which asks for their Touch ID.
4. A payment is done only when the result says `"final": true` and `"success": true`.
5. Never ask for or handle a seed phrase or private key. There is none to handle.

## Examples

- "Send 0.5 AETH to 0x1234…": call `aether_send {to, amount: "0.5"}`, then report the block and hash.
- "Pay Alice and Bob 1 AETH each": call `aether_pay_many {payments: [{to: A, amount: "1"}, {to: B, amount: "1"}]}`. It is one transaction: both succeed or neither does.
- "How much can you still spend today?": call `aether_wallet` and read `left_now_aeth`.
