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
        SettingsSection(LocalizedStringKey("Network history"), explanation: LocalizedStringKey("Keep a share of the network's past.")) {
            SettingsControlRow(LocalizedStringKey("Keep up to")) {
                Picker("Keep up to", selection: choice) {
                    if !registered {
                        Text("Nothing").tag("off")
                    }
                    ForEach(StorageSetting.choicesGB, id: \.self) { gb in
                        Text(gb == 50 ? String(localized: "50 GB (default)") : "\(gb) GB")
                            .tag(String(gb))
                            .disabled(!StorageSetting.allows(gb: gb, freeBytes: freeBytes ?? 0,
                                                             heldBytes: Int(node.history?.bytes ?? 0)))
                    }
                    Text(freeBytes.map(StorageSetting.freeSpaceReserve(freeBytes:)).map { String(localized: "All free space (keeps \(gb($0)) free)") }
                         ?? String(localized: "All free space"))
                        .tag("free")
                }
            }
            Label("Storage rewards are not on yet.", systemImage: "clock")
                .font(.aeFootnote).foregroundStyle(DesignTokens.Palette.textMuted.color)
            if let freeBytes {
                Text("Free space on this disk: \(gb(freeBytes))")
                    .font(.aeFootnote).monospacedDigit().foregroundStyle(DesignTokens.Palette.textMuted.color)
            }
            if refusedCurrent {
                Label("This is more than the free space allows: 20 GB always stays free. Pick a smaller size.",
                      systemImage: "exclamationmark.triangle")
                    .font(.aeFootnote).foregroundStyle(Color.warn)
                    .fixedSize(horizontal: false, vertical: true)
            }
            if let kept = node.history {
                if kept.shards == 0 {
                    Text("No network history kept yet")
                        .font(.aeFootnote).foregroundStyle(DesignTokens.Palette.textMuted.color)
                } else {
                    let size = gb(Int(min(kept.bytes, UInt64(Int.max))))
                    Text("Keeping \(size) of network history now")
                        .font(.aeFootnote).monospacedDigit().foregroundStyle(DesignTokens.Palette.textMuted.color)
                    if let pct = kept.passPercent {
                        Text("\(pct)% of checks passed in the last \(kept.windowDays) days")
                            .font(.aeFootnote).monospacedDigit().foregroundStyle(DesignTokens.Palette.textMuted.color)
                    }
                }
            }
            SettingsLearnMore {
                Text("Your Mac keeps a share of the network's past so anyone can check it. Keeping more, for longer, earns more once storage rewards are switched on.")
                Text("Storage rewards are not on yet. A later network upgrade turns them on, and no amount is promised.")
                Text("A new size applies the next time the node starts. Nothing kept is lost; a smaller size deletes only what goes over it.")
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
