# Agents

AI agents (Claude Code, Codex, Antigravity, OpenClaw, Hermes, or any MCP client) pay and check balances on Aether through `aether-agent`.
- **Key:** in the Mac's Secure Enclave. It cannot be exported.
- **Limits:** the owner sets payment caps, recipients, and expiry with Touch ID; the account contract enforces them on chain. A tricked agent can still spend within those limits. New agents cannot pay until the owner approves a payee.
- **DEX:** agents can read the Aether DEX (`dex_pools`, `dex_token_info`, `dex_quote`). These tools sign nothing, and agents cannot swap yet.

```bash
scripts/build-agent.sh --install   # builds ~/.local/bin/aether-agent
aether-agent init                  # owner, once: creates keys; payments remain off
aether-agent payee add --name Shop --address 0x...  # owner approves a recipient with Touch ID
aether-agent stop                  # owner revokes the session with Touch ID
aether-agent setup all --apply     # registers the MCP server "aether" with every installed agent tool
```

- Tools and rules: [agents/skills/aether-wallet/SKILL.md](agents/skills/aether-wallet/SKILL.md)
- Source: [apps/agent](apps/agent)
