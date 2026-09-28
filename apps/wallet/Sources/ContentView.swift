import SwiftUI

/// Simple mode (default): a dashboard for everyday use. Developer mode: the
/// full view with verification details, raw logs and blocks. The switch is out of
/// the way: View ▸ Developer Mode (⇧⌘D) and Settings on the Mac, the bottom of the
/// Network page on iPhone; Developer mode itself has a Done button to leave it.
struct ContentView: View {
    @EnvironmentObject var model: WalletModel
    @AppStorage("developerMode") private var developerMode = false
    /// The terms version this user accepted (0: none yet).
    @AppStorage("acceptedTerms") private var acceptedTerms = 0

    var body: some View {
        page
            .onAppear { model.start() }
            .sheet(isPresented: Binding(get: { Self.needsTerms(acceptedTerms) }, set: { _ in })) {
                TermsSheet { acceptedTerms = Terms.version }
            }
    }

    /// Design previews skip the gate: screenshots need the dashboard, not terms.
    private static func needsTerms(_ accepted: Int) -> Bool {
        guard accepted < Terms.version else { return false }
        #if DEBUG
        if DesignPreview.on { return false }
        #endif
        return true
    }

    @ViewBuilder private var page: some View {
        if developerMode {
            DeveloperView()
        } else {
            SimpleDashboard()
        }
    }
}

struct DeveloperView: View {
    @EnvironmentObject var model: WalletModel
    @AppStorage("developerMode") private var developerMode = false

    #if os(macOS)
    /// Below this width the blocks panel moves under the wallet and the page scrolls.
    @State private var stacked = false
    #endif

    var body: some View {
        #if os(macOS)
        Group {
            if stacked {
                stackedLayout
            } else {
                HStack(alignment: .top, spacing: 16) {
                    VStack(alignment: .leading, spacing: 14) {
                        header
                        accountCard
                        sendCard
                        recoveryCard
                        activity
                    }
                    .frame(minWidth: 400)
                    blocksPanel.frame(width: 300)
                }
                .padding(20)
            }
        }
        .frame(minWidth: 380, minHeight: 520)
        .onGeometryChange(for: Bool.self) { $0.size.width < 760 } action: { stacked = $0 }
        #else
        stackedLayout
        #endif
    }

    /// One column: iPhone and narrow Mac windows.
    private var stackedLayout: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                header
                accountCard
                sendCard
                recoveryCard
                activity
                blocksPanel.frame(height: 320)
            }
            .padding(16)
        }
    }

    private var header: some View {
        HStack {
            Image(systemName: "cube.transparent").font(.title)
            VStack(alignment: .leading) {
                Text("Aether Wallet").font(.title2.bold())
                if let s = model.status {
                    Text("devnet \(s.chainId) · height \(s.height) · \(model.validators) validators")
                        .font(.caption).foregroundStyle(.secondary)
                } else {
                    Text("connecting…").font(.caption).foregroundStyle(.secondary)
                }
                // Under the title (not beside it) so it stays readable in a narrow window.
                Label(model.connectionInfo, systemImage: "network").font(.caption).foregroundStyle(.secondary)
                    .lineLimit(1).truncationMode(.middle)
            }
            Spacer(minLength: 0)
            Button("Done") { withAnimation(.easeInOut(duration: 0.2)) { developerMode = false } }
                .help("Leave Developer mode")
        }
    }

    private var accountCard: some View {
        GroupBox {
            VStack(alignment: .leading, spacing: 8) {
                HStack {
                    Text(model.address.isEmpty ? "—" : model.address).font(.callout.monospaced()).textSelection(.enabled)
                        .lineLimit(1).truncationMode(.middle)
                    Button { Clipboard.copy(model.address) }
                        label: { Image(systemName: "doc.on.doc") }.buttonStyle(.borderless)
                }
                Text(model.account.map { "\(Wei.format($0.balanceWei)) AETH" } ?? "…")
                    .font(.system(size: 34, weight: .semibold, design: .rounded))
                if let a = model.account, model.verifyError == nil {
                    Label("Verified by this device", systemImage: "checkmark.seal.fill").foregroundStyle(.green).font(.headline)
                    Text("Block \(a.certifiedBlock) finality: one BLS threshold signature from a \(a.validators)-validator committee, checked against its group key · state root \(a.stateRoot.prefix(12))… · EIP-7864 proof for this address")
                        .font(.caption).foregroundStyle(.secondary)
                } else if let e = model.verifyError {
                    Label("Not verified", systemImage: "exclamationmark.triangle.fill").foregroundStyle(.orange).font(.headline)
                    Text(e).font(.caption).foregroundStyle(.secondary).lineLimit(3)
                }
                HStack {
                    Label(model.keyLabel, systemImage: "lock.shield").font(.caption)
                    Spacer()
                    Button("Get 10 test AETH") { model.faucet() }.disabled(model.busy || model.address.isEmpty)
                }
            }.frame(maxWidth: .infinity, alignment: .leading)
        } label: { Text("Account") }
    }

    private var sendCard: some View {
        GroupBox {
            HStack {
                TextField("0x recipient (several: comma-separated, one signature)", text: $model.sendTo).textFieldStyle(.roundedBorder).font(.callout.monospaced())
                TextField("AETH", text: $model.sendAmount).textFieldStyle(.roundedBorder).frame(width: 80)
                Button("Send") { model.send() }.keyboardShortcut(.return).disabled(model.busy || model.sendTo.isEmpty)
            }
        } label: { Text("Send (signed in the Secure Enclave)") }
    }

    private var recoveryCard: some View {
        GroupBox {
            VStack(alignment: .leading, spacing: 8) {
                HStack {
                    Text("This device's recovery-key code").font(.caption).foregroundStyle(.secondary)
                    Text(model.recoveryCode.isEmpty ? "…" : "\(model.recoveryCode.prefix(16))…").font(.caption.monospaced())
                        .lineLimit(1).truncationMode(.middle)
                    Button {
                        Clipboard.copy(model.recoveryCode)
                        model.note("Recovery-key code copied. Give it to the account owner who wants this Mac as their recovery key.")
                    } label: { Image(systemName: "doc.on.doc") }.buttonStyle(.borderless).disabled(model.recoveryCode.isEmpty)
                }
                HStack {
                    TextField("Other device's recovery-key code", text: $model.guardianInput).textFieldStyle(.roundedBorder).font(.caption.monospaced())
                    Button("Make it my recovery key") { model.setRecoveryKey() }.disabled(model.busy || model.guardianInput.isEmpty)
                }
                HStack {
                    TextField("0x lost account (that trusts this device)", text: $model.lostInput).textFieldStyle(.roundedBorder).font(.caption.monospaced())
                    Button("Propose recovery") { model.recover() }.disabled(model.busy || model.lostInput.isEmpty || model.outgoingRecovery != nil)
                }
                if let p = model.outgoingRecovery {
                    HStack {
                        Text("Pending: \(Wei.format(p.request.valueWei)) AETH from \(p.request.lost.prefix(10))… · ready \(p.readyAt.formatted())").font(.caption)
                        Spacer()
                        Button("Finish") { model.finishRecovery() }.disabled(model.busy || !p.isReady)
                    }
                }
                if let r = model.incomingRecovery {
                    HStack {
                        Label("A recovery of THIS account is pending (ready \(Date(timeIntervalSince1970: TimeInterval(r.readyAt)).formatted()))", systemImage: "exclamationmark.triangle.fill")
                            .font(.caption).foregroundStyle(.orange)
                        Spacer()
                        Button("Cancel it") { model.cancelIncomingRecovery() }.disabled(model.busy)
                        Button("Cancel and remove all recovery keys") { model.removeRecoveryKeys() }.disabled(model.busy)
                    }
                }
            }
        } label: { Text("Recovery (second device's Secure Enclave key · delayed, cancellable)") }
    }

    private var activity: some View {
        GroupBox {
            ScrollView {
                VStack(alignment: .leading, spacing: 4) {
                    ForEach(Array(model.log.enumerated()), id: \.offset) { _, line in
                        Text(line).font(.caption.monospaced()).frame(maxWidth: .infinity, alignment: .leading)
                    }
                }
            }.frame(minHeight: 120)
        } label: { Text("Activity") }
    }

    private var blocksPanel: some View {
        GroupBox {
            ScrollView {
              LazyVStack(alignment: .leading, spacing: 10) {
                ForEach(model.blocks, id: \.height) { b in
                VStack(alignment: .leading, spacing: 2) {
                    HStack {
                        Text("#\(b.height)").font(.callout.bold().monospacedDigit())
                        Spacer()
                        Text("\(b.txs) tx").font(.caption).foregroundStyle(b.txs > 0 ? .primary : .secondary)
                    }
                    Text("root \(b.stateRoot.prefix(18))…").font(.caption2.monospaced()).foregroundStyle(.secondary)
                    Text("proposer \(b.proposer.prefix(10))…").font(.caption2.monospaced()).foregroundStyle(.secondary)
                    Divider()
                }
                }
              }.padding(.vertical, 4)
            }
        } label: { Text("Finalized blocks") }
    }
}

enum Clipboard {
    static func copy(_ s: String) {
        #if os(macOS)
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(s, forType: .string)
        #else
        UIPasteboard.general.string = s
        #endif
    }
}
