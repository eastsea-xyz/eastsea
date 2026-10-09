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
            HStack {
                Text("Language")
                Spacer()
                Text(verbatim: AppLanguage.nativeName).foregroundStyle(.secondary)
                Button("Change in System Settings…", action: openLanguageSettings)
            }
            if model.developmentNetwork {
                Text("Dev network · 127.0.0.1:\(String(developmentNetworkPort))")
                    .font(.caption.bold()).foregroundStyle(.orange)
            }
            Toggle("Run a node on this Mac", isOn: $node.enabled)
            AccountPayoutSettings(store: model.accountStore)
            Toggle("Only while on the power adapter", isOn: $node.onlyOnPower)
                .help("On a laptop, pause the node on battery and resume on power.")
            Label(node.awakeNote, systemImage: node.keepsAwake ? "sun.max.fill" : "moon.zzz")
                .font(.caption).foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            Toggle("Open \(Brand.name) at login", isOn: Binding(get: { node.startAtLogin }, set: { node.startAtLogin = $0 }))
            UnattendedSection()
            PublicReadSection()
            HistoryStorageSection()
            ResourcesSection()
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
        VStack(alignment: .leading, spacing: 6) {
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
            Text("Choose where node rewards are received. Switching your wallet account does not change this.")
                .font(.caption).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            if let error { Text(error).font(.caption).foregroundStyle(.orange) }
        }
    }
}

/// The same preference file controls the app's follower and the unattended
/// node. Changes are atomic and the public-read service reloads each request.
struct PublicReadSection: View {
    @State private var settings = PublicReadSettings.defaultValue
    @State private var needsRepair = false
    @State private var saveFailed = false

    private let offeredLimits: [UInt64] = [64, 128, 256, 512, 1024]
        .map { $0 * PublicReadSettings.bytesPerMiB }

    private var enabled: Binding<Bool> {
        Binding(get: { settings.enabled }, set: { value in
            var next = settings
            next.enabled = value
            apply(next)
        })
    }

    private var dailyBytes: Binding<UInt64> {
        Binding(get: { settings.dailyBytes }, set: { value in
            var next = settings
            next.dailyBytes = value
            apply(next)
        })
    }

    private func limitInMiB(_ bytes: UInt64) -> String {
        if bytes.isMultiple(of: PublicReadSettings.bytesPerMiB) {
            return String(bytes / PublicReadSettings.bytesPerMiB)
        }
        return String(format: "%.2f", locale: Locale.current,
                      Double(bytes) / Double(PublicReadSettings.bytesPerMiB))
    }

    var body: some View {
        Section("Share chain data") {
            Toggle("Help browsers read the chain", isOn: enabled)
            Picker("Daily sharing limit", selection: dailyBytes) {
                ForEach(offeredLimits, id: \.self) { bytes in
                    Text("\(limitInMiB(bytes)) MiB").tag(bytes)
                }
                // Preserve a limit chosen outside the app, including zero;
                // opening Settings must never silently replace that budget.
                if !offeredLimits.contains(settings.dailyBytes) {
                    Text("\(limitInMiB(settings.dailyBytes)) MiB").tag(settings.dailyBytes)
                }
            }
            .disabled(!settings.enabled)
            Text("Your Mac shares finalized blocks with browsers while its node is running. Sharing is read-only and stops at your daily limit.")
                .font(.caption).foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            Text("Changes apply immediately. The limit resets at midnight UTC.")
                .font(.caption).foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            if needsRepair {
                Label("Sharing is paused because its saved settings could not be read. Turn sharing on to save them again.",
                      systemImage: "exclamationmark.triangle")
                    .font(.caption).foregroundStyle(.orange)
                    .fixedSize(horizontal: false, vertical: true)
            }
            if saveFailed {
                Label("Could not save sharing settings. Change the setting to try again.",
                      systemImage: "exclamationmark.triangle")
                    .font(.caption).foregroundStyle(.orange)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .onAppear {
            let read = PublicReadSettings.read(in: NodeController.dataDir)
            settings = read.settings
            needsRepair = read.needsRepair
            saveFailed = false
        }
    }

    private func apply(_ next: PublicReadSettings) {
        do {
            try next.write(in: NodeController.dataDir)
            settings = next
            needsRepair = false
            saveFailed = false
        } catch {
            saveFailed = true
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
