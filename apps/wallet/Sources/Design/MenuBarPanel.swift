#if canImport(SwiftUI)
import SwiftUI

extension EastSeaDesign {
    struct MenuBarLabels {
        let brandName: String
        let balance: String
        let node: String
        let receive: String
        let receiveHint: String
        let copyAddress: String
        let openWallet: String
        let qrAccessibility: String
        let receiveUnavailable: String
    }

    struct MenuBarSnapshot {
        let accountIdentity: String
        let accountName: String
        let balance: Decimal?
        let balanceAccessibility: String
        let currency: String
        let verificationText: String
        let isVerified: Bool
        let nodeStatus: EastSeaNodeStatus
        let nodeText: String
        let address: String?
        var verificationHelp: String = ""
        var statusEvent: String? = nil
        var rewardEvent: String? = nil
        var successEvent: String? = nil
    }

    /// Localized truth and actions come from the wallet. Typed content slots
    /// retain its account picker and diagnostics without a second visual panel.
    @available(macOS 14.0, iOS 17.0, *)
    struct MenuBarPanel<AccountControl: View, NodeDetails: View, ReceiveDetails: View, Footer: View>: View {
        typealias Labels = MenuBarLabels
        typealias Snapshot = MenuBarSnapshot

        let snapshot: Snapshot
        let labels: Labels
        @Binding var nodeEnabled: Bool
        let formatBalance: (Decimal) -> String
        let onCopyAddress: (String) -> Void
        let onOpenWallet: () -> Void
        let hapticsEnabled: Bool
        private let accountControl: AccountControl
        private let nodeDetails: NodeDetails
        private let receiveDetails: ReceiveDetails
        private let footer: Footer
        @Environment(\.accessibilityReduceMotion) private var reduceMotion

        init(snapshot: Snapshot, labels: Labels, nodeEnabled: Binding<Bool>,
             formatBalance: @escaping (Decimal) -> String,
             onCopyAddress: @escaping (String) -> Void, onOpenWallet: @escaping () -> Void,
             hapticsEnabled: Bool = true,
             @ViewBuilder accountControl: () -> AccountControl,
             @ViewBuilder nodeDetails: () -> NodeDetails,
             @ViewBuilder receiveDetails: () -> ReceiveDetails,
             @ViewBuilder footer: () -> Footer) {
            self.snapshot = snapshot
            self.labels = labels
            _nodeEnabled = nodeEnabled
            self.formatBalance = formatBalance
            self.onCopyAddress = onCopyAddress
            self.onOpenWallet = onOpenWallet
            self.hapticsEnabled = hapticsEnabled
            self.accountControl = accountControl()
            self.nodeDetails = nodeDetails()
            self.receiveDetails = receiveDetails()
            self.footer = footer()
        }

        init(snapshot: Snapshot, labels: Labels, nodeEnabled: Binding<Bool>,
             formatBalance: @escaping (Decimal) -> String,
             onCopyAddress: @escaping (String) -> Void, onOpenWallet: @escaping () -> Void,
             hapticsEnabled: Bool = true)
        where AccountControl == Text, NodeDetails == EmptyView, ReceiveDetails == EmptyView, Footer == EmptyView {
            self.init(snapshot: snapshot, labels: labels, nodeEnabled: nodeEnabled,
                      formatBalance: formatBalance, onCopyAddress: onCopyAddress, onOpenWallet: onOpenWallet,
                      hapticsEnabled: hapticsEnabled,
                      accountControl: { Text(verbatim: snapshot.accountName) },
                      nodeDetails: { EmptyView() }, receiveDetails: { EmptyView() }, footer: { EmptyView() })
        }

        var body: some View {
            panelBody.id(snapshot.accountIdentity)
        }

        private var panelBody: some View {
            VStack(alignment: .leading, spacing: 0) {
                header.padding(.bottom, DesignTokens.Space.s3)
                balancePlate
                nodeRow.padding(.vertical, DesignTokens.Space.s5)
                nodeDetails
                Rectangle().fill(DesignTokens.Palette.line.color).frame(height: 1)
                receiveRow.padding(.vertical, DesignTokens.Space.s5)
                receiveDetails
                openButton
                footer
            }
            .padding(DesignTokens.Space.s4)
            .frame(width: 336)
            .background(DesignTokens.Palette.surface.color)
            .foregroundStyle(DesignTokens.Palette.text.color)
            .clipShape(RoundedRectangle(cornerRadius: DesignTokens.Radius.lg, style: .continuous))
            .eastSeaSuccessFeedback(event: snapshot.successEvent, enabled: hapticsEnabled)
            .transaction { transaction in
                if reduceMotion {
                    transaction.animation = nil
                    transaction.disablesAnimations = true
                }
            }
        }

        private var header: some View {
            HStack(spacing: DesignTokens.Space.s2) {
                EastSeaDawnMark().frame(width: 24, height: 24)
                Text(verbatim: labels.brandName).font(DesignTokens.TypeScale.headline.font.weight(.semibold))
                    .fixedSize()
                accountControl
                    .font(DesignTokens.TypeScale.caption.font)
                    .foregroundStyle(DesignTokens.Palette.textMuted.color)
                    .frame(maxWidth: .infinity, alignment: .trailing)
            }
            .frame(minHeight: 24)
        }

        private var balancePlate: some View {
            VStack(alignment: .leading, spacing: DesignTokens.Space.s2) {
                Text(verbatim: labels.balance).font(DesignTokens.TypeScale.caption.font)
                    .foregroundStyle(DesignTokens.Palette.plateSoft.color)
                HStack(alignment: .firstTextBaseline, spacing: DesignTokens.Space.s2) {
                    if let balance = snapshot.balance, !balance.isNaN {
                        BalanceCountUp(amount: balance, accessibilityText: snapshot.balanceAccessibility, format: formatBalance)
                            .font(DesignTokens.TypeScale.amountMd.font)
                            .tracking(DesignTokens.TypeScale.amountMd.tracking * DesignTokens.TypeScale.amountMd.size)
                            .lineLimit(1).minimumScaleFactor(0.45)
                            .layoutPriority(1)
                    } else {
                        Text(verbatim: "—").font(DesignTokens.TypeScale.amountMd.font)
                            .accessibilityLabel(snapshot.balanceAccessibility)
                    }
                    Text(verbatim: snapshot.currency).font(DesignTokens.TypeScale.headline.font)
                        .foregroundStyle(DesignTokens.Palette.plateSoft.color)
                        .fixedSize()
                }
                HStack(spacing: DesignTokens.Space.s1) {
                    Image(systemName: snapshot.isVerified ? "checkmark" : "clock")
                    Text(verbatim: snapshot.verificationText).fixedSize(horizontal: false, vertical: true)
                }
                .font(DesignTokens.TypeScale.caption.font)
                .foregroundStyle(DesignTokens.Palette.plateSoft.color)
                .help(snapshot.verificationHelp)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(DesignTokens.Space.s4)
            .frame(minHeight: 128, alignment: .leading)
            .foregroundStyle(DesignTokens.Palette.plateInk.color)
            .eastSeaNavyPlate(cornerRadius: DesignTokens.Radius.md)
            .dblnRewardShine(arrival: snapshot.rewardEvent, cornerRadius: DesignTokens.Radius.md)
        }

        private var nodeRow: some View {
            HStack(spacing: DesignTokens.Space.s3) {
                VStack(alignment: .leading, spacing: DesignTokens.Space.s1) {
                    Text(verbatim: labels.node).font(DesignTokens.TypeScale.headline.font)
                    HStack(spacing: DesignTokens.Space.s1) {
                        NodeStatusPulse(status: snapshot.nodeStatus, event: snapshot.statusEvent, accessibilityLabel: snapshot.nodeText)
                            .accessibilityHidden(true)
                        Text(verbatim: snapshot.nodeText)
                            .font(DesignTokens.TypeScale.caption.font)
                            .foregroundStyle(DesignTokens.Palette.textMuted.color)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
                Spacer(minLength: 0)
                Toggle(isOn: $nodeEnabled) { Text(verbatim: labels.node) }
                    .labelsHidden().toggleStyle(.switch).tint(DesignTokens.Palette.success.color)
            }
        }

        private var receiveRow: some View {
            HStack(alignment: .center, spacing: DesignTokens.Space.s3) {
                VStack(alignment: .leading, spacing: DesignTokens.Space.s2) {
                    Text(verbatim: labels.receive).font(DesignTokens.TypeScale.headline.font)
                    Text(verbatim: labels.receiveHint).font(DesignTokens.TypeScale.caption.font)
                        .foregroundStyle(DesignTokens.Palette.textMuted.color)
                        .fixedSize(horizontal: false, vertical: true)
                    if let address = snapshot.address, !address.isEmpty {
                        HStack(spacing: DesignTokens.Space.s2) {
                            Text(verbatim: address.count > 14 ? "\(address.prefix(6))…\(address.suffix(4))" : address)
                                .font(.system(size: DesignTokens.TypeScale.caption.size, design: .monospaced))
                                .textSelection(.enabled)
                            Button { onCopyAddress(address) } label: { Image(systemName: "doc.on.doc") }
                                .buttonStyle(.plain)
                                .frame(minWidth: 24, minHeight: 24)
                                .accessibilityLabel(labels.copyAddress)
                        }
                    } else {
                        Text(verbatim: labels.receiveUnavailable).font(DesignTokens.TypeScale.caption.font)
                    }
                }
                Spacer(minLength: 0)
                if let address = snapshot.address, !address.isEmpty {
                    EastSeaReceiveCode(address: address, accessibilityText: labels.qrAccessibility)
                        .frame(width: 104, height: 104)
                        .clipShape(RoundedRectangle(cornerRadius: DesignTokens.Radius.sm))
                }
            }
        }

        private var openButton: some View {
            Button(action: onOpenWallet) {
                HStack(spacing: DesignTokens.Space.s2) {
                    Text(verbatim: labels.openWallet)
                    Image(systemName: "arrow.right")
                }
                .font(DesignTokens.TypeScale.bodyUi.font.weight(.semibold))
                .frame(maxWidth: .infinity, minHeight: DesignTokens.Space.s10)
                .foregroundStyle(DesignTokens.Palette.onAccentFill.color)
                .background(DesignTokens.Palette.accentFill.color, in: Capsule())
            }
            .buttonStyle(.plain)
            .accessibilityLabel(labels.openWallet)
        }
    }
}
#endif
