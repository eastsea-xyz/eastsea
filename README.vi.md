# EastSea Node

[English](README.md) · [한국어](README.ko.md) · [中文](README.zh-CN.md) · [日本語](README.ja.md) · **Tiếng Việt** · [Español](README.es.md)

> Máy Mac nào cũng có thể làm validator, và chính máy Mac của bạn tự xác minh ví của bạn. Hiện tại, các validator trên những đường internet gia đình khác nhau đã đạt đồng thuận qua đường truyền công cộng, và ứng dụng ví trên Mac và iPhone chạy trên chính mạng đó.

[![Legal Disclaimer](https://img.shields.io/badge/Legal-Disclaimer%20%26%20Terms-red.svg)](DISCLAIMER.md)

> [!IMPORTANT]
> EastSea Node là phần mềm nghiên cứu thử nghiệm, phi thương mại, được cung cấp **"NGUYÊN TRẠNG" (AS IS)**. Đây không phải là blockchain dùng cho môi trường production và chưa được kiểm toán. Mọi token (AETH) và phần thưởng chỉ là sản phẩm thử nghiệm, **không có bất kỳ giá trị tiền tệ nào**. Xem [DISCLAIMER.md](DISCLAIMER.md).

## Đây là gì

- **Ưu tiên Mac.**
  - Khóa ví nằm trong Secure Enclave và mỗi lần thanh toán đều yêu cầu Touch ID. Không có cụm từ khôi phục (seed phrase).
  - Có thể đăng ký một thiết bị Apple thứ hai làm khóa khôi phục.
- **Xác minh, không tin tưởng.**
  - Ví kiểm tra từng số dư ngay trên thiết bị: một chữ ký ngưỡng BLS từ ủy ban validator, cộng với một bằng chứng trạng thái EIP-7864.
  - Ví không bao giờ tin lời máy chủ.
- **Không cần mở cổng, không cần VPN.**
  - Validator và ví tìm thấy nhau bằng node ID trên BitTorrent Mainline DHT.
  - Chúng kết nối qua iroh QUIC với hole punching, và chuyển sang relay khi không thành công.
- **Cũng được xây dựng cho AI agent.**
  - `aether-agent` cung cấp ví cho Claude Code, Codex, Antigravity, OpenClaw, Hermes hoặc bất kỳ MCP client nào.
  - Khóa của nó nằm trong Secure Enclave, và hợp đồng tài khoản thực thi hạn mức chi tiêu ngay trên chuỗi; chỉ bạn mới thay đổi được chúng, bằng Touch ID.

## Dùng thử

```bash
scripts/demo.sh          # khởi động 4 validator; trình diễn chuyển tiền, hợp đồng, kiểm tra bằng chứng, các node đồng thuận
scripts/devnet.sh stop   # dừng chúng
```

Cần toolchain rustup 1.98.1 (`rust-toolchain.toml`). Nếu rustc của Homebrew đứng trước trong PATH, hãy chạy `export PATH="$HOME/.cargo/bin:$PATH"`.

### Ứng dụng ví (macOS, iOS)

```bash
scripts/build-wallet.sh           # ứng dụng macOS
scripts/build-wallet.sh ios-sim   # iOS Simulator
```

- **Chế độ Đơn giản (mặc định):**
  - Màn hình chính hiển thị số dư và biểu đồ số dư, cùng các chức năng Gửi, Nhận (QR) và Nhận token thử nghiệm.
  - Các trang cho hoạt động, trạng thái mạng và thiết lập khôi phục.
- **Chế độ Nhà phát triển:** bằng chứng, state root, log thô và block. Dùng công tắc ở góc trên bên trái để chuyển chế độ.

### Ví cho AI agent

```bash
scripts/build-agent.sh --install   # ~/.local/bin/aether-agent
aether-agent init                  # bạn làm một lần: tạo khóa; nạp tiền vào tài khoản, rồi đặt hạn mức (Touch ID)
aether-agent setup all --apply     # đăng ký MCP server "aether" với mọi công cụ agent bạn đã cài
```

- **Công cụ:** status, wallet, balance, send, pay_many (một giao dịch), receipt, history.
- **Hạn mức mặc định:** 1 AETH mỗi lần thanh toán và 10 AETH mỗi 24 giờ. Thay đổi bằng `aether-agent policy set`, lệnh này yêu cầu Touch ID.
- **Thực thi trên chuỗi:** agent thanh toán bằng một session key của tài khoản của nó; hợp đồng kiểm tra mọi khoản thanh toán theo hạn mức, danh sách người nhận được phép và thời hạn hiệu lực. Không tệp hay tiến trình cục bộ nào có thể vượt qua được. Phí gas được lấy từ một khoản số dư nhỏ riêng.
- Chi tiết: [AGENTS.md](AGENTS.md) và [tệp skill](agents/skills/aether-wallet/SKILL.md).

### Chạy một mạng thật

Trên mỗi máy validator, tạo khóa riêng của máy đó. Sau đó thu thập các phần công khai, cùng nhau thực hiện nghi thức tạo khóa (key ceremony), rồi khởi động các node:

```bash
aether keygen --data ~/aether/v1                         # trên mỗi máy; khóa bí mật ở lại máy đó
aether network v1.pub.json v2.pub.json … > network.json  # chỉ gồm phần công khai; gửi cho mọi người
AETHER_NETWORK=network.json scripts/devnet.sh dkg 4      # các máy khác: aether dkg --network network.json --port … --data …
AETHER_NETWORK=network.json scripts/devnet.sh start 4    # các máy khác: aether node --network network.json …
# ví: sao chép <data>/network.json (node ID + khóa ủy ban) vào apps/wallet/Resources/
```

Khóa ủy ban vẫn giữ nguyên khi thay đổi validator. Dùng `aether reshare` để chuyển sang một tập validator mới.

### Dòng lệnh

```bash
target/debug/aether dev-accounts                               # khóa thử nghiệm công khai được cấp tiền ở genesis (không có giá trị)
target/debug/aether send --from-dev 1 --to 0x… --value 1000 --wait
target/debug/aether balance 0x… --rpc http://127.0.0.1:8547   # xác minh cục bộ bằng bằng chứng, không tin tưởng
target/debug/aether batch --from-dev 1 --to 0xA,0xB --value 1  # nhiều khoản thanh toán, một chữ ký
target/debug/aether blocks 10
```

## Trạng thái (2026-09-26)

| Lĩnh vực | Đã hoạt động | Chưa có |
|---|---|---|
| Đồng thuận | Commonware simplex BFT: 4 validator, block 1 giây, vẫn chạy tiếp khi một validator ngừng hoạt động. Chứng chỉ ngưỡng BLS12-381 (131 B, kiểm tra bằng một khóa nhóm). Khóa được tạo cục bộ; khóa ủy ban từ DKG không cần dealer. Luân chuyển validator bằng reshare. Leader ngẫu nhiên với seed từ VRF | Thay đổi ủy ban on-chain, chọn ủy ban bằng VRF |
| Mạng | iroh QUIC giữa validator và ví, qua hole punching hoặc relay. Địa chỉ được tìm bằng node ID trên Mainline DHT. Không bao giờ dùng đường Tailscale, CGNAT hay loopback. Đã thử nghiệm với các máy Mac trên hai ISP khác nhau | Danh sách validator on-chain, relay riêng |
| Thực thi | revm: chuyển tiền, triển khai và gọi hợp đồng. Mỗi validator thực thi lại từng block và kết quả phải khớp với block access list (BAL) và gas của block đó. Thực thi song song lạc quan cho kết quả giống thực thi tuần tự | Commit cây song song |
| Phí | Phí cơ sở riêng cho thực thi và chứng minh, điều chỉnh theo cách của EIP-4844. Phí cơ sở thực thi bị đốt, còn phí chứng minh được chuyển vào quỹ ký quỹ cho prover. Tiền tip chia 60% cho proposer, 20% vào quỹ ký quỹ prover, 20% bị đốt | Rút tiền ký quỹ theo từng đoạn đã được chứng minh |
| Trạng thái | Cây nhị phân EIP-7864 (Poseidon2), với khóa và root khớp với bản tham chiếu geth. Bằng chứng tồn tại và bằng chứng không tồn tại. Lưu nguyên tử theo từng block trong redb, và tiếp tục từ checkpoint sau khi khởi động lại | Phân trang đĩa, đồng bộ snapshot |
| Tài khoản | P-256 (Secure Enclave), secp256k1 và Ed25519. Ủy quyền EIP-7702 cho `AetherAccount` cho phép thanh toán theo lô với một chữ ký. Khóa Secure Enclave của thiết bị thứ hai có thể làm khóa khôi phục | Session key, nhiều người giám hộ, khôi phục có khóa thời gian |
| Client | Ví Mac và iOS (chế độ Đơn giản và Nhà phát triển), CLI và `aether-agent` (MCP) đều xác minh số dư cục bộ | Bằng chứng ZK cho block trong client, TestFlight |
| Chống kiểm duyệt | Danh sách bao gồm kiểu FOCIL: validator từ chối bỏ phiếu cho block bỏ sót các giao dịch có trong danh sách | Mempool mã hóa |
| Chứng minh (spike) | Jolt zkVM chứng minh được các block EastSea thật, và root khớp với kết quả thực thi gốc. Khoảng 270 giao dịch mỗi giờ trên mỗi máy Mac. Bằng chứng đi sau chuỗi | Backend Metal, bằng chứng checkpoint |

`legacy/` là bản demo một node trước đây và đã được thay thế.

## Thiết kế

- [Thiết kế triển khai](docs/design/00-overview.md): định danh, các quyết định D1–D18 và thiết kế của từng tầng
- [Nghiên cứu](docs/research/): nguồn tài liệu đằng sau từng quyết định, bao gồm [tokenomics 2026](docs/research/tokenomics-2026.md)
- [Kết quả spike](docs/research/spike-2026-10.md)

## Kiểm thử

```bash
cargo test --workspace
cargo test -p aether-state --test eip7864_compat   # đối chiếu với bản tham chiếu EIP-7864
cargo test -p aether-execution --test fees         # chia phí và bảo toàn giá trị
```

## Cấu trúc thư mục

```
crates/
├── node/        # binary aether: validator (simplex + marshal), JSON-RPC, CLI, DKG
├── execution/   # thực thi revm, kiểm tra tx, BAL, phí, receipt, gas chứng minh
├── state/       # cây nhị phân EIP-7864 và bằng chứng
├── types/       # envelope, block, BAL, chứng chỉ, bằng chứng
├── light/       # light client: khóa ủy ban, kiểm tra chứng chỉ
├── net/         # kết nối iroh, khám phá qua Mainline DHT
├── ffi/         # lõi ví cho Swift (UniFFI)
├── hash/ crypto/ consensus/ proving/ da/
apps/
├── wallet/      # ví macOS và iOS (SwiftUI)
└── agent/       # aether-agent: MCP server và JSON CLI cho AI agent
agents/skills/   # SKILL.md cho các công cụ agent
contracts/       # AetherAccount (EIP-7702 batch + khôi phục)
spike/           # thử nghiệm chứng minh bằng zkVM
scripts/         # devnet.sh, demo.sh, build-wallet.sh, build-agent.sh
docs/design, docs/research
```

## Giấy phép

Cấp phép kép theo MIT hoặc Apache-2.0.
