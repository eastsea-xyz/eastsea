import SwiftUI

struct DappSimulationView: View {
    let simulation: DappSimulation
    let chainId: UInt64
    @EnvironmentObject var model: WalletModel

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Label(simulation.success ? String(localized: "Would succeed") : String(localized: "Would fail"),
                  systemImage: simulation.success ? "checkmark.circle" : "exclamationmark.triangle")
                .font(.aeHeadline).foregroundStyle(simulation.success ? Color.primary : Color.warn)
            if let reason = simulation.failureReason {
                Text(reason).font(.aeBody).fixedSize(horizontal: false, vertical: true).textSelection(.enabled)
            }
            Text("Balance changes").font(.aeFootnote).foregroundStyle(.secondary)
            if simulation.nativeDeltaWei != "0" {
                Text("\(amount(simulation.nativeDeltaWei, decimals: 18)) \(Brand.coinTicker(chainId: chainId))")
                    .font(.aeBody.monospacedDigit()).textSelection(.enabled)
            }
            ForEach(simulation.changes) { change in
                VStack(alignment: .leading, spacing: 3) {
                    if let tokenId = change.tokenId {
                        Text("\(change.delta.hasPrefix("-") ? String(localized: "Send NFT") : String(localized: "Receive NFT")) #\(tokenId)").font(.aeBody)
                    } else if let token = model.tokens.first(where: { $0.token.address.lowercased() == change.contract })?.token,
                              let decimals = TokenDenomination.of(chainId: chainId, address: change.contract, claimed: token).decimals {
                        Text("\(amount(change.delta, decimals: decimals)) \(token.symbol)").font(.aeBody.monospacedDigit())
                    } else {
                        Text("\(change.delta) smallest units").font(.aeBody.monospacedDigit())
                    }
                    Text(change.contract).font(.aeCaption.monospaced()).foregroundStyle(.secondary).textSelection(.enabled)
                }
            }
            if simulation.nativeDeltaWei == "0" && simulation.changes.isEmpty {
                Text("No balance changes were observed.").font(.aeBody)
            }
            if !simulation.balancesMeasured && !simulation.changes.isEmpty {
                Text("Token movements reported by the contract; balances could not be measured.").font(.aeFootnote).foregroundStyle(Color.warn)
            }
            Text("Approvals").font(.aeFootnote).foregroundStyle(.secondary)
            if simulation.approvals.isEmpty {
                Text("No spending approvals were observed.").font(.aeBody)
            }
            ForEach(simulation.approvals) { approval in
                VStack(alignment: .leading, spacing: 3) {
                    Text(approval.amount == "0" ? String(localized: "Revoke spending permission")
                         : approval.unlimited ? String(localized: "Unlimited spending permission")
                         : approval.tokenId.map { String(localized: "Permission for NFT #\($0)") }
                            ?? String(localized: "Spending limit: \(approval.amount) smallest units"))
                        .font(.aeBody).foregroundStyle(approval.unlimited ? Color.warn : Color.primary)
                    Text("Spender: \(approval.spender)").font(.aeCaption.monospaced()).textSelection(.enabled)
                    Text("Token contract: \(approval.contract)").font(.aeCaption.monospaced()).textSelection(.enabled)
                }
            }
            Text("Gas used in simulation: \(simulation.gasUsed)").font(.aeFootnote).foregroundStyle(.secondary)
            Text("This is the node’s estimate, before fees. State can change; contracts may emit incomplete or misleading events.")
                .font(.aeFootnote).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
        }
        .padding(12)
        .background(.background.tertiary, in: RoundedRectangle(cornerRadius: Radius.inner))
    }

    private func amount(_ raw: String, decimals: Int) -> String {
        let negative = raw.hasPrefix("-")
        var value = TokenAmount.exact(negative ? String(raw.dropFirst()) : raw, decimals: decimals)
        if value.contains(".") {
            while value.hasSuffix("0") { value.removeLast() }
            if value.hasSuffix(".") { value.removeLast() }
        }
        return (negative ? "−" : "+") + value
    }
}

struct TypedMessageFieldsView: View {
    let fields: TypedMessageFields

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text("Message type: \(fields.primaryType)").font(.aeHeadline)
            Text("Domain").font(.aeFootnote).foregroundStyle(.secondary)
            fieldRows(fields.domain)
            if !fields.domain.contains(where: { $0.path == "verifyingContract" }) {
                Text("No verifying contract").font(.aeFootnote).foregroundStyle(Color.warn)
            }
            Divider()
            Text("Message fields").font(.aeFootnote).foregroundStyle(.secondary)
            fieldRows(fields.message)
        }
        .padding(12)
        .background(.background.tertiary, in: RoundedRectangle(cornerRadius: Radius.inner))
    }

    private func fieldRows(_ values: [TypedMessageField]) -> some View {
        ForEach(values) { field in
            VStack(alignment: .leading, spacing: 3) {
                Text(field.path).font(.aeCaption).foregroundStyle(.secondary)
                Text(field.value).font(.aeBody).textSelection(.enabled).fixedSize(horizontal: false, vertical: true)
                if let detail = field.detail {
                    DisclosureGroup("Bytes") {
                        Text(detail).font(.aeCaption.monospaced()).textSelection(.enabled).fixedSize(horizontal: false, vertical: true)
                    }
                }
            }
        }
    }
}

/// Both the capability upgrade and the exposed-original-key exit live where
/// the wallet already explains recovery: Security.
struct AccountMigrationPanel: View {
    @EnvironmentObject var model: WalletModel
    @State private var support: Bool?
    @State private var context: DappRequestContext?
    @State private var confirmUpgrade = false
    @State private var busy = false
    @State private var notice: String?
    @State private var destination = ""
    @State private var sendSheet = false
    @State private var supportGeneration: UInt64 = 0

    var body: some View {
        Card {
            VStack(alignment: .leading, spacing: 14) {
                Label("Account upgrade and migration", systemImage: "arrow.triangle.2.circlepath").font(.aeHeadline)
                Text("Message signing needs the current account code. An upgrade keeps your address, balances, recovery settings, and spending limits.")
                    .font(.aeBody).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
                if model.status?.chainId == 7780 {
                    Text("This network uses legacy account code. Message signing cannot be enabled here. Changing networks does not move your funds.")
                        .font(.aeFootnote).foregroundStyle(Color.warn).fixedSize(horizontal: false, vertical: true)
                } else if support == true {
                    Label("Message signing is enabled", systemImage: "checkmark.shield").font(.aeBody)
                } else {
                    Text(support == nil ? String(localized: "Checking account code…") : String(localized: "Message signing is not enabled"))
                        .font(.aeBody).foregroundStyle(.secondary)
                    if confirmUpgrade, let context {
                        Text("Upgrade this account: \(context.account)").font(.aeFootnote).textSelection(.enabled)
                        Text("The wallet will delegate to its verified account contract. You will approve the transaction with Touch ID or your password.")
                            .font(.aeFootnote).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
                        HStack {
                            Button("Cancel") { confirmUpgrade = false }
                            Button("Confirm account upgrade") { upgrade(context) }.buttonStyle(.borderedProminent).disabled(busy || model.busy)
                        }
                    } else {
                        Button("Enable message signing") { context = model.dappContext; confirmUpgrade = true }
                            .disabled(support == nil || busy || model.busy || model.dappContext == nil)
                    }
                }
                if let notice { Text(notice).font(.aeFootnote).foregroundStyle(Color.warn).fixedSize(horizontal: false, vertical: true) }
                Divider()
                Text("Move funds to a new account").font(.aeHeadline)
                Text("An upgrade or guardian recovery cannot revoke this account’s original key. If it may be exposed, use a new account created on a trusted device.")
                    .font(.aeFootnote).foregroundStyle(Color.warn).fixedSize(horizontal: false, vertical: true)
                TextField("New account address", text: $destination).textFieldStyle(.roundedBorder).font(.aeBody.monospaced())
                Text("Move tokens first so the old account can still pay fees. Move the remaining native coins last. Each asset opens the normal send review.")
                    .font(.aeFootnote).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
                ForEach(model.tokens) { token in
                    Button { beginMigration(token) } label: { Text("Move \(token.token.symbol) · \(Short.address(token.token.address))") }
                        .disabled(!canMove)
                }
                Button { beginMigration(nil) } label: { Text("Move remaining \(Brand.coinTicker(chainId: model.status?.chainId ?? 0))") }
                    .disabled(!canMove || model.account == nil)
                Text("Check NFTs and other assets separately. Revoke old approvals, orders, and sign-in sessions; update saved payment addresses and future deposits.")
                    .font(.aeFootnote).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            }
        }
        .task(id: model.status?.chainId) { await checkSupport() }
        .task(id: model.account?.certifiedBlock) { await checkSupport() }
        .onChange(of: model.address) { _, _ in support = nil; confirmUpgrade = false; Task { await checkSupport() } }
        .sheet(isPresented: $sendSheet) { SendSheet().environmentObject(model) }
    }

    private var canMove: Bool {
        AccountMigration.validDestination(destination.trimmingCharacters(in: .whitespacesAndNewlines), current: model.address)
            && !model.busy && !busy
    }

    private func checkSupport() async {
        supportGeneration &+= 1
        let turn = supportGeneration
        #if WALLET_SCREENS
        support = false
        return
        #else
        guard let snapshot = model.dappContext else { return }
        do {
            let result = try await Task.detached { try accountSigningSupport(address: snapshot.account) }.value
            guard !Task.isCancelled, turn == supportGeneration, model.isCurrentDappContext(snapshot) else { return }
            support = result
            context = snapshot
        } catch {
            guard !Task.isCancelled, turn == supportGeneration, model.isCurrentDappContext(snapshot) else { return }
            support = nil
            notice = String(localized: "The node did not answer. Turn it on and try again.")
        }
        #endif
    }

    private func upgrade(_ snapshot: DappRequestContext) {
        busy = true
        Task {
            let result = await model.redelegateAccount(context: snapshot, stillApproved: { confirmUpgrade })
            busy = false
            confirmUpgrade = false
            notice = result.hash != nil ? String(localized: "Account upgrade submitted. Check Activity for the final result.") : result.refusal
            await checkSupport()
        }
    }

    private func beginMigration(_ token: TokenHolding?) {
        guard canMove else { return }
        let to = destination.trimmingCharacters(in: .whitespacesAndNewlines)
        if let token {
            guard let decimals = TokenDenomination.of(chainId: model.status?.chainId ?? 0, address: token.token.address, claimed: token.token).decimals else { return }
            model.sendAmount = TokenAmount.exact(token.balance, decimals: decimals)
        } else {
            guard let balance = model.account?.balanceWei,
                  let fee = try? transferQuote(recipient: to, validators: model.validators).feeWei else {
                notice = String(localized: "The node did not answer. Turn it on and try again.")
                return
            }
            model.sendAmount = Wei.exact(WeiMath.subtract(balance, fee))
        }
        model.sendTo = to
        model.sendToken = token
        model.paymentRequest = nil
        sendSheet = true
    }
}
