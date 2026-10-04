#if os(macOS)
import AppKit
import Foundation
import SwiftUI

/// The Mac wallet reads only the agent's local display files. Every change is
/// made by its bundled helper, which asks for the owner's Touch ID/password.
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
            HStack {
                Text("AI 비서").font(.aeHeadline)
                Spacer()
                Button("비서 멈추기") { command(["stop"]) }
                    .disabled(working).accessibilityLabel("비서 멈추기")
            }
            Text("멈추기는 Touch ID로 세션을 체인에서 제거합니다. 이미 제출한 거래는 취소되지 않습니다.")
                .font(.aeFootnote).foregroundStyle(.secondary)
            if !requests.isEmpty {
                Text("새 수취인 승인 요청").font(.aeHeadline)
                ForEach(requests.indices, id: \.self) { i in
                    let r = requests[i]
                    let address = r["address"] as? String ?? ""
                    VStack(alignment: .leading, spacing: 5) {
                        Text(address).font(.aeFootnote.monospaced()).textSelection(.enabled)
                        Text("목적: \(r["purpose"] as? String ?? "—")").font(.aeFootnote)
                        Text("요청 금액: \(r["amount"] as? String ?? "—") \(r["asset"] as? String ?? "")").font(.aeFootnote)
                        HStack {
                            TextField("수취인 이름", text: Binding(get: { newNames[address] ?? "" }, set: { newNames[address] = $0 }))
                                .textFieldStyle(.roundedBorder)
                            Button("Touch ID로 허용") {
                                command(["payee", "add", "--name", newNames[address] ?? "", "--address", address])
                            }.disabled(working || (newNames[address] ?? "").trimmingCharacters(in: .whitespaces).isEmpty)
                        }
                    }
                }
            }
            Text("비서 사용 내역").font(.aeHeadline)
            if history.isEmpty { Text("확정된 사용 내역이 없습니다.").font(.aeFootnote).foregroundStyle(.secondary) }
            ForEach(history.indices, id: \.self) { i in
                let item = history[i]
                let hash = item["hash"] as? String ?? ""
                VStack(alignment: .leading, spacing: 4) {
                    Text("\(status(item)) · \(item["amount"] as? String ?? "—") \(item["asset"] as? String ?? Brand.coinTicker)")
                        .font(.aeBody)
                    Text(date(item)).font(.aeFootnote).foregroundStyle(.secondary)
                    Text("받는 사람: \(displayNames(item))").font(.aeFootnote)
                    Text("목적: \(item["purpose"] as? String ?? "기록 없음")").font(.aeFootnote)
                    Button("거래 확인 · \(hash.prefix(12))…") { command(["receipt", "--hash", hash]) }
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
        case "confirmed": return "확정"
        case "failed": return "실패"
        default: return "이전 기록 · 확인 필요"
        }
    }

    private func date(_ item: [String: Any]) -> String {
        guard let seconds = item["date"] as? Double else { return "" }
        return Date(timeIntervalSince1970: seconds).formatted(date: .abbreviated, time: .shortened)
    }

    private func rows(_ file: String) -> [[String: Any]] {
        guard let data = try? Data(contentsOf: home.appendingPathComponent(file)),
              let values = try? JSONSerialization.jsonObject(with: data) as? [[String: Any]] else { return [] }
        return values
    }

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
            } else { result = "이 앱 빌드에는 aether-agent 도우미가 없습니다." }
            await MainActor.run {
                message = result.trimmingCharacters(in: .whitespacesAndNewlines)
                working = false
                reload()
            }
        }
    }
}
#endif
