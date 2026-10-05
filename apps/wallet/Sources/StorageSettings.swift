#if os(macOS)
import SwiftUI

/// 설정 ▸ 역사 보관 (docs/design/15-node-rewards.md "C. 보관"): how much of
/// the network's past this Mac keeps. Consumer words only — gigabytes, never
/// "shards" or "eras". The choice maps to the node's `--max-shards`
/// (`StorageSetting`) on its next start; both the app's node and the
/// unattended daemon read the same resolved value. Rewards are phase 2, a
/// later protocol upgrade: the copy says "once switched on" and promises no
/// amount.
struct HistoryStorageSection: View {
    @EnvironmentObject var node: NodeController
    /// The data volume's free space, read when Settings appears — the guard
    /// and the "남는 공간 사용" label stand on it.
    @State private var freeBytes: Int?

    /// A registered candidate has no 보관 안 함 (docs/design/15: 등록 후보만
    /// 보관) — the row is hidden and a stored "off" runs as the default.
    private var registered: Bool { node.voting?.registered ?? false }

    private var choice: Binding<String> {
        Binding(get: { registered && node.historyStorage == "off" ? StorageSetting.defaultChoice : node.historyStorage },
                set: { node.historyStorage = $0 })
    }

    private func gb(_ bytes: Int?) -> String {
        bytes.map { "\($0 / 1_073_741_824) GB" } ?? "—"
    }

    var body: some View {
        Section("역사 보관") {
            Picker("보관 용량", selection: choice) {
                if !registered {
                    Text("보관 안 함").tag("off")
                }
                ForEach(StorageSetting.choicesGB, id: \.self) { gb in
                    Text(gb == 50 ? "50 GB (기본)" : "\(gb) GB")
                        .tag(String(gb))
                        .disabled(!StorageSetting.allows(gb: gb, freeBytes: freeBytes ?? 0,
                                                         heldBytes: Int(node.history?.bytes ?? 0)))
                }
                Text("남는 공간 사용" + (freeBytes.map(StorageSetting.freeSpaceReserve(freeBytes:)).map { " (\(gb($0)) 남김)" } ?? ""))
                    .tag("free")
            }
            .help("Your Mac keeps a share of the network's past so anyone can check it. Keeping more, for longer, earns more once storage rewards are switched on.")
            Text("이 Mac이 네트워크의 과거를 나눠 보관해서 누구나 검증할 수 있게 해요. 더 많이, 더 오래 보관할수록 보관 보상이 켜진 뒤 더 많이 받아요.")
                .font(.caption).foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            Text("보관 보상은 아직 켜져 있지 않아요. 나중에 있을 프로토콜 업그레이드(2단계)로 켜지며, 약속된 금액은 없어요.")
                .font(.caption).foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            if let freeBytes {
                Text("데이터 볼륨 남은 공간: \(gb(freeBytes))")
                    .font(.caption).foregroundStyle(.secondary)
            }
            if refusedCurrent {
                Label("이 크기는 남은 공간보다 커요. 20 GB은 남겨 두려고 해요 — 더 작은 크기를 골라 주세요.",
                      systemImage: "exclamationmark.triangle")
                    .font(.caption).foregroundStyle(.orange)
            }
            Text("바꾼 용량은 노드가 다음에 시작될 때 적용돼요. 보관하던 데이터는 사라지지 않고, 줄이면 배정을 넘는 만큼만 지워요.")
                .font(.caption).foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            if let kept = node.history {
                if kept.shards == 0 {
                    Text("아직 보관 중인 역사가 없어요")
                        .font(.caption).foregroundStyle(.secondary)
                } else {
                    let rate = kept.passPercent.map { " · 최근 \(kept.windowDays)일 확인 통과 \($0)%" } ?? ""
                    Text("지금 \(gb(Int(min(kept.bytes, UInt64(Int.max)))))의 역사를 보관 중\(rate)")
                        .font(.caption).foregroundStyle(.secondary)
                }
            }
        }
        .onAppear {
            freeBytes = StorageSetting.freeBytes(atPath: NodeController.dataDir.path)
        }
    }

    /// The stored choice no longer fits the volume as it is right now (it was
    /// chosen when there was more room, or the disk filled since).
    private var refusedCurrent: Bool {
        guard let gb = Int(node.historyStorage), StorageSetting.choicesGB.contains(gb) else { return false }
        return !StorageSetting.allows(gb: gb, freeBytes: freeBytes ?? 0, heldBytes: Int(node.history?.bytes ?? 0))
    }
}
#endif
