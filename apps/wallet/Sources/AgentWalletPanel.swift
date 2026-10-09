#if os(macOS)
import AppKit
import Foundation
import SwiftUI

/// "AI agent payments": AI agents on this Mac (Claude Code, Codex, any MCP
/// client) pay from this wallet through the bundled agent helper, only within
/// the limits the owner set with Touch ID — the account contract enforces them
/// on chain. The Mac wallet reads only the agent's local display files. Every
/// change is made by the helper, which asks for the owner's Touch ID/password.
struct AgentWalletPanel: View {
    @EnvironmentObject private var model: WalletModel
    @State private var history: [[String: Any]] = []
    @State private var requests: [[String: Any]] = []
    @State private var names: [String: String] = [:]
    @State private var newNames: [String: String] = [:]
    @State private var message = ""
    @State private var working = false

    private var home: URL {
        FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Application Support/Aether/agent")
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(alignment: .top, spacing: 14) {
                Image(systemName: "sparkles").font(.system(size: 30)).foregroundStyle(Color.aether)
                VStack(alignment: .leading, spacing: 4) {
                    Text("AI agent payments").font(.aeHeadline)
                    Text("AI agents on this Mac, like Claude Code or Codex, can pay from this wallet — only to payees you approve and within the limits you set with Touch ID. The network refuses anything over those limits.")
                        .font(.aeBody).foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            HStack(alignment: .firstTextBaseline) {
                Text("Stopping takes the agents' permission to pay away at once, with Touch ID. Payments already sent stay sent.")
                    .font(.aeFootnote).foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                Spacer(minLength: 8)
                Button("Stop agent payments") { command(["stop"]) }
                    .disabled(working)
            }
            if !requests.isEmpty {
                Text("Payees waiting for your approval").font(.aeHeadline)
                ForEach(requests.indices, id: \.self) { i in
                    let r = requests[i]
                    let address = r["address"] as? String ?? ""
                    VStack(alignment: .leading, spacing: 5) {
                        Text(address).font(.aeFootnote.monospaced()).textSelection(.enabled)
                        Text("Purpose: \(r["purpose"] as? String ?? "—")").font(.aeFootnote)
                        Text("Amount asked: \(r["amount"] as? String ?? "—") \(r["asset"] as? String ?? "")").font(.aeFootnote)
                        HStack {
                            TextField("Payee name", text: Binding(get: { newNames[address] ?? "" }, set: { newNames[address] = $0 }))
                                .textFieldStyle(.roundedBorder)
                            Button("Allow with Touch ID") {
                                command(["payee", "add", "--name", newNames[address] ?? "", "--address", address])
                            }.disabled(working || (newNames[address] ?? "").trimmingCharacters(in: .whitespaces).isEmpty)
                        }
                    }
                }
            }
            Text("Agent payments so far").font(.aeHeadline)
            if history.isEmpty { Text("No agent payments yet.").font(.aeFootnote).foregroundStyle(.secondary) }
            ForEach(history.indices, id: \.self) { i in
                let item = history[i]
                let hash = item["hash"] as? String ?? ""
                VStack(alignment: .leading, spacing: 4) {
                    Text("\(status(item)) · \(item["amount"] as? String ?? "—") \(item["asset"] as? String ?? Brand.networkCoinTicker)")
                        .font(.aeBody)
                    Text(date(item)).font(.aeFootnote).foregroundStyle(.secondary)
                    Text("To: \(displayNames(item))").font(.aeFootnote)
                    Text("Purpose: \(item["purpose"] as? String ?? String(localized: "not recorded"))").font(.aeFootnote)
                    Button("Show receipt · \(String(hash.prefix(12)))…") { command(["receipt", "--hash", hash]) }
                        .font(.aeFootnote).disabled(working || hash.isEmpty)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                Divider()
            }
            if !message.isEmpty { Text(message).font(.aeFootnote).textSelection(.enabled) }
        }
        .onAppear {
            reload()
            if let hash = model.agentTransactionHash { command(["receipt", "--hash", hash]) }
        }
        .onChange(of: model.agentTransactionHash) { _, hash in
            if let hash { command(["receipt", "--hash", hash]) }
        }
        .onReceive(NotificationCenter.default.publisher(for: NSApplication.didBecomeActiveNotification)) { _ in reload() }
    }

    private func displayNames(_ item: [String: Any]) -> String {
        let addresses = item["to"] as? [String] ?? []
        let recorded = item["payeeNames"] as? [String] ?? []
        return addresses.enumerated().map { i, address in
            i < recorded.count && recorded[i] != address ? recorded[i] : names[address.lowercased()] ?? address
        }.joined(separator: ", ")
    }

    private func status(_ item: [String: Any]) -> String {
        switch item["status"] as? String {
        case "confirmed": return String(localized: "Confirmed")
        case "failed": return String(localized: "Failed")
        default: return String(localized: "Older record · not confirmed")
        }
    }

    private func date(_ item: [String: Any]) -> String {
        guard let seconds = item["date"] as? Double else { return "" }
        return Date(timeIntervalSince1970: seconds).formatted(date: .abbreviated, time: .shortened)
    }

    private func rows(_ file: String) -> [[String: Any]] {
        #if DEBUG
        if DesignPreview.on { return Self.previewRows[file] ?? [] }
        #endif
        guard let data = try? Data(contentsOf: home.appendingPathComponent(file)),
              let values = try? JSONSerialization.jsonObject(with: data) as? [[String: Any]] else { return [] }
        return values
    }

    #if DEBUG
    /// Design preview: one payee waiting and two past payments.
    static let previewRows: [String: [[String: Any]]] = [
        "payee-requests.json": [["address": "0x4be1c0de00000000000000000000000000009a7f", "purpose": String(localized: "Server hosting"),
                                 "amount": "3", "asset": Brand.networkCoinTicker]],
        "history.json": [
            ["hash": "0x9f2c41d7aa00000000000000000000000000000000000000000000000000beef", "status": "confirmed",
             "amount": "1.5", "asset": Brand.networkCoinTicker, "date": Date().addingTimeInterval(-7_200).timeIntervalSince1970,
             "to": ["0x12ab00000000000000000000000000000000090ab"], "payeeNames": ["Shop"], "purpose": String(localized: "API credits")],
            ["hash": "0x1c0ffee000000000000000000000000000000000000000000000000000000042", "status": "failed",
             "amount": "0.2", "asset": Brand.networkCoinTicker, "date": Date().addingTimeInterval(-90_000).timeIntervalSince1970,
             "to": ["0x12ab00000000000000000000000000000000090ab"], "payeeNames": ["Shop"]],
        ],
        "payees.json": [["address": "0x12ab00000000000000000000000000000000090ab", "name": "Shop"]],
    ]
    #endif

    private func reload() {
        history = rows("history.json")
        requests = rows("payee-requests.json")
        names = Dictionary(uniqueKeysWithValues: rows("payees.json").compactMap { item -> (String, String)? in
            guard let address = item["address"] as? String, let name = item["name"] as? String else { return nil }
            return (address.lowercased(), name)
        })
    }

    private func command(_ args: [String]) {
        working = true
        let helper = Bundle.main.bundleURL.appendingPathComponent("Contents/Helpers/aether-agent")
        Task.detached {
            let result: String
            if FileManager.default.isExecutableFile(atPath: helper.path) {
                let process = Process()
                process.executableURL = helper
                process.arguments = args
                let output = Pipe()
                process.standardOutput = output
                process.standardError = output
                do {
                    try process.run()
                    let data = output.fileHandleForReading.readDataToEndOfFile()
                    process.waitUntilExit()
                    result = String(decoding: data, as: UTF8.self)
                } catch { result = "\(error)" }
            } else { result = String(localized: "This build of the app does not include the agent helper.") }
            await MainActor.run {
                message = result.trimmingCharacters(in: .whitespacesAndNewlines)
                working = false
                reload()
            }
        }
    }
}
#endif
