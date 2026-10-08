import SwiftUI

/// Everything this wallet holds: the native coin (verified on this device) and the ERC-20 tokens
/// with a balance (the node's answer, labeled as such). Tokens this wallet moved by
/// its own signed action are listed; what only arrived by someone else's transfer
/// waits collapsed under "Unverified", out of any total
/// (docs/research/token-spam-2026.md §6).
struct AssetsSheet: View {
    @EnvironmentObject var model: WalletModel
    @Environment(\.dismiss) private var dismiss
    /// Open the Send sheet with this token already chosen (tap a row).
    var onSend: ((TokenHolding) -> Void)?
    @State private var unverifiedOpen = false

    private var balance: Double? { model.account.flatMap { Double(Wei.format($0.balanceWei)) } }
    private var sections: (main: [TokenHolding], unverified: [TokenHolding]) { model.tokenSections }

    var body: some View {
        VStack(alignment: .leading, spacing: DesignTokens.Space.s5) {
            HStack(spacing: DesignTokens.Space.s2) {
                EastSeaDawnMark().frame(width: 24, height: 24)
                Text("Assets").font(.aeTitle)
            }
            nativeBalance
            mainTokens
            if !sections.unverified.isEmpty {
                Rectangle().fill(DesignTokens.Palette.line.color).frame(height: 1)
                unverifiedTokens
            }
            HStack {
                TimelineView(.periodic(from: .now, by: 30)) { tl in
                    Text(updated(now: tl.date)).font(.aeCaption).foregroundStyle(DesignTokens.Palette.textMuted.color)
                }
                Spacer()
                Button("Done") { dismiss() }.buttonStyle(EastSeaQuietButtonStyle()).keyboardShortcut(.cancelAction)
            }
        }
        .padding(DesignTokens.Space.s6)
        .macMinSize(width: 420)
        .sheetScroll()
        .eastSeaSheet()
        .onAppear { model.refreshTokens(force: true) }
    }

    private var nativeBalance: some View {
        VStack(alignment: .leading, spacing: DesignTokens.Space.s3) {
            HStack(spacing: DesignTokens.Space.s2) {
                TokenIcon(chainId: Brand.networkChainId, address: nil, symbol: Brand.networkCoinTicker, size: 28)
                Text(Brand.networkCoinName).font(.aeHeadline)
                Spacer(minLength: 0)
                if model.account != nil && model.verifyError == nil {
                    Label("Verified", systemImage: "checkmark.shield.fill")
                        .font(.aeCaption).foregroundStyle(DesignTokens.Palette.plateSoft.color)
                }
            }
            HStack(alignment: .firstTextBaseline, spacing: DesignTokens.Space.s2) {
                Text(balance.map { Amount.text($0) } ?? "—")
                    .font(DesignTokens.TypeScale.amountMd.font).lineLimit(1).minimumScaleFactor(0.6)
                Text(Brand.networkCoinTicker).font(.aeFootnote)
                    .foregroundStyle(DesignTokens.Palette.plateSoft.color)
            }
            if let since = model.chainPausedSince { NetworkPausedBadge(since: since, onPlate: true) }
        }
        .padding(DesignTokens.Space.s5)
        .frame(maxWidth: .infinity, alignment: .leading)
        .foregroundStyle(DesignTokens.Palette.plateInk.color)
        .eastSeaNavyPlate(cornerRadius: DesignTokens.Radius.lg)
    }

    @ViewBuilder private var mainTokens: some View {
        VStack(alignment: .leading, spacing: DesignTokens.Space.s3) {
            VStack(alignment: .leading, spacing: DesignTokens.Space.s1) {
                Text("Tokens").font(.aeHeadline)
                Text("Read from the node · not verified on this device").font(.aeCaption)
                    .foregroundStyle(DesignTokens.Palette.textMuted.color)
                    .fixedSize(horizontal: false, vertical: true)
            }
            rows(sections.main, unverified: false)
            if sections.main.isEmpty && sections.unverified.isEmpty {
                if model.tokensUpdated == nil {
                    HStack(spacing: 8) {
                        ProgressView().controlSize(.small)
                        Text("Looking for tokens…").font(.aeBody).foregroundStyle(DesignTokens.Palette.textMuted.color)
                    }
                } else if model.tokensError != nil {
                    Text("Could not read tokens from the node. It tries again shortly.").font(.aeBody).foregroundStyle(DesignTokens.Palette.textMuted.color)
                } else {
                    Text("No other tokens in this wallet.").font(.aeBody).foregroundStyle(DesignTokens.Palette.textMuted.color)
                }
            }
        }
    }

    /// Someone sent these in; nothing this wallet signed ever touched them, so
    /// they start collapsed and out of any total. A tap still opens a send —
    /// with the address always in sight.
    private var unverifiedTokens: some View {
        VStack(alignment: .leading, spacing: DesignTokens.Space.s3) {
            DisclosureGroup(String(localized: "Unverified (\(sections.unverified.count))"), isExpanded: $unverifiedOpen) {
                VStack(alignment: .leading, spacing: 0) {
                    Text("Someone sent these to you. Nothing you signed ever touched them — anyone can create a token, so check the contract address before trusting one.")
                        .font(.aeFootnote).foregroundStyle(DesignTokens.Palette.textMuted.color).fixedSize(horizontal: false, vertical: true)
                        .padding(.bottom, DesignTokens.Space.s3)
                    rows(sections.unverified, unverified: true)
                }
            }
            .font(.aeHeadline)
        }
    }

    @ViewBuilder private func rows(_ holdings: [TokenHolding], unverified: Bool) -> some View {
        // The divider between sections comes from the caller; no trailing one.
        ForEach(Array(holdings.enumerated()), id: \.element.id) { i, t in
            TokenHoldingRow(holding: t, official: model.officialSymbols, chainId: model.status?.chainId ?? 0, onSend: onSend.map { send in { send(t) } })
                .contextMenu {
                    Button("Copy Token Address") { Clipboard.copy(t.token.address) }
                    Divider()
                    if unverified {
                        Button("Show in main list") { model.showTokenInMainList(t.token.address) }
                    } else {
                        Button("Hide from main list") { model.setTokenHidden(t.token.address, true) }
                    }
                }
            if i < holdings.count - 1 { Rectangle().fill(DesignTokens.Palette.line.color).frame(height: 1) }
        }
    }

    private func updated(now: Date) -> String {
        guard let at = model.tokensUpdated else { return "" }
        return now.timeIntervalSince(at) < 45 ? String(localized: "Tokens updated just now")
            : String(localized: "Tokens updated \(RelativeDateTimeFormatter().localizedString(for: at, relativeTo: now))")
    }
}

private struct TokenHoldingRow: View {
    let holding: TokenHolding
    let official: [(symbol: String, name: String)]
    /// The chain the trust list is keyed by; 0 keeps everything unverified.
    var chainId: UInt64 = 0
    var onSend: (() -> Void)?

    var body: some View {
        Button(action: { onSend?() }) {
            HStack(alignment: .top, spacing: DesignTokens.Space.s3) {
                TokenIcon(chainId: chainId, address: holding.token.address, symbol: holding.token.symbol, size: 40)
                VStack(alignment: .leading, spacing: DesignTokens.Space.s1) {
                    Text(holding.token.name.isEmpty ? holding.token.symbol
                         : KnownTokens.displayName(chainId: chainId, address: holding.token.address, name: holding.token.name)).font(.aeBody.weight(.semibold)).lineLimit(1)
                    // Never the symbol alone: anyone can deploy another "USDT".
                    Text(TokenLabel.row(holding.token)).font(.aeCaption.monospaced()).foregroundStyle(DesignTokens.Palette.textMuted.color)
                    TokenBadges(holding: holding, official: official, chainId: chainId)
                }
                Spacer(minLength: 8)
                Text("\(holding.amount) \(holding.token.symbol)").font(.aeBody.weight(.semibold).monospacedDigit())
                    .lineLimit(1).minimumScaleFactor(0.7)
            }
            .padding(.vertical, DesignTokens.Space.s2)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }
}

/// The warnings a token carries with it: created on the launchpad (anyone can),
/// a symbol/name mimicking an official token, or — audit R2-2 — units the wallet
/// does not vouch for. No prices, no returns.
struct TokenBadges: View {
    let holding: TokenHolding
    let official: [(symbol: String, name: String)]
    /// The chain the shipped trust list is keyed by; 0 keeps everything unverified.
    var chainId: UInt64 = 0

    var body: some View {
        if isLaunchpad || lookAlike || nodeDisagrees || unverifiedUnits {
            VStack(alignment: .leading, spacing: DesignTokens.Space.s1) {
                if isLaunchpad {
                    Label("Launchpad · unverified", systemImage: "exclamationmark.bubble.fill")
                        .font(.aeCaption.weight(.semibold)).foregroundStyle(Color.warn)
                }
                if lookAlike {
                    Label("Mimics an official token", systemImage: "exclamationmark.shield.fill")
                        .font(.aeCaption.weight(.semibold)).foregroundStyle(Color.warn)
                }
                if nodeDisagrees {
                    Label("Node disagrees · units from the shipped list", systemImage: "arrow.triangle.2.circlepath")
                        .font(.aeCaption.weight(.semibold)).foregroundStyle(DesignTokens.Palette.textMuted.color)
                }
                if unverifiedUnits {
                    Label("Unverified units", systemImage: "questionmark.circle")
                        .font(.aeCaption.weight(.semibold)).foregroundStyle(DesignTokens.Palette.textMuted.color)
                }
            }
            .fixedSize(horizontal: false, vertical: true)
        }
    }

    private var isLaunchpad: Bool { holding.token.origin == "launchpad" }
    private var lookAlike: Bool {
        SendSafety.looksLikeOfficial(symbol: holding.token.symbol, name: holding.token.name, official: official)
    }
    private var denomination: TokenDenomination {
        TokenDenomination.of(chainId: chainId, address: holding.token.address, claimed: holding.token)
    }
    private var nodeDisagrees: Bool { denomination.nodeDisagrees }
    private var unverifiedUnits: Bool { denomination.unverifiedUnits }
}
