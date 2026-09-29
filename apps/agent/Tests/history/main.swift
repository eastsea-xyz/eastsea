import Foundation

// Compile with Sources/History.swift; keep this test independent of keys and RPC.
enum Paths {
    static let dir = URL(fileURLWithPath: ProcessInfo.processInfo.environment["AETHER_AGENT_TEST_TMP"]!, isDirectory: true)
        .appendingPathComponent(UUID().uuidString, isDirectory: true)
    static let history = dir.appendingPathComponent("history.json")
    static let pending = dir.appendingPathComponent("pending.json")
    static let payees = dir.appendingPathComponent("payees.json")
    static let payeeRequests = dir.appendingPathComponent("payee-requests.json")
    static func write(_ data: Data, to url: URL) throws { try data.write(to: url) }
}

try FileManager.default.createDirectory(at: Paths.dir, withIntermediateDirectories: true)
let payment = PendingPayment(date: Date(), to: ["0x0000000000000000000000000000000000000001"], totalWei: "100",
                             hash: "0xabc", purpose: "buy a document", payeeNames: ["Bookshop"], asset: "AETH", amount: "0.0000000000000001")
try History.submit(payment)
precondition(History.load().isEmpty, "a submission must not appear as final spending")
precondition(History.pending().count == 1)
try History.finalize(hash: "0xabc", success: true)
precondition(History.pending().isEmpty)
precondition(History.load().first?.status == "confirmed")
precondition(History.load().first?.purpose == "buy a document")
precondition(History.load().first?.payeeNames == ["Bookshop"])
precondition(History.load().first?.txLink == "aether://tx?hash=0xabc")
try History.finalize(hash: "0xabc", success: true)
precondition(History.load().count == 1, "receipt checks must not duplicate a finalized payment")
try History.submit(PendingPayment(date: Date(), to: payment.to, totalWei: "200", hash: "0xdef",
                                  purpose: "retry", payeeNames: ["Bookshop"], asset: "AETH", amount: "0.0000000000000002"))
try History.finalize(hash: "0xdef", success: false)
precondition(History.load().first?.status == "failed")
try Payees.request(address: payment.to[0], purpose: "first purchase", amount: "1", asset: "AETH")
precondition(Payees.requests().count == 1)
precondition(Payees.requests().first?.amount == "1")
try Payees.add(name: "Bookshop", address: payment.to[0])
precondition(Payees.name(payment.to[0]) == "Bookshop")
precondition(Payees.requests().isEmpty)
print("agent history/payee tests passed")
precondition(AgentPolicy.defaultExpires(now: Date(timeIntervalSince1970: 100)) == 100 + 7 * 86_400)
precondition(AgentPolicy.permits("0xAB", allow: ["0xab"]))
precondition(!AgentPolicy.permits("0xCD", allow: ["0xab"]))
print("agent policy tests passed")
