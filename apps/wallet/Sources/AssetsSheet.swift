import SwiftUI

/// Everything this wallet holds: AETH (verified on this device) and the ERC-20 tokens
/// with a balance (the node's answer, labeled as such).
struct AssetsSheet: View {
    @EnvironmentObject var model: WalletModel
    @Environment(\.dismiss) private var dismiss

    private var balance: Double? { model.account.flatMap { Double(Wei.format($0.balanceWei)) } }

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("Assets").font(.aeTitle)
            TokenRow(symbol: "AETH", name: "Aether", amount: balance, verified: model.account != nil && model.verifyError == nil)
            if let since = model.chainPausedSince { NetworkPausedBadge(since: since) }
            Divider()
            tokens
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

    @ViewBuilder private var tokens: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(alignment: .firstTextBaseline) {
                Text("Tokens").font(.aeHeadline)
                Spacer()
                Text("Read from the node · not verified on this device").font(.aeCaption).foregroundStyle(.secondary)
                    .multilineTextAlignment(.trailing)
            }
            if !model.tokens.isEmpty {
                ForEach(model.tokens) { t in
                    TokenHoldingRow(holding: t)
                    if t.id != model.tokens.last?.id { Divider() }
                }
            } else if model.tokensUpdated == nil {
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

    private func updated(now: Date) -> String {
        guard let at = model.tokensUpdated else { return "" }
        return now.timeIntervalSince(at) < 45 ? "Tokens updated just now" : "Tokens updated \(RelativeDateTimeFormatter().localizedString(for: at, relativeTo: now))"
    }
}

private struct TokenHoldingRow: View {
    let holding: TokenHolding

    var body: some View {
        HStack(spacing: 12) {
            Text(String(holding.token.symbol.prefix(1)).uppercased()).font(.aeHeadline).foregroundStyle(.secondary)
                .frame(width: 40, height: 40)
                .background(.quaternary, in: Circle())
            VStack(alignment: .leading, spacing: 2) {
                Text(holding.token.name.isEmpty ? holding.token.symbol : holding.token.name).font(.aeBody.weight(.semibold)).lineLimit(1)
                Text(Short.address(holding.token.address)).font(.aeCaption.monospaced()).foregroundStyle(.secondary)
            }
            Spacer(minLength: 8)
            Text("\(holding.amount) \(holding.token.symbol)").font(.aeBody.weight(.semibold).monospacedDigit())
                .lineLimit(1).minimumScaleFactor(0.7)
        }
        .contextMenu {
            Button("Copy Token Address") { Clipboard.copy(holding.token.address) }
        }
    }
}
