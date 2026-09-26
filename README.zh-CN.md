# Aether Node

[English](README.md) · [한국어](README.ko.md) · **中文** · [日本語](README.ja.md) · [Tiếng Việt](README.vi.md) · [Español](README.es.md)

> 任何一台 Mac 都可以当验证者，你的 Mac 也会自己验证你的钱包。目前，分布在不同家庭宽带上的验证者已经能通过公网路径达成共识，Mac 和 iPhone 钱包应用也运行在这个网络上。

[![Legal Disclaimer](https://img.shields.io/badge/Legal-Disclaimer%20%26%20Terms-red.svg)](DISCLAIMER.md)

> [!IMPORTANT]
> Aether Node 是实验性、非商业的研究软件，按 **"现状"（AS IS）** 提供。它不是生产级区块链，也未经过审计。所有代币（AETH）和奖励都只是测试产物，**没有任何货币价值**。详见 [DISCLAIMER.md](DISCLAIMER.md)。

## 这是什么

- **Mac 优先。**
  - 钱包密钥保存在 Secure Enclave 中，每笔支付都需要 Touch ID 确认。没有助记词。
  - 可以把另一台 Apple 设备登记为恢复密钥。
- **验证，而不是信任。**
  - 钱包在设备上核验每个余额：一个来自验证者委员会的 BLS 门限签名，加上一个 EIP-7864 状态证明。
  - 它从不直接相信服务器给出的结果。
- **不用开端口，不用 VPN。**
  - 验证者和钱包通过 BitTorrent Mainline DHT 上的节点 ID 互相发现。
  - 它们通过 iroh QUIC 连接并进行打洞，打洞失败时回退到中继。
- **也为 AI 智能体而设计。**
  - `aether-agent` 为 Claude Code、Codex、Antigravity、OpenClaw、Hermes 或任何 MCP 客户端提供一个钱包。
  - 它的密钥存放在 Secure Enclave 中，支出限额只有你本人通过 Touch ID 才能修改。

## 试用

```bash
scripts/demo.sh          # 启动 4 个验证者；演示转账、合约、证明校验以及节点达成一致
scripts/devnet.sh stop   # 停止它们
```

需要 rustup 工具链 1.98.1（`rust-toolchain.toml`）。如果 PATH 中 Homebrew 的 rustc 排在前面，请运行 `export PATH="$HOME/.cargo/bin:$PATH"`。

### 钱包应用（macOS、iOS）

```bash
scripts/build-wallet.sh           # macOS 应用
scripts/build-wallet.sh ios-sim   # iOS 模拟器
```

- **简单模式（默认）：**
  - 主屏幕显示余额和余额图表，并提供发送、接收（二维码）和获取测试代币功能。
  - 另有活动记录、网络状态和恢复设置页面。
- **开发者模式：** 证明、状态根、原始日志和区块。用左上角的开关切换模式。

### 面向 AI 智能体的钱包

```bash
scripts/build-agent.sh --install   # ~/.local/bin/aether-agent
aether-agent init                  # 由你本人执行一次：创建密钥和默认限额（Touch ID）
aether-agent setup all --apply     # 在你已安装的每个智能体工具中注册 MCP 服务器 "aether"
```

- **工具：** status、wallet、balance、send、pay_many（单笔交易）、receipt、history。
- **默认限额：** 每笔支付 1 AETH，每 24 小时 10 AETH。用 `aether-agent policy set` 修改，该命令会要求 Touch ID。
- **防篡改检查：** 如果智能体修改了策略文件，支出就会停止。支出日志经过签名，并与链上 nonce 交叉核对。
- 详情：[AGENTS.md](AGENTS.md) 和 [技能文件](agents/skills/aether-wallet/SKILL.md)。

### 运行真实网络

在每台验证者机器上生成各自的密钥。然后收集公钥部分，一起完成密钥仪式，再启动节点：

```bash
aether keygen --data ~/aether/v1                         # 在每台机器上执行；私钥只留在本机
aether network v1.pub.json v2.pub.json … > network.json  # 只包含公钥部分；分发给所有人
AETHER_NETWORK=network.json scripts/devnet.sh dkg 4      # 其他机器：aether dkg --network network.json --port … --data …
AETHER_NETWORK=network.json scripts/devnet.sh start 4    # 其他机器：aether node --network network.json …
# 钱包：把 <data>/network.json（节点 ID + 委员会公钥）复制到 apps/wallet/Resources/
```

委员会公钥在验证者变更后保持不变。使用 `aether reshare` 迁移到新的验证者集合。

### 命令行

```bash
target/debug/aether dev-accounts                               # 创世时注资的公开测试密钥（无价值）
target/debug/aether send --from-dev 1 --to 0x… --value 1000 --wait
target/debug/aether balance 0x… --rpc http://127.0.0.1:8547   # 在本地用证明验证，而不是信任
target/debug/aether batch --from-dev 1 --to 0xA,0xB --value 1  # 多笔支付，一个签名
target/debug/aether blocks 10
```

## 状态（2026-09-26）

| 领域 | 目前可用 | 尚未实现 |
|---|---|---|
| 共识 | Commonware simplex BFT：4 个验证者，1 秒出块，一个验证者宕机时仍能继续运行。BLS12-381 门限证书（131 B，用一个组公钥即可校验）。密钥在本地生成；委员会公钥来自无庄家（dealerless）DKG。通过 reshare 轮换验证者。由 VRF 播种的随机出块者 | 链上委员会变更、VRF 委员会选择 |
| 网络 | 验证者与钱包之间使用 iroh QUIC，通过打洞或中继连接。地址通过 Mainline DHT 上的节点 ID 查找。从不使用 Tailscale、CGNAT 和回环路径。已在两家不同 ISP 的 Mac 上测试 | 链上验证者列表、自有中继 |
| 执行 | revm：转账、合约部署和调用。每个验证者都会重新执行每个区块，结果必须与其区块访问列表（BAL）和 gas 一致。乐观并行执行与顺序执行结果相同 | 并行树提交 |
| 费用 | 执行和证明分别设有基础费用，按 EIP-4844 的方式调整。执行基础费用被销毁，证明费用进入证明者托管账户。小费按 60% 出块者、20% 证明者托管、20% 销毁分配 | 按已证明区块段领取托管资金 |
| 状态 | EIP-7864 二叉树（Poseidon2），键和根与 geth 参考实现一致。支持包含证明和不存在证明。每个区块原子地存入 redb，重启后从检查点恢复 | 磁盘分页、快照同步 |
| 账户 | P-256（Secure Enclave）、secp256k1 和 Ed25519。通过 EIP-7702 委托给 `AetherAccount`，可用一个签名完成批量支付。另一台设备的 Secure Enclave 密钥可作为恢复密钥 | 会话密钥、多个监护人、时间锁恢复 |
| 客户端 | Mac 和 iOS 钱包（简单模式和开发者模式）、CLI 以及 `aether-agent`（MCP）都在本地验证余额 | 客户端内的 ZK 区块证明、TestFlight |
| 抗审查 | FOCIL 风格的包含列表：如果区块遗漏了列表中的交易，验证者拒绝为其投票 | 加密内存池 |
| 证明（spike） | Jolt zkVM 可以证明真实的 Aether 区块，根与原生执行结果一致。每台 Mac 每小时约 270 笔交易。证明落后于链 | Metal 后端、检查点证明 |

`legacy/` 是早期的单节点演示，已被取代。

## 设计

- [实现设计](docs/design/00-overview.md)：身份、决策 D1–D18 以及各层的设计
- [研究](docs/research/)：每项决策背后的资料来源，包括 [tokenomics 2026](docs/research/tokenomics-2026.md)
- [Spike 结果](docs/research/spike-2026-10.md)

## 测试

```bash
cargo test --workspace
cargo test -p aether-state --test eip7864_compat   # 与 EIP-7864 参考实现交叉核对
cargo test -p aether-execution --test fees         # 费用分配与价值守恒
```

## 目录结构

```
crates/
├── node/        # aether 二进制：验证者（simplex + marshal）、JSON-RPC、CLI、DKG
├── execution/   # revm 执行、交易校验、BAL、费用、收据、证明 gas
├── state/       # EIP-7864 二叉树与证明
├── types/       # 信封、区块、BAL、证书、证明
├── light/       # 轻客户端：委员会公钥、证书校验
├── net/         # iroh 连接、Mainline DHT 发现
├── ffi/         # 供 Swift 使用的钱包核心（UniFFI）
├── hash/ crypto/ consensus/ proving/ da/
apps/
├── wallet/      # macOS 和 iOS 钱包（SwiftUI）
└── agent/       # aether-agent：面向 AI 智能体的 MCP 服务器和 JSON CLI
agents/skills/   # 供智能体工具使用的 SKILL.md
contracts/       # AetherAccount（EIP-7702 批量 + 恢复）
spike/           # zkVM 证明实验
scripts/         # devnet.sh、demo.sh、build-wallet.sh、build-agent.sh
docs/design, docs/research
```

## 许可证

采用 MIT 或 Apache-2.0 双重许可。
