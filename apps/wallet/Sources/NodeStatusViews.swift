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
        let c = reason.copy()
        VStack(alignment: .leading, spacing: DesignTokens.Space.s2) {
            Label(c.title, systemImage: reason.isIncident ? "exclamationmark.triangle.fill" : "pause.circle")
                .font(compact ? .aeCaption.weight(.semibold) : .aeBody.weight(.semibold))
                .foregroundStyle(reason.isIncident ? Color.warn : DesignTokens.Palette.textMuted.color)
                .fixedSize(horizontal: false, vertical: true)
            Text(c.paragraph)
                .font(compact ? .aeCaption : .aeFootnote).foregroundStyle(DesignTokens.Palette.textMuted.color)
                .fixedSize(horizontal: false, vertical: true)
            if let action = c.action, let label = c.actionLabel {
                Button(label) { node.perform(action) }
                    .controlSize(compact ? .small : .regular)
                    .buttonStyle(EastSeaQuietButtonStyle())
                    .disabled(node.keyRebindInProgress)
            }
            if reason == .keyElsewhere, let error = node.keyRebindError {
                Text(error).font(compact ? .aeCaption : .aeFootnote).foregroundStyle(Color.warn)
                    .fixedSize(horizontal: false, vertical: true)
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
            VStack(alignment: .leading, spacing: DesignTokens.Space.s2) {
                Text(sentence).font(compact ? .aeCaption : .aeFootnote).foregroundStyle(DesignTokens.Palette.textMuted.color)
                    .fixedSize(horizontal: false, vertical: true)
                if unattended.status == .needsApproval, !unattended.blockDataOnExternalDisk {
                    Button(String(localized: "Open Login Items")) { unattended.openApprovalPane() }
                        .controlSize(.small)
                        .buttonStyle(EastSeaQuietButtonStyle())
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


    private var placeLine: String {
        if node.chainDataPath.isEmpty {
            return String(localized: "Default (this Mac's internal disk)")
        }
        return node.chainDataPath
    }

    var body: some View {
        VStack(alignment: .leading, spacing: DesignTokens.Space.s3) {
            Text(String(localized: "Block data location")).font(.aeHeadline)
            Text("Keys and identity stay on this Mac.")
                .font(.aeFootnote).foregroundStyle(DesignTokens.Palette.textMuted.color)
            Text(placeLine).font(.aeFootnote.monospaced()).foregroundStyle(DesignTokens.Palette.text.color)
                .textSelection(.enabled)
                .fixedSize(horizontal: false, vertical: true)
            if let p = node.storageMovePercent {
                ProgressView(value: Double(p), total: 100) {
                    Text(String(localized: "Copying and checking · \(p)%")).font(.aeFootnote)
                }
                .tint(DesignTokens.Palette.accent.color)
            } else {
                ViewThatFits(in: .horizontal) {
                    HStack(spacing: DesignTokens.Space.s2) { locationActions }
                    VStack(alignment: .leading, spacing: DesignTokens.Space.s2) { locationActions }
                }
                .buttonStyle(EastSeaQuietButtonStyle())
            }
            if let error = node.storageMoveError {
                Label(error, systemImage: "exclamationmark.triangle").font(.aeFootnote).foregroundStyle(Color.warn)
                    .fixedSize(horizontal: false, vertical: true)
                if node.storageMoveOffersDiskUtility {
                    Button(String(localized: "Open Disk Utility")) { node.openDiskUtility() }
                        .controlSize(.small)
                        .buttonStyle(EastSeaQuietButtonStyle())
                }
            }
            SettingsLearnMore {
                Text(String(localized: "Only the block data moves; the keys and the node's identity stay on this Mac. APFS or Mac OS Extended disks only."))
            }
            Divider().overlay(DesignTokens.Palette.line.color)
            SettingsControlRow(LocalizedStringKey("Keep full history (archive)")) {
                Toggle(isOn: Binding(get: { node.archive }, set: { on in
                    if on { showArchiveInfo = true } else { confirmArchiveOff = true }
                })) {
                    Text(String(localized: "Keep full history (archive)")).font(.aeBody)
                }
                .toggleStyle(.switch)
            }
            Text("Re-check every block and keep the full history.")
                .font(.aeFootnote).foregroundStyle(DesignTokens.Palette.textMuted.color)
            SettingsLearnMore {
                Text(String(localized: "Off, the node fetches the recent state and catches up fast. On, it re-checks every block from the first and keeps the whole history."))
            }
        }
        .foregroundStyle(DesignTokens.Palette.text.color)
        .tint(DesignTokens.Palette.accent.color)
        .onChange(of: node.chooseDiskRequested) { _, asked in
            if asked { node.chooseDiskRequested = false; node.chooseBlockDataLocation() }
        }
        .sheet(isPresented: $showArchiveInfo) {
            ArchiveRequirementsSheet(height: node.height) { node.archive = true }
        }
        .confirmationDialog(String(localized: "Turn off full history?"), isPresented: $confirmArchiveOff) {
            Button(String(localized: "Turn Off, Keep the History")) { node.turnArchiveOff(deleteHistory: false) }
            Button(String(localized: "Turn Off and Delete the History"), role: .destructive) { node.turnArchiveOff(deleteHistory: true) }
            Button(String(localized: "Cancel"), role: .cancel) {}
        } message: {
            Text(String(localized: "The node goes back to normal. Keep the history and turning it on again picks up where it left off."))
        }
    }

    @ViewBuilder private var locationActions: some View {
        Button(String(localized: "Choose Location…")) { node.chooseBlockDataLocation() }
        if !node.chainDataPath.isEmpty {
            Button(String(localized: "Move Back to Default")) { node.moveBlockData(to: nil) }
        }
    }
}

/// The honest requirements before archive mode goes on.
struct ArchiveRequirementsSheet: View {
    let height: UInt64
    let enable: () -> Void
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        VStack(alignment: .leading, spacing: DesignTokens.Space.s6) {
            HStack(spacing: DesignTokens.Space.s3) {
                Image(systemName: "externaldrive").font(.system(size: 24))
                    .foregroundStyle(DesignTokens.Palette.accent.color).accessibilityHidden(true)
                Text(String(localized: "Keep the full history?")).font(.aeTitle)
            }
            VStack(alignment: .leading, spacing: DesignTokens.Space.s4) {
                ForEach(ArchiveRequirements(height: height).lines(), id: \.self) { line in
                    Label(line, systemImage: "circle.fill").labelStyle(BulletLabel())
                        .font(.aeBody).fixedSize(horizontal: false, vertical: true)
                }
            }
            HStack(spacing: DesignTokens.Space.s2) {
                Spacer()
                Button(String(localized: "Cancel")) { dismiss() }
                    .buttonStyle(EastSeaQuietButtonStyle())
                Button(String(localized: "Turn On")) { enable(); dismiss() }
                    .buttonStyle(EastSeaPrimaryButtonStyle())
            }
        }
        .padding(DesignTokens.Space.s6)
        .frame(width: 440)
        .eastSeaSheet()
    }
}

private struct BulletLabel: LabelStyle {
    func makeBody(configuration: Configuration) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: DesignTokens.Space.s3) {
            Image(systemName: "circle.fill").font(.system(size: 4))
                .foregroundStyle(DesignTokens.Palette.textMuted.color).accessibilityHidden(true)
            configuration.title
        }
    }
}
#endif
