// The shell harness inserts WalletModel's actual approveCall, reply, and track
// methods here. All signing, network, tracking, and browser dependencies are
// fakes: this test cannot touch a real key, node, wallet, or browser.
import Foundation

struct CallRequest {
    let to = ""
    let value = "0"
    let data = "0x"
    let gas: UInt64 = 21_000
    let callback: URL?
}

struct FakeEnclave {
    let publicKey = "fixture-public-key"
    func sign(_ message: String) throws -> String { "fixture-signature" }
}

struct PreparedCall {
    let signingMessage = "fixture-message"
    let envelopeJson = "{}"
}

enum Wei {
    static func from(aeth: String) -> String? { "0" }
}

enum Short {
    static func address(_ address: String) -> String { address }
}

struct ActivityItem {
    enum Kind { case sent }
    init(kind: Kind, title: String, amount: Double?, token: String?) {}
}

struct TokenCatalog {
    let tokens: [String: Bool] = [:]
}

func prepareCall(p256PublicKey: String, to: String, valueWei: String,
                 dataHex: String, gasLimit: UInt64) throws -> PreparedCall {
    PreparedCall()
}

let fixtureHash = "0x" + String(repeating: "a", count: 64)
func submitSigned(envelopeJson: String, signature: String, p256PublicKey: String) throws -> String {
    fixtureHash
}

// approveCall delivers replies on MainActor. The test also reads this capture
// on MainActor, with no real NSWorkspace or external URL opening involved.
final class NSWorkspace: @unchecked Sendable {
    static let shared = NSWorkspace()
    var opened: [URL] = []
    @discardableResult func open(_ url: URL) -> Bool {
        opened.append(url)
        return true
    }
}

final class WalletModel: @unchecked Sendable {
    let enclave: FakeEnclave? = FakeEnclave()
    var callRequest: CallRequest?
    let tokenCatalog = TokenCatalog()
    var busy = false
    let outcome: TxTrack.Row

    init(outcome: TxTrack.Row, callback: URL) {
        self.outcome = outcome
        self.callRequest = CallRequest(callback: callback)
    }

    func note(_ message: String) {}
    private func follow(_ hash: String, label: String, item: ActivityItem) async -> TxTrack.Row {
        outcome
    }

    // INSERT_APPROVE_CALL
    // INSERT_REPLY
    // INSERT_TRACK
}

@main enum CallCallbackRegression {
    @MainActor static func main() async {
        let callback = URL(string: "https://example.invalid/callback?request=original")!
        let cases: [(TxTrack.Row, String)] = [
            (.notIncluded, "not_included"), (.pending, "pending"),
            (.done, "success"), (.failed, "failed"),
        ]
        for (outcome, expectedStatus) in cases {
            NSWorkspace.shared.opened = []
            let wallet = WalletModel(outcome: outcome, callback: callback)
            wallet.approveCall()
            for _ in 0..<200 {
                if !NSWorkspace.shared.opened.isEmpty { break }
                try? await Task.sleep(nanoseconds: 5_000_000)
            }
            guard let url = NSWorkspace.shared.opened.first,
                  let query = URLComponents(url: url, resolvingAgainstBaseURL: false)?.queryItems else {
                fail("contract call \(outcome.rawValue) produced no callback")
            }
            let status = query.first { $0.name == "status" }?.value
            guard status == expectedStatus else {
                fail("contract call \(outcome.rawValue) callback status \(status ?? "missing"); expected \(expectedStatus)")
            }
            guard query.first(where: { $0.name == "tx" })?.value == fixtureHash,
                  query.first(where: { $0.name == "request" })?.value == "original" else {
                fail("contract call callback lost its hash or original request")
            }
            print("ok   actual approveCall callback preserves \(outcome.rawValue)")
        }
        print("call-callback: all ok")
    }

    static func fail(_ message: String) -> Never {
        print("FAIL \(message)")
        exit(1)
    }
}
