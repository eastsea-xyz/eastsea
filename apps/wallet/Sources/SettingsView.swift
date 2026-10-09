#if os(macOS)
import SwiftUI

/// EastSea ▸ Settings: how the node runs on this Mac.
struct SettingsView: View {
    @EnvironmentObject var node: NodeController
    @EnvironmentObject var model: WalletModel
    @EnvironmentObject var updates: Updates
    @EnvironmentObject var unattended: UnattendedDaemon
    @AppStorage("developerMode") private var developerMode = false
    @AppStorage("useDevelopmentNetwork") private var useDevelopmentNetwork = false
    @AppStorage("developmentNetworkPort") private var developmentNetworkPort = 18546

    var body: some View {
        Form {
            VStack(alignment: .leading, spacing: DesignTokens.Space.s6) {
                HStack(spacing: DesignTokens.Space.s3) {
                    EastSeaDawnMark().frame(width: 32, height: 32)
                    Text("Settings").font(.aeTitle)
                }
                SettingsSection(LocalizedStringKey("General"), explanation: LocalizedStringKey("Language follows your macOS settings.")) {
                    SettingsControlRow(LocalizedStringKey("Language")) {
                        VStack(alignment: .trailing, spacing: DesignTokens.Space.s2) {
                            Text(verbatim: AppLanguage.nativeName).foregroundStyle(DesignTokens.Palette.textMuted.color)
                            Button("Change in System Settings…", action: openLanguageSettings)
                                .controlSize(.small)
                        }
                    }
                }
                if model.developmentNetwork {
                    Label("Dev network · 127.0.0.1:\(String(developmentNetworkPort))", systemImage: "hammer")
                        .font(.aeFootnote).foregroundStyle(Color.warn)
                }
                SettingsSection(LocalizedStringKey("Node on this Mac"), explanation: LocalizedStringKey("Verify blocks and choose where node rewards arrive.")) {
                    SettingsControlRow(LocalizedStringKey("On this Mac")) {
                        Toggle("Run a node on this Mac", isOn: $node.enabled)
                    }
                    AccountPayoutSettings(store: model.accountStore)
                    SettingsControlRow(LocalizedStringKey("On power only")) {
                        Toggle("Only while on the power adapter", isOn: $node.onlyOnPower)
                            .help("On a laptop, pause the node on battery and resume on power.")
                    }
                    Label(node.awakeNote, systemImage: node.keepsAwake ? "sun.max.fill" : "moon.zzz")
                        .font(.aeFootnote).foregroundStyle(DesignTokens.Palette.textMuted.color)
                        .fixedSize(horizontal: false, vertical: true)
                    SettingsLearnMore {
                        Text("Choose where node rewards are received. Switching your wallet account does not change this.")
                        Text("Your node verifies every block itself and your wallet asks it instead of the network. Quitting \(Brand.name) stops it.")
                        Text("On a laptop, pause the node on battery and resume on power.")
                        // Honest power ranges: no price or reward promise.
                        Text("Power: roughly 5–6 W while only verifying (about 4 kWh a month); proving on the GPU adds roughly 28–50 W (about 20–36 kWh a month).")
                    }
                }
                SettingsSection(LocalizedStringKey("Startup"), explanation: LocalizedStringKey("Choose how the node starts and resumes.")) {
                    SettingsControlRow(LocalizedStringKey("At login")) {
                        Toggle("Open \(Brand.name) at login", isOn: Binding(get: { node.startAtLogin }, set: { node.startAtLogin = $0 }))
                    }
                    UnattendedSection()
                }
                HistoryStorageSection()
                ResourcesSection()
                PresencePrivacySection()
                Section("Privacy") {
                    Group {
                        Text("Private keys stay on this device. Addresses, balances, transactions, rewards and registration records are public on chain indefinitely, even after you stop using the app.")
                        Text("Joining encrypts a DeviceCheck token to Pipln's registrar; only it can decrypt it and send it to Apple (USA), at registration and for daily checks. The registrar keeps the voting key, operator and beacon addresses, node ID and registration time without automatic expiry.")
                        Text("Peers and relays see connection IP addresses; RPC nodes see queried addresses. Cloudflare hosts the site and gateway; GitHub receives update requests made by Sparkle, including IP address and app version. Contact privacy support to delete removable service data; public chain copies cannot be recalled.")
                    }
                    .font(.caption).foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                    Link("Read the privacy policy", destination: Terms.privacyURL)
                    Link("Contact privacy support", destination: Terms.privacyContactURL)
                }
                SettingsSection(LocalizedStringKey("Developer"), explanation: LocalizedStringKey("Extra controls for local development.")) {
                    SettingsControlRow(LocalizedStringKey("Developer mode")) {
                        Toggle("Developer mode (proofs, state roots, raw logs)", isOn: $developerMode)
                            .help("Also in View ▸ Developer Mode (⇧⌘D)")
                    }
                    if developerMode {
                        SettingsControlRow(LocalizedStringKey("Network")) {
                            Picker("Network", selection: $useDevelopmentNetwork) {
                                Text("Default").tag(false)
                                Text("Local development network").tag(true)
                            }
                        }
                        SettingsControlRow(LocalizedStringKey("Local RPC")) {
                            HStack(spacing: DesignTokens.Space.s2) {
                                Text(verbatim: String(developmentNetworkPort)).monospacedDigit()
                                Stepper("Local RPC: http://127.0.0.1:\(String(developmentNetworkPort))", value: $developmentNetworkPort, in: 1024...65535)
                                    .fixedSize()
                            }
                            .disabled(!useDevelopmentNetwork)
                        }
                    }
                    SettingsLearnMore {
                        Text("Developer mode (proofs, state roots, raw logs)")
                        Text("Also in View ▸ Developer Mode (⇧⌘D)")
                        if developerMode {
                            Text("Local RPC: http://127.0.0.1:\(String(developmentNetworkPort))")
                                .monospaced().textSelection(.enabled)
                        }
                    }
                }
                if updates.pendingRelease != nil || updates.approvalIssue != nil {
                    SettingsSection(LocalizedStringKey("Software updates"), explanation: LocalizedStringKey("Releases approved by the network.")) {
                        if let pending = updates.pendingRelease {
                            Text("Approved release \(pending.version) (\(pending.build))").font(.aeBody.weight(.semibold))
                            if pending.emergency {
                                Label("Emergency release · all three builders signed", systemImage: "exclamationmark.shield")
                                    .font(.aeFootnote).foregroundStyle(Color.warn)
                            } else if let date = pending.availableAt {
                                Text("Installable after \(date.formatted())")
                                    .font(.aeFootnote).foregroundStyle(DesignTokens.Palette.textMuted.color)
                            }
                            SettingsLearnMore {
                                Text("SHA-256: \(pending.fingerprint)")
                                    .monospaced().textSelection(.enabled)
                                Text("Published in block \(String(pending.publishedBlock))")
                            }
                        }
                        if let issue = updates.approvalIssue {
                            Label(issue, systemImage: "exclamationmark.triangle")
                                .font(.aeFootnote).foregroundStyle(Color.warn)
                        }
                    }
                }
            }
        }
        .formStyle(.columns)
        .font(.aeBody)
        .foregroundStyle(DesignTokens.Palette.text.color)
        .tint(DesignTokens.Palette.accent.color)
        .onChange(of: useDevelopmentNetwork) { _, dev in
            model.selectNetwork(development: dev, port: UInt16(developmentNetworkPort))
            if !dev { node.refreshWalletRoute() }
        }
        .onChange(of: developmentNetworkPort) { _, port in
            if useDevelopmentNetwork { model.selectNetwork(development: true, port: UInt16(port)) }
        }
        .onChange(of: developerMode) { _, enabled in
            if !enabled { useDevelopmentNetwork = false; model.selectNetwork(development: false); node.refreshWalletRoute() }
        }
        .padding(DesignTokens.Space.s5)
        .background(DesignTokens.Palette.bg.color)
        .frame(width: 420)
    }

    private func openLanguageSettings() {
        let workspace = NSWorkspace.shared
        let settings = URL(string: "x-apple.systempreferences:com.apple.Localization-Settings.extension")!
        if !workspace.open(settings) {
            workspace.open(URL(fileURLWithPath: "/System/Library/PreferencePanes/Localization.prefPane"))
        }
    }
}

@MainActor
private struct AccountPayoutSettings: View {
    @ObservedObject var store: AccountStore
    @State private var error: String?

    var body: some View {
        VStack(alignment: .leading, spacing: DesignTokens.Space.s2) {
            SettingsControlRow(LocalizedStringKey("Payout account")) {
                Picker("Node payout account", selection: Binding(get: {
                    store.list().first { $0.address == store.payoutAddress }?.id ?? -1
                }, set: { id in
                    do { try store.setPayoutAccount(id); error = nil }
                    catch { self.error = error.localizedDescription }
                })) {
                    if !store.payoutAddress.isEmpty && !store.list().contains(where: { $0.address == store.payoutAddress }) {
                        Text("Current payout address").tag(-1)
                    }
                    ForEach(store.list()) { account in
                        Text(verbatim: "\(account.name) · \(Short.address(account.address))").tag(account.id)
                    }
                }
                .disabled(store.state != .ready)
            }
            if let error { Text(error).font(.aeFootnote).foregroundStyle(Color.warn) }
        }
    }
}

/// Settings ▸ the "keep this Mac's node running after restarts" switch
/// (docs/design/29-unattended-restart.md): the toggle, the one-time system
/// approval it needs, and the honest power sentences — what comes back after
/// a power cut, and where macOS itself stops (FileVault).
struct UnattendedSection: View {
    @EnvironmentObject var node: NodeController
    @EnvironmentObject var unattended: UnattendedDaemon

    var body: some View {
        VStack(alignment: .leading, spacing: DesignTokens.Space.s3) {
            SettingsControlRow(LocalizedStringKey("After restarts")) {
                Toggle("Keep this Mac's node running after restarts", isOn: $unattended.enabled)
                    .help("After a reboot the node — and this Mac's vote — come back by themselves, without anyone logging in, everywhere macOS allows it. A FileVault cold boot waits for one unlock first.")
            }
            switch unattended.status {
            case .needsApproval:
                Label("One more step: allow \(Brand.name) in System Settings ▸ General ▸ Login Items. Until then this does not run.", systemImage: "hand.raised")
                    .font(.aeFootnote).foregroundStyle(Color.warn)
                    .fixedSize(horizontal: false, vertical: true)
                Button("Open System Settings") { unattended.openApprovalPane() }
                    .controlSize(.small)
            case .failed(let why):
                Label(why, systemImage: "exclamationmark.triangle")
                    .font(.aeFootnote).foregroundStyle(Color.warn)
                    .fixedSize(horizontal: false, vertical: true)
            case .off, .approved:
                EmptyView()
            }
            SettingsLearnMore {
                Text("After a reboot the node — and this Mac's vote — come back by themselves, without anyone logging in, everywhere macOS allows it. A FileVault cold boot waits for one unlock first.")
                if unattended.enabled {
                    Label("This Mac signs blocks, or can be picked to: that is why this is on by default. While nobody is logged in, the node keeps verifying and voting; the daily reward check-in resumes when the app is open again.", systemImage: "arrow.clockwise")
                }
                ForEach(UnattendedDecision.powerLines(unattended.power), id: \.self) { line in
                    Label(line, systemImage: "bolt")
                }
            }
        }
        .onAppear {
            // The user approves outside the app; re-read both facts whenever
            // Settings appears (docs/design/29).
            unattended.refreshStatus()
            unattended.refreshPower()
        }
    }
}

/// A single quiet settings group. Reuses the wallet surface and token scale.
struct SettingsSection<Content: View>: View {
    let title: LocalizedStringKey
    let explanation: LocalizedStringKey
    let content: Content

    init(_ title: LocalizedStringKey, explanation: LocalizedStringKey, @ViewBuilder content: () -> Content) {
        self.title = title
        self.explanation = explanation
        self.content = content()
    }

    var body: some View {
        VStack(alignment: .leading, spacing: DesignTokens.Space.s3) {
            VStack(alignment: .leading, spacing: DesignTokens.Space.s1) {
                Text(title).font(.aeHeadline)
                Text(explanation).font(.aeFootnote).foregroundStyle(DesignTokens.Palette.textMuted.color)
                    .fixedSize(horizontal: false, vertical: true)
            }
            content
        }
        .padding(DesignTokens.Space.s4)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(DesignTokens.Palette.surface.color, in: RoundedRectangle(cornerRadius: DesignTokens.Radius.lg))
    }
}

/// All labels share one column; controls keep their original accessible labels.
struct SettingsControlRow<Control: View>: View {
    let title: LocalizedStringKey
    let control: Control

    init(_ title: LocalizedStringKey, @ViewBuilder control: () -> Control) {
        self.title = title
        self.control = control()
    }

    var body: some View {
        Grid(alignment: .leading, horizontalSpacing: DesignTokens.Space.s4, verticalSpacing: DesignTokens.Space.s2) {
            GridRow(alignment: .center) {
                Text(title).font(.aeBody)
                    .frame(width: 148, alignment: .leading)
                    .fixedSize(horizontal: false, vertical: true)
                control.labelsHidden().toggleStyle(.switch)
                    .frame(maxWidth: .infinity, alignment: .trailing)
                    .gridColumnAlignment(.trailing)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

/// The native disclosure keeps complete operational details in reach.
struct SettingsLearnMore<Content: View>: View {
    let content: Content

    init(@ViewBuilder content: () -> Content) { self.content = content() }

    var body: some View {
        DisclosureGroup("Learn more") {
            VStack(alignment: .leading, spacing: DesignTokens.Space.s2) { content }
                .fixedSize(horizontal: false, vertical: true)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.top, DesignTokens.Space.s2)
        }
        .font(.aeFootnote)
        .foregroundStyle(DesignTokens.Palette.textMuted.color)
    }
}
struct PresencePrivacySection: View {
    @EnvironmentObject var node: NodeController
    @State private var showCountryNotice = false

    private var countryLocale: Locale {
        Locale(identifier: Bundle.main.preferredLocalizations.first ?? "en")
    }

    var body: some View {
        Section("Live network privacy") {
            LabeledContent("Default sub-region", value: node.presenceDefaultRegionLabel)
            if node.needsCountryNotice {
                Button("Choose country sharing…") { showCountryNotice = true }
            } else {
                Toggle("Share this Mac's country", isOn: $node.presenceShareCountry)
                if node.presenceShareCountry {
                    Picker("Country", selection: $node.presenceCountryCode) {
                        Text("Choose a country").tag("")
                        ForEach(PresenceCountry.codes, id: \.self) { code in
                            Text(verbatim: countryLocale.localizedString(forRegionCode: code) ?? code).tag(code)
                        }
                    }
                }
            }
            Text("The default UN M49 sub-region follows this Mac's Region setting independently of country sharing. Country sharing requires an explicit choice and can select a different sub-region. Public presence does not publish country codes. Turning it off keeps the default sub-region. Peers and relays still see connection IP addresses.")
                .font(.caption).foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
        }
        .sheet(isPresented: $showCountryNotice) {
            CountryNoticeSheet(country: node.presenceCountryCode)
                .environmentObject(node)
                .onChange(of: node.needsCountryNotice) { _, needed in
                    if !needed { showCountryNotice = false }
                }
        }
    }
}

#endif
