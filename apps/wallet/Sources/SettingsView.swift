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
            if model.developmentNetwork {
                Text("Dev network · 127.0.0.1:\(String(developmentNetworkPort))")
                    .font(.caption.bold()).foregroundStyle(.orange)
            }
            Toggle("Run a node on this Mac", isOn: $node.enabled)
            Toggle("Only while on the power adapter", isOn: $node.onlyOnPower)
                .help("On a laptop, pause the node on battery and resume on power.")
            Label(node.awakeNote, systemImage: node.keepsAwake ? "sun.max.fill" : "moon.zzz")
                .font(.caption).foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            Toggle("Open \(Brand.name) at login", isOn: Binding(get: { node.startAtLogin }, set: { node.startAtLogin = $0 }))
            UnattendedSection()
            HistoryStorageSection()
            ResourcesSection()
            PresencePrivacySection()
            Section("Privacy") {
                Group {
                    Text("Private keys stay on this device. Addresses, balances, transactions, rewards and registration records are public on chain indefinitely, even after you stop using the app.")
                    Text("Joining encrypts a DeviceCheck token to Pipln's registrar; only it can decrypt it and send it to Apple (USA), at registration and for daily checks. The registrar keeps the voting key, operator and beacon addresses, node ID and registration time without automatic expiry.")
                    Text("Peers and relays see connection IP addresses; RPC nodes see queried addresses. Cloudflare hosts the site and gateway; GitHub receives update requests made by Sparkle, including IP address and app version. Ask privacy@eastsea.xyz to delete removable service data; public chain copies cannot be recalled.")
                }
                .font(.caption).foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
                Link("Read the privacy policy", destination: Terms.privacyURL)
            }
            Text("Your node verifies every block itself and your wallet asks it instead of the network. Quitting \(Brand.name) stops it.")
                .font(.caption).foregroundStyle(.secondary)
            // Honest power ranges (docs/research/mac-power-cost-2026.md): the node is
            // cheap; GPU proving is the costly part. No won figure — electricity
            // prices vary, and the range is the honest statement.
            Text("Power: roughly 5–6 W while only verifying (about 4 kWh a month); proving on the GPU adds roughly 28–50 W (about 20–36 kWh a month).")
                .font(.caption).foregroundStyle(.secondary)
            Divider()
            Toggle("Developer mode (proofs, state roots, raw logs)", isOn: $developerMode)
                .help("Also in View ▸ Developer Mode (⇧⌘D)")
            if developerMode {
                Picker("Network", selection: $useDevelopmentNetwork) {
                    Text("Default").tag(false)
                    Text("Local development network").tag(true)
                }
                Stepper("Local RPC: http://127.0.0.1:\(String(developmentNetworkPort))", value: $developmentNetworkPort, in: 1024...65535)
                    .disabled(!useDevelopmentNetwork)
            }
            if let pending = updates.pendingRelease {
                Divider()
                Text("Approved release \(pending.version) (\(pending.build))")
                Text("SHA-256: \(pending.fingerprint)")
                    .font(.caption.monospaced()).fixedSize(horizontal: false, vertical: true)
                    .textSelection(.enabled)
                Text("Published in block \(String(pending.publishedBlock))")
                    .font(.caption).foregroundStyle(.secondary)
                if pending.emergency {
                    Label("Emergency release · all three builders signed", systemImage: "exclamationmark.shield")
                } else if let date = pending.availableAt {
                    Text("Installable after \(date.formatted())")
                        .font(.caption).foregroundStyle(.secondary)
                }
            }
            if let issue = updates.approvalIssue {
                Text(issue).font(.caption).foregroundStyle(.orange)
            }
        }
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
        .padding(20)
        .frame(width: 420)
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
            Text("Country sharing is optional. Your selected country stays on this Mac and chooses a broad region bucket. Public observations show only counts for groups of at least three. Turning it off stops future use of the country preference. Your relay's region and connection IP address remain visible to peers.")
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

/// Settings ▸ the "keep this Mac's node running after restarts" switch
/// (docs/design/29-unattended-restart.md): the toggle, the one-time system
/// approval it needs, and the honest power sentences — what comes back after
/// a power cut, and where macOS itself stops (FileVault).
struct UnattendedSection: View {
    @EnvironmentObject var node: NodeController
    @EnvironmentObject var unattended: UnattendedDaemon

    var body: some View {
        Group {
            Toggle("Keep this Mac's node running after restarts", isOn: $unattended.enabled)
                .help("After a reboot the node — and this Mac's vote — come back by themselves, without anyone logging in, everywhere macOS allows it. A FileVault cold boot waits for one unlock first.")
            switch unattended.status {
            case .needsApproval:
                Label("One more step: allow \(Brand.name) in System Settings ▸ General ▸ Login Items. Until then this does not run.", systemImage: "hand.raised")
                    .font(.caption).foregroundStyle(.orange)
                Button("Open System Settings") { unattended.openApprovalPane() }
            case .failed(let why):
                Label(why, systemImage: "exclamationmark.triangle")
                    .font(.caption).foregroundStyle(.orange)
            case .off, .approved:
                EmptyView()
            }
            if unattended.enabled {
                Label("This Mac signs blocks, or can be picked to: that is why this is on by default. While nobody is logged in, the node keeps verifying and voting; the daily reward check-in resumes when the app is open again.", systemImage: "arrow.clockwise")
                    .font(.caption).foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            ForEach(UnattendedDecision.powerLines(unattended.power), id: \.self) { line in
                Label(line, systemImage: "bolt")
                    .font(.caption).foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
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
#endif
