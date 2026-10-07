#if os(macOS)
import SwiftUI

/// The node's one stop reason, where a person looks (the Node page, the
/// menu-bar panel): what happened, when it resumes, one button. Never
/// truncated — it wraps.
struct NodeStopRow: View {
    @EnvironmentObject var node: NodeController
    let reason: NodeStopReason
    var compact = false

    var body: some View {
        let c = reason.copy(ko: HealthCheck.korean)
        VStack(alignment: .leading, spacing: 6) {
            Label(c.title, systemImage: reason.isIncident ? "exclamationmark.triangle.fill" : "pause.circle")
                .font(compact ? .aeCaption.weight(.semibold) : .aeBody.weight(.semibold))
                .foregroundStyle(reason.isIncident ? Color.warn : Color.secondary)
                .fixedSize(horizontal: false, vertical: true)
            Text(c.paragraph)
                .font(compact ? .aeCaption : .aeFootnote).foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            if let action = c.action, let label = c.actionLabel {
                Button(label) { node.perform(action) }
                    .controlSize(compact ? .small : .regular)
            }
        }
    }
}

/// The daemon waits for "Allow in the Background": the node keeps running
/// in the app; one sentence and the button to the Login Items pane.
struct UnattendedApprovalLine: View {
    @EnvironmentObject var unattended: UnattendedDaemon
    var compact = false

    var body: some View {
        if let sentence = unattended.approvalSentence {
            VStack(alignment: .leading, spacing: 6) {
                Text(sentence).font(compact ? .aeCaption : .aeFootnote).foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                if unattended.status == .needsApproval, !unattended.blockDataOnExternalDisk {
                    Button(HealthCheck.korean ? "로그인 항목 열기" : "Open Login Items") { unattended.openApprovalPane() }
                        .controlSize(.small)
                }
            }
        }
    }
}

/// 블록 데이터 위치 + 전체 기록 보관 (아카이브), on the Node page.
struct BlockDataSection: View {
    @EnvironmentObject var node: NodeController
    @State private var showArchiveInfo = false
    @State private var confirmArchiveOff = false

    private var ko: Bool { HealthCheck.korean }

    private var placeLine: String {
        if node.chainDataPath.isEmpty {
            return ko ? "기본 위치 (이 Mac의 내장 디스크)" : "Default (this Mac's internal disk)"
        }
        return node.chainDataPath
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text(ko ? "블록 데이터 위치" : "Block data location").font(.aeHeadline)
            Text(placeLine).font(.aeFootnote.monospaced()).foregroundStyle(.secondary)
                .textSelection(.enabled)
                .fixedSize(horizontal: false, vertical: true)
            Text(ko ? "블록 데이터만 옮겨요. 키와 노드 신원은 이 Mac에 그대로 남아요. APFS 또는 Mac OS 확장 형식의 디스크만 쓸 수 있어요."
                 : "Only the block data moves; the keys and the node's identity stay on this Mac. APFS or Mac OS Extended disks only.")
                .font(.aeFootnote).foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            if let p = node.storageMovePercent {
                ProgressView(value: Double(p), total: 100) {
                    Text(ko ? "옮기고 확인하는 중 · \(p)%" : "Copying and checking · \(p)%").font(.aeFootnote)
                }
            } else {
                HStack {
                    Button(ko ? "다른 위치 선택…" : "Choose Location…") { node.chooseBlockDataLocation() }
                    if !node.chainDataPath.isEmpty {
                        Button(ko ? "기본 위치로 되돌리기" : "Move Back to Default") { node.moveBlockData(to: nil) }
                    }
                }
            }
            if let error = node.storageMoveError {
                Label(error, systemImage: "exclamationmark.triangle").font(.aeFootnote).foregroundStyle(Color.warn)
                    .fixedSize(horizontal: false, vertical: true)
            }
            Divider()
            Toggle(isOn: Binding(get: { node.archive }, set: { on in
                if on { showArchiveInfo = true } else { confirmArchiveOff = true }
            })) {
                Text(ko ? "전체 기록 보관 (아카이브)" : "Keep full history (archive)").font(.aeBody)
            }
            .toggleStyle(.switch)
            Text(ko ? "꺼 두면 최근 상태만 받아 빠르게 따라가요. 켜면 처음 블록부터 전부 다시 확인하고 모든 기록을 보관해요."
                 : "Off, the node fetches the recent state and catches up fast. On, it re-checks every block from the first and keeps the whole history.")
                .font(.aeFootnote).foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
        }
        .onChange(of: node.chooseDiskRequested) { _, asked in
            if asked { node.chooseDiskRequested = false; node.chooseBlockDataLocation() }
        }
        .sheet(isPresented: $showArchiveInfo) {
            ArchiveRequirementsSheet(height: node.height) { node.archive = true }
        }
        .confirmationDialog(ko ? "전체 기록 보관을 끌까요?" : "Turn off full history?", isPresented: $confirmArchiveOff) {
            Button(ko ? "끄고 보관한 기록은 남겨 두기" : "Turn Off, Keep the History") { node.turnArchiveOff(deleteHistory: false) }
            Button(ko ? "끄고 보관한 기록 지우기" : "Turn Off and Delete the History", role: .destructive) { node.turnArchiveOff(deleteHistory: true) }
            Button(ko ? "취소" : "Cancel", role: .cancel) {}
        } message: {
            Text(ko ? "일반 노드로 돌아가요. 보관한 기록을 남겨 두면 다시 켤 때 이어서 써요."
                 : "The node goes back to normal. Keep the history and turning it on again picks up where it left off.")
        }
    }
}

/// The honest requirements before archive mode goes on.
struct ArchiveRequirementsSheet: View {
    let height: UInt64
    let enable: () -> Void
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        let ko = HealthCheck.korean
        VStack(alignment: .leading, spacing: 12) {
            Text(ko ? "전체 기록 보관을 켤까요?" : "Keep the full history?").font(.aeHeadline)
            ForEach(ArchiveRequirements(height: height).lines(ko: ko), id: \.self) { line in
                Label(line, systemImage: "circle.fill").labelStyle(BulletLabel())
                    .font(.aeBody).fixedSize(horizontal: false, vertical: true)
            }
            HStack {
                Spacer()
                Button(ko ? "취소" : "Cancel") { dismiss() }
                Button(ko ? "켜기" : "Turn On") { enable(); dismiss() }.buttonStyle(.borderedProminent)
            }
        }
        .padding(20)
        .frame(width: 440)
    }
}

private struct BulletLabel: LabelStyle {
    func makeBody(configuration: Configuration) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            Text("•")
            configuration.title
        }
    }
}
#endif
