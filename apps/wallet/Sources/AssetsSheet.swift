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
        VStack(alignment: .leading, spacing: 16) {
            Text("Assets").font(.aeTitle)
            TokenRow(symbol: Brand.networkCoinTicker, name: Brand.networkCoinName, amount: balance, verified: model.account != nil && model.verifyError == nil)
            if let since = model.chainPausedSince { NetworkPausedBadge(since: since) }
            Divider()
            mainTokens
            if !sections.unverified.isEmpty { Divider(); unverifiedTokens }
            HStack {
                TimelineView(.periodic(from: .now, by: 30)) { tl in
                    Text(updated(now: tl.date)).font(.aeCaption).foregroundStyle(.secondary)
                }
                Spacer()
                Button("Done") { dismiss() }.keyboardShortcut(.cancelAction)
            }
        }
        .padding(24)
        .macMinSize(width: 420)
        .sheetScroll()
        .onAppear { model.refreshTokens(force: true) }
    }

    @ViewBuilder private var mainTokens: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(alignment: .firstTextBaseline) {
                Text("Tokens").font(.aeHeadline)
                Spacer()
                Text("Read from the node · not verified on this device").font(.aeCaption).foregroundStyle(.secondary)
                    .multilineTextAlignment(.trailing)
            }
            rows(sections.main, unverified: false)
            if sections.main.isEmpty && sections.unverified.isEmpty {
                if model.tokensUpdated == nil {
                    HStack(spacing: 8) {
                        ProgressView().controlSize(.small)
                        Text("Looking for tokens…").font(.aeBody).foregroundStyle(.secondary)
                    }
                } else if model.tokensError != nil {
                    Text("Could not read tokens from the node. It tries again shortly.").font(.aeBody).foregroundStyle(.secondary)
                } else {
                    Text("No other tokens in this wallet.").font(.aeBody).foregroundStyle(.secondary)
                }
            }
        }
    }

    /// Someone sent these in; nothing this wallet signed ever touched them, so
    /// they start collapsed and out of any total. A tap still opens a send —
    /// with the address always in sight.
    private var unverifiedTokens: some View {
        VStack(alignment: .leading, spacing: 12) {
            DisclosureGroup(String(localized: "Unverified (\(sections.unverified.count))"), isExpanded: $unverifiedOpen) {
                VStack(alignment: .leading, spacing: 0) {
                    Text("Someone sent these to you. Nothing you signed ever touched them — anyone can create a token, so check the contract address before trusting one.")
                        .font(.aeFootnote).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
                        .padding(.bottom, 10)
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
            if i < holdings.count - 1 { Divider() }
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
            HStack(spacing: 12) {
                TokenIcon(chainId: chainId, address: holding.token.address, symbol: holding.token.symbol, size: 40)
                VStack(alignment: .leading, spacing: 2) {
                    Text(holding.token.name.isEmpty ? holding.token.symbol
                         : KnownTokens.displayName(chainId: chainId, address: holding.token.address, name: holding.token.name)).font(.aeBody.weight(.semibold)).lineLimit(1)
                    // Never the symbol alone: anyone can deploy another "USDT".
                    Text(TokenLabel.row(holding.token)).font(.aeCaption.monospaced()).foregroundStyle(.secondary)
                    TokenBadges(holding: holding, official: official, chainId: chainId)
                }
                Spacer(minLength: 8)
                Text("\(holding.amount) \(holding.token.symbol)").font(.aeBody.weight(.semibold).monospacedDigit())
                    .lineLimit(1).minimumScaleFactor(0.7)
            }
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
            HStack(spacing: 10) {
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
                        .font(.aeCaption.weight(.semibold)).foregroundStyle(.secondary)
                }
                if unverifiedUnits {
                    Label("Unverified units", systemImage: "questionmark.circle")
                        .font(.aeCaption.weight(.semibold)).foregroundStyle(.secondary)
                }
            }
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
