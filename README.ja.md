# Aether Node

[English](README.md) · [한국어](README.ko.md) · [中文](README.zh-CN.md) · **日本語** · [Tiếng Việt](README.vi.md) · [Español](README.es.md)

> どの Mac でもバリデータになれ、自分のウォレットは自分の Mac が検証します。現在、異なる家庭用インターネット回線上のバリデータが公開経路を通じて合意に達しており、Mac と iPhone のウォレットアプリはこのネットワーク上で動作します。

[![Legal Disclaimer](https://img.shields.io/badge/Legal-Disclaimer%20%26%20Terms-red.svg)](DISCLAIMER.md)

> [!IMPORTANT]
> Aether Node は **「現状のまま（AS IS）」** 提供される、実験的かつ非商用の研究用ソフトウェアです。本番用のブロックチェーンではなく、監査も受けていません。すべてのトークン（AETH）と報酬はテスト用の成果物であり、**金銭的価値は一切ありません**。[DISCLAIMER.md](DISCLAIMER.md) を参照してください。

## 概要

- **Mac ファースト。**
  - ウォレットの鍵は Secure Enclave に保管され、支払いのたびに Touch ID を求めます。シードフレーズはありません。
  - 2 台目の Apple デバイスをリカバリーキーとして登録できます。
- **信頼せず、検証する。**
  - ウォレットは各残高をデバイス上で確認します。バリデータ委員会による BLS 閾値署名 1 つと、EIP-7864 の状態証明を使います。
  - サーバーの言うことをそのまま信じることはありません。
- **ポート開放も VPN も不要。**
  - バリデータとウォレットは BitTorrent Mainline DHT 上でノード ID によって互いを見つけます。
  - iroh QUIC でホールパンチングにより接続し、失敗した場合はリレーにフォールバックします。
- **AI エージェントにも対応。**
  - `aether-agent` は Claude Code、Codex、Antigravity、OpenClaw、Hermes、その他任意の MCP クライアントにウォレットを提供します。
  - 鍵は Secure Enclave にあり、支出上限は Touch ID を使ってあなただけが変更できます。

## 試してみる

```bash
scripts/demo.sh          # バリデータを 4 つ起動し、送金、コントラクト、証明チェック、ノード間の合意を表示する
scripts/devnet.sh stop   # 停止する
```

rustup ツールチェーン 1.98.1（`rust-toolchain.toml`）が必要です。PATH 上で Homebrew の rustc が先に見つかる場合は、`export PATH="$HOME/.cargo/bin:$PATH"` を実行してください。

### ウォレットアプリ（macOS、iOS）

```bash
scripts/build-wallet.sh           # macOS アプリ
scripts/build-wallet.sh ios-sim   # iOS シミュレータ
```

- **Simple モード（デフォルト）：**
  - 残高と残高チャート、および送金、受け取り（QR）、テストトークン取得を備えたホーム画面。
  - アクティビティ、ネットワーク状態、リカバリー設定のページ。
- **Developer モード：** 証明、state root、生のログとブロック。左上のスイッチでモードを切り替えます。

### AI エージェント向けウォレット

```bash
scripts/build-agent.sh --install   # ~/.local/bin/aether-agent
aether-agent init                  # あなたが一度だけ実行：鍵とデフォルトの上限を作成する（Touch ID）
aether-agent setup all --apply     # インストール済みのすべてのエージェントツールに MCP サーバー "aether" を登録する
```

- **ツール：** status、wallet、balance、send、pay_many（1 トランザクション）、receipt、history。
- **デフォルトの上限：** 1 回の支払いにつき 1 AETH、24 時間あたり 10 AETH。`aether-agent policy set` で変更でき、その際に Touch ID を求められます。
- **改ざんチェック：** エージェントがポリシーファイルを編集すると、支出は停止します。支出ログは署名されており、オンチェーンの nonce と突き合わせて検証されます。
- 詳細：[AGENTS.md](AGENTS.md) および [skill ファイル](agents/skills/aether-wallet/SKILL.md)。

### 実際のネットワークを動かす

各バリデータマシンで、それぞれ自分の鍵を生成します。次に公開部分を集め、全員で鍵セレモニーを実施してから、ノードを起動します。

```bash
aether keygen --data ~/aether/v1                         # 各マシンで実行。秘密鍵はそのマシンから出ない
aether network v1.pub.json v2.pub.json … > network.json  # 公開部分のみ。全員に配布する
AETHER_NETWORK=network.json scripts/devnet.sh dkg 4      # 他のマシン：aether dkg --network network.json --port … --data …
AETHER_NETWORK=network.json scripts/devnet.sh start 4    # 他のマシン：aether node --network network.json …
# ウォレット：<data>/network.json（ノード ID + 委員会鍵）を apps/wallet/Resources/ にコピーする
```

委員会鍵はバリデータが入れ替わっても維持されます。新しいバリデータセットに移行するには `aether reshare` を使います。

### コマンドライン

```bash
target/debug/aether dev-accounts                               # ジェネシスで資金が割り当てられた公開テスト鍵（価値なし）
target/debug/aether send --from-dev 1 --to 0x… --value 1000 --wait
target/debug/aether balance 0x… --rpc http://127.0.0.1:8547   # 信頼せず、証明を使ってローカルで検証する
target/debug/aether batch --from-dev 1 --to 0xA,0xB --value 1  # 複数の支払いを 1 つの署名で行う
target/debug/aether blocks 10
```

## ステータス（2026-09-26）

| 領域 | 現在動作するもの | 未対応 |
|---|---|---|
| コンセンサス | Commonware simplex BFT：バリデータ 4 つ、1 秒ブロック、バリデータが 1 つ停止しても継続。BLS12-381 閾値証明書（131 B、1 つのグループ鍵で検証）。鍵はローカルで生成し、委員会鍵はディーラーレス DKG で生成。reshare によるバリデータのローテーション。VRF シードによるランダムなリーダー選出 | オンチェーンでの委員会変更、VRF による委員会選出 |
| ネットワーク | バリデータとウォレット間の iroh QUIC（ホールパンチングまたはリレー）。Mainline DHT 上でノード ID によりアドレスを発見。Tailscale、CGNAT、ループバックの経路は使用しない。異なる 2 つの ISP 上の Mac でテスト済み | オンチェーンのバリデータ一覧、独自リレー |
| 実行 | revm：送金、コントラクトのデプロイと呼び出し。すべてのバリデータが各ブロックを再実行し、ブロックアクセスリスト（BAL）と gas に一致しなければならない。楽観的並列実行は逐次実行と同じ結果を出す | 並列ツリーコミット |
| 手数料 | 実行と証明に別々の base fee を設け、EIP-4844 と同様に調整。実行の base fee はバーンされ、証明手数料は prover escrow に送られる。チップは proposer 60%、prover escrow 20%、バーン 20% に分配 | 証明済みチャンク単位の escrow 請求 |
| 状態 | EIP-7864 バイナリツリー（Poseidon2）。鍵と root は geth のリファレンスと一致。包含証明と不在証明。ブロックごとに redb へアトミックに保存され、再起動後はチェックポイントから再開 | ディスクページング、スナップショット同期 |
| アカウント | P-256（Secure Enclave）、secp256k1、Ed25519。`AetherAccount` への EIP-7702 委任により、1 つの署名で一括支払いが可能。2 台目のデバイスの Secure Enclave 鍵をリカバリーキーとして使用可能 | セッションキー、複数ガーディアン、時間ロック付きリカバリー |
| クライアント | Mac と iOS のウォレット（Simple モードと Developer モード）、CLI、`aether-agent`（MCP）のすべてが残高をローカルで検証 | クライアント内での ZK ブロック証明、TestFlight |
| 検閲耐性 | FOCIL 方式のインクルージョンリスト：リスト上のトランザクションを含まないブロックには、バリデータが投票を拒否する | 暗号化 mempool |
| 証明（spike） | Jolt zkVM が実際の Aether ブロックを証明し、root はネイティブ実行と一致。Mac 1 台あたり毎時約 270 トランザクション。証明はチェーンより遅れて追従する | Metal バックエンド、チェックポイント証明 |

`legacy/` は以前の単一ノードのデモで、すでに置き換えられています。

## 設計

- [実装設計](docs/design/00-overview.md)：アイデンティティ、決定事項 D1–D18、各レイヤーの設計
- [リサーチ](docs/research/)：各決定の根拠となる資料。[tokenomics 2026](docs/research/tokenomics-2026.md) を含む
- [Spike の結果](docs/research/spike-2026-10.md)

## テスト

```bash
cargo test --workspace
cargo test -p aether-state --test eip7864_compat   # EIP-7864 のリファレンスと突き合わせて検証する
cargo test -p aether-execution --test fees         # 手数料の分配と価値の保存を確認する
```

## 構成

```
crates/
├── node/        # aether binary: validator (simplex + marshal), JSON-RPC, CLI, DKG
├── execution/   # revm execution, tx validation, BAL, fees, receipts, prove gas
├── state/       # EIP-7864 binary tree and proofs
├── types/       # envelopes, blocks, BAL, certificates, proofs
├── light/       # light client: committee key, certificate checks
├── net/         # iroh links, Mainline DHT discovery
├── ffi/         # wallet core for Swift (UniFFI)
├── hash/ crypto/ consensus/ proving/ da/
apps/
├── wallet/      # macOS and iOS wallet (SwiftUI)
└── agent/       # aether-agent: MCP server and JSON CLI for AI agents
agents/skills/   # SKILL.md for agent tools
contracts/       # AetherAccount (EIP-7702 batch + recovery)
spike/           # zkVM proving experiments
scripts/         # devnet.sh, demo.sh, build-wallet.sh, build-agent.sh
docs/design, docs/research
```

## ライセンス

MIT または Apache-2.0 のデュアルライセンスです。
