# Agents

AI agents (Claude Code, Codex, Antigravity, OpenClaw, Hermes, or any MCP client) pay and check balances on Aether through `aether-agent`.
- **Key:** in the Mac's Secure Enclave. It cannot be exported.
- **Limits:** the owner sets them and signs them with Touch ID.

```bash
scripts/build-agent.sh --install   # builds ~/.local/bin/aether-agent
aether-agent init                  # owner, once: creates the keys and default limits (Touch ID)
aether-agent setup all --apply     # registers the MCP server "aether" with every installed agent tool
```

- Tools and rules: [agents/skills/aether-wallet/SKILL.md](agents/skills/aether-wallet/SKILL.md)
- Source: [apps/agent](apps/agent)
