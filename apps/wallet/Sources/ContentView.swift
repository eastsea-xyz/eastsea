import SwiftUI

/// Simple mode (default): a dashboard for everyday use. Developer mode: the
/// full view with verification details, raw logs and blocks. The switch is out of
/// the way: View ▸ Developer Mode (⇧⌘D) and Settings on the Mac, the bottom of the
/// Network page on iPhone; Developer mode itself has a Done button to leave it.
struct ContentView: View {
    @EnvironmentObject var model: WalletModel
    #if os(macOS)
    @EnvironmentObject var node: NodeController
    #endif
    @AppStorage("developerMode") private var developerMode = false
    /// The terms version this user accepted (0: none yet).
    @AppStorage("acceptedTerms") private var acceptedTerms = 0

    var body: some View {
        page
            .eastSeaPage()
            .eastSeaPresentation(.panel, value: developerMode)
            .onAppear { model.start() }
            .onChange(of: developerMode) { _, enabled in
                if !enabled && model.developmentNetwork {
                    model.selectNetwork(development: false)
                    #if os(macOS)
                    node.refreshWalletRoute()
                    #endif
                }
            }
            .safeAreaInset(edge: .top, spacing: 0) {
                if model.developmentNetwork {
                    Text("Dev network · 127.0.0.1")
                        .font(.aeCaption.bold()).frame(maxWidth: .infinity)
                        .padding(.vertical, DesignTokens.Space.s2)
                        .background(DesignTokens.Palette.warn.color)
                        .foregroundStyle(DesignTokens.Palette.bg.color)
                }
            }
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
                HStack(alignment: .top, spacing: DesignTokens.Space.s4) {
                    VStack(alignment: .leading, spacing: DesignTokens.Space.s4) {
                        header
                        accountCard
                        sendCard
                        recoveryCard
                        activity
                    }
                    .frame(minWidth: 400)
                    blocksPanel.frame(width: 300)
                }
                .padding(DesignTokens.Space.s5)
            }
        }
        .frame(minWidth: 380, minHeight: 520)
        .onGeometryChange(for: Bool.self) { $0.size.width < 760 } action: { stacked = $0 }
        .measuringNarrowLayout()
        .eastSeaPage()
        #else
        stackedLayout.measuringNarrowLayout().eastSeaPage()
        #endif
    }

    /// One column: iPhone and narrow Mac windows.
    private var stackedLayout: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: DesignTokens.Space.s4) {
                header
                accountCard
                sendCard
                recoveryCard
                activity
                blocksPanel.frame(height: 320)
            }
            .padding(DesignTokens.Space.s4)
        }
    }

    private var header: some View {
        HStack {
            EastSeaDawnMark().frame(width: 40, height: 40)
            VStack(alignment: .leading) {
                Text("\(Brand.name) Wallet").font(.aeTitle)
                if let s = model.status {
                    Text(model.developmentNetwork
                         ? String(localized: "devnet \(String(s.chainId)) · height \(String(s.height)) · \(model.validators) validators")
                         : String(localized: "chain \(String(s.chainId)) · height \(String(s.height)) · \(model.validators) validators"))
                        .font(.aeCaption).foregroundStyle(DesignTokens.Palette.textMuted.color)
                } else {
                    Text("connecting…").font(.aeCaption).foregroundStyle(DesignTokens.Palette.textMuted.color)
                }
                // Under the title (not beside it) so it stays readable in a narrow window.
                Label(model.connectionInfo, systemImage: "network").font(.aeCaption).foregroundStyle(DesignTokens.Palette.textMuted.color)
                    .lineLimit(1).truncationMode(.middle)
            }
            Spacer(minLength: 0)
            Button("Done") { developerMode = false }
                .buttonStyle(EastSeaQuietButtonStyle())
                .help("Leave Developer mode")
        }
    }

    private var accountCard: some View {
        Card {
            VStack(alignment: .leading, spacing: DesignTokens.Space.s3) {
                HStack {
                    Text("Account").font(.aeHeadline)
                    Spacer()
                    AccountSwitcherButton(store: model.accountStore, compact: true)
                }
                HStack {
                    Text(model.address.isEmpty ? "—" : model.address).font(.aeBody.monospaced()).textSelection(.enabled)
                        .lineLimit(1).truncationMode(.middle)
                    Button { Clipboard.copy(model.address) }
                        label: { Image(systemName: "doc.on.doc") }.buttonStyle(.borderless)
                }
                Text(model.account.map { "\(Wei.format($0.balanceWei)) \(Brand.networkCoinTicker)" } ?? "…")
                    .font(DesignTokens.TypeScale.amountMd.font).monospacedDigit()
                    .lineLimit(1).minimumScaleFactor(0.5)
                    .accessibilityLabel("\(model.account.map { Wei.exact($0.balanceWei) } ?? "…") \(Brand.networkCoinTicker)")
                    .foregroundStyle(DesignTokens.Palette.plateInk.color)
                    .padding(DesignTokens.Space.s4)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .eastSeaNavyPlate()
                if let a = model.account, model.verifyError == nil {
                    Label("Verified by this device", systemImage: "checkmark.seal.fill").foregroundStyle(DesignTokens.Palette.success.color).font(.aeHeadline)
                    Text("Block \(String(a.certifiedBlock)) finality: one BLS threshold signature from a \(a.validators)-validator committee, checked against its group key · state root \(a.stateRoot.prefix(12))… · EIP-7864 proof for this address")
                        .font(.aeCaption).foregroundStyle(DesignTokens.Palette.textMuted.color)
                } else if let e = model.verifyError {
                    Label("Not verified", systemImage: "exclamationmark.triangle.fill").foregroundStyle(DesignTokens.Palette.warn.color).font(.aeHeadline)
                    Text(e).font(.aeCaption).foregroundStyle(DesignTokens.Palette.textMuted.color).lineLimit(3)
                }
                HStack {
                    Label(model.keyLabel, systemImage: "lock.shield").font(.aeCaption)
                    Spacer()
                    if model.developmentNetwork {
                        Button("Get 10 test \(Brand.networkCoinTicker)") { model.faucet() }.disabled(model.busy || model.address.isEmpty)
                    }
                }
            }.frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    private var sendCard: some View {
        Card {
            VStack(alignment: .leading, spacing: DesignTokens.Space.s3) {
                Text("Send (signed in the Secure Enclave)").font(.aeHeadline)
                AdaptiveStack(spacing: DesignTokens.Space.s3) {
                    TextField("0x recipient (several: comma-separated, one signature)", text: $model.sendTo).textFieldStyle(EastSeaTextFieldStyle()).font(.aeBody.monospaced())
                    TextField("\(Brand.networkCoinTicker)", text: $model.sendAmount).textFieldStyle(EastSeaTextFieldStyle()).frame(width: 100)
                    Button("Send") { Task { await model.send() } }
                        .buttonStyle(EastSeaPrimaryButtonStyle())
                        .keyboardShortcut(.return).disabled(model.busy || model.sendTo.isEmpty)
                }
            }
        }
    }

    private var recoveryCard: some View {
        Card {
            VStack(alignment: .leading, spacing: DesignTokens.Space.s3) {
                Text("Recovery (second device's Secure Enclave key · delayed, cancellable)").font(.aeHeadline)
                AdaptiveStack(spacing: DesignTokens.Space.s3) {
                    Text("This device's recovery-key code").font(.aeCaption).foregroundStyle(DesignTokens.Palette.textMuted.color)
                    Text(model.recoveryCode.isEmpty ? "…" : "\(model.recoveryCode.prefix(16))…").font(.aeCaption.monospaced())
                        .lineLimit(1).truncationMode(.middle)
                    Button {
                        Clipboard.copy(model.recoveryCode)
                        model.note("Recovery-key code copied. Give it to the account owner who wants this Mac as their recovery key.")
                    } label: { Image(systemName: "doc.on.doc") }.buttonStyle(.borderless).disabled(model.recoveryCode.isEmpty)
                }
                AdaptiveStack(spacing: DesignTokens.Space.s3) {
                    TextField("Other device's recovery-key code", text: $model.guardianInput).textFieldStyle(EastSeaTextFieldStyle()).font(.aeCaption.monospaced())
                    Button("Make it my recovery key") { model.setRecoveryKey() }.disabled(model.busy || model.guardianInput.isEmpty)
                }
                AdaptiveStack(spacing: DesignTokens.Space.s3) {
                    TextField("0x lost account (that trusts this device)", text: $model.lostInput).textFieldStyle(EastSeaTextFieldStyle()).font(.aeCaption.monospaced())
                    Button("Propose recovery") { model.recover() }.disabled(model.busy || model.lostInput.isEmpty || model.outgoingRecovery != nil)
                }
                // F-05: recovery saves a lost key, never a stolen one.
                Text(KeyExposureNotice.recovery.text).font(.aeCaption).foregroundStyle(DesignTokens.Palette.warn.color)
                    .fixedSize(horizontal: false, vertical: true)
                if let p = model.outgoingRecovery {
                    AdaptiveStack(spacing: DesignTokens.Space.s3) {
                        Text("Pending: \(Wei.format(p.request.valueWei)) \(Brand.networkCoinTicker) from \(p.request.lost.prefix(10))… · ready \(p.readyAt.formatted())").font(.aeCaption)
                        Spacer()
                        Button("Finish") { model.finishRecovery() }.disabled(model.busy || !p.isReady)
                    }
                }
                if let r = model.incomingRecovery {
                    AdaptiveStack(spacing: DesignTokens.Space.s3) {
                        Label("A recovery of THIS account is pending (ready \(Date(timeIntervalSince1970: TimeInterval(r.readyAt)).formatted()))", systemImage: "exclamationmark.triangle.fill")
                            .font(.aeCaption).foregroundStyle(DesignTokens.Palette.warn.color)
                        Spacer()
                        Button("Cancel it") { model.cancelIncomingRecovery() }.disabled(model.busy)
                        Button("Cancel and remove all recovery keys") { model.removeRecoveryKeys() }.disabled(model.busy)
                    }
                }
            }
        }
    }

    private var activity: some View {
        VStack(alignment: .leading, spacing: DesignTokens.Space.s2) {
            LinkedWalletsCard()
            Card {
                VStack(alignment: .leading, spacing: DesignTokens.Space.s3) {
                    Text("Activity").font(.aeHeadline)
                    ScrollView {
                        LazyVStack(alignment: .leading, spacing: DesignTokens.Space.s2) {
                            ForEach(model.activity) { item in
                                VStack(alignment: .leading, spacing: 2) {
                                    Text(item.title).font(.aeBody)
                                    Text("\(item.source ?? String(localized: "On this device")) · \(item.date.formatted()) · \(item.hash ?? "")")
                                        .font(.aeCaption).foregroundStyle(DesignTokens.Palette.textMuted.color)
                                }
                                Divider().overlay(DesignTokens.Palette.line.color)
                            }
                        }
                    }
                    .frame(minHeight: 120, maxHeight: 300)
                    if model.olderActivityAvailable {
                        Button("Load older activity") { model.loadOlderActivity() }
                    }
                }
            }
        }
    }

    private var blocksPanel: some View {
        Card {
            VStack(alignment: .leading, spacing: DesignTokens.Space.s3) {
                Text("Finalized blocks").font(.aeHeadline)
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: DesignTokens.Space.s3) {
                        ForEach(model.blocks, id: \.height) { b in
                            VStack(alignment: .leading, spacing: DesignTokens.Space.s1) {
                                HStack {
                                    Text("#\(String(b.height))").font(.aeBody.bold().monospacedDigit())
                                    Spacer()
                                    Text("\(b.txs) tx").font(.aeCaption)
                                        .foregroundStyle(b.txs > 0 ? DesignTokens.Palette.text.color : DesignTokens.Palette.textMuted.color)
                                }
                                Text("root \(b.stateRoot.prefix(18))…").font(.aeCaption.monospaced()).foregroundStyle(DesignTokens.Palette.textMuted.color)
                                Text("proposer \(b.proposer.prefix(10))…").font(.aeCaption.monospaced()).foregroundStyle(DesignTokens.Palette.textMuted.color)
                                Divider().overlay(DesignTokens.Palette.line.color)
                            }
                        }
                    }
                    .padding(.vertical, DesignTokens.Space.s1)
                }
            }
        }
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
