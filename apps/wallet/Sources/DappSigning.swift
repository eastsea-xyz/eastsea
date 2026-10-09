import Foundation
import CoreFoundation

/// Main-actor callers may cancel preparation, but a broadcast must deliver its
/// actual outcome even if its sheet or originating document has disappeared.
final class DappApprovalLifecycle {
    private enum Phase { case reviewing, submitting, finished }
    private var phase = Phase.reviewing
    private let reply: (Result<Any?, ProviderError>) -> Void

    var canCancel: Bool { phase == .reviewing }
    var isSubmitting: Bool { phase == .submitting }

    init(reply: @escaping (Result<Any?, ProviderError>) -> Void) { self.reply = reply }

    /// Called synchronously after signing and before starting the broadcast.
    @discardableResult
    func beginSubmission(stillApproved: Bool) -> Bool {
        guard phase == .reviewing, stillApproved else { return false }
        phase = .submitting
        return true
    }

    @discardableResult
    func cancel(_ error: ProviderError) -> Bool {
        guard canCancel else { return false }
        return finish(.failure(error))
    }

    @discardableResult
    func finish(_ result: Result<Any?, ProviderError>) -> Bool {
        guard phase != .finished else { return false }
        phase = .finished
        reply(result)
        return true
    }
}

/// A pending approval belongs to one account and one network generation.
struct DappRequestContext: Equatable {
    let account: String
    let chainId: UInt64
    let port: UInt16
    let generation: UInt64
    var permissionGeneration: UInt64 = 0

    func matches(account: String, chainId: UInt64, port: UInt16, generation: UInt64, permissionGeneration: UInt64 = 0) -> Bool {
        self.account.lowercased() == account.lowercased() && self.chainId == chainId
            && self.port == port && self.generation == generation && self.permissionGeneration == permissionGeneration
    }
}

struct DappAssetChange: Equatable, Identifiable {
    let contract: String
    let tokenId: String?
    let delta: String
    var id: String { contract + (tokenId ?? "") }
}

struct DappApproval: Equatable, Identifiable {
    let contract: String
    let spender: String
    let amount: String
    let tokenId: String?
    let allTokens: Bool
    var id: String { contract + spender + (tokenId ?? "") }
    var unlimited: Bool { allTokens && amount != "0" || amount == PageTransaction.decimal(fromHex: "0x" + String(repeating: "f", count: 64)) }
}

/// Interprets a node execution result. Amounts never pass through floating point.
struct DappSimulation: Equatable {
    static let transferTopic = "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef"
    static let approvalTopic = "0x8c5be1e5ebec7d5bd14f71427d1e84f3dd0314c0f7b2291e5b200ac8c7c3b925"
    static let approvalForAllTopic = "0x17307eab39ab6107e8899845ad3d59bd9653f200f220920489ca2b5937696c31"
    let success: Bool
    let gasUsed: UInt64
    let failureReason: String?
    let nativeDeltaWei: String
    let changes: [DappAssetChange]
    let approvals: [DappApproval]
    /// Included in approval equality so a changed return value needs review.
    let output: String
    let balancesMeasured: Bool

    func canSign(extraConfirmation: Bool) -> Bool { success || extraConfirmation }

    static func parse(_ raw: Any, account: String) throws -> DappSimulation {
        let bad = ProviderError(code: ProviderErrorCode.internalError, message: String(localized: "The simulation could not be read. Try again."))
        guard let value = raw as? [String: Any],
              let ok = value["success"] as? NSNumber, CFGetTypeID(ok) == CFBooleanGetTypeID(),
              let gas = value["gasUsed"] as? String, gas.hasPrefix("0x"),
              let gasUsed = UInt64(gas.dropFirst(2), radix: 16),
              let output = value["output"] as? String, hex(output),
              let native = value["nativeChanges"] as? [[String: Any]], native.count <= 2048,
              let logs = value["logs"] as? [[String: Any]], logs.count <= 2048 else { throw bad }
        let success = ok.boolValue
        var nativeDelta = "0"
        for entry in native {
            guard let address = entry["address"] as? String, PageTransaction.isAddress(address),
                  let delta = entry["deltaWei"] as? String, signedDecimal(delta) else { throw bad }
            if address.lowercased() == account.lowercased() { nativeDelta = addSigned(nativeDelta, delta) }
        }
        var changes: [String: DappAssetChange] = [:]
        var approvals: [String: DappApproval] = [:]
        let measured = (value["tokenCoverageComplete"] as? Bool) == true
        var measuredTokens: Set<String> = []
        if let addresses = value["measuredTokens"] as? [String] {
            guard addresses.count <= 128, addresses.allSatisfy(PageTransaction.isAddress) else { throw bad }
            measuredTokens = Set(addresses.map { $0.lowercased() })
        }
        if value["tokenChanges"] != nil {
            guard let tokens = value["tokenChanges"] as? [[String: Any]], tokens.count <= 128 else { throw bad }
            for entry in tokens {
                guard let token = entry["token"] as? String, PageTransaction.isAddress(token),
                      let delta = entry["delta"] as? String, signedDecimal(delta) else { throw bad }
                changes[token.lowercased()] = DappAssetChange(contract: token.lowercased(), tokenId: nil, delta: delta)
                measuredTokens.insert(token.lowercased())
            }
        }
        for log in logs {
            guard let address = log["address"] as? String, PageTransaction.isAddress(address),
                  let topics = log["topics"] as? [String], topics.count <= 4,
                  topics.allSatisfy({ hex($0) && $0.count == 66 }),
                  let data = log["data"] as? String, hex(data) else { throw bad }
            guard topics.count >= 3, let owner = topicAddress(topics[1]), let recipient = topicAddress(topics[2]) else { continue }
            let kind = topics[0].lowercased(), contract = address.lowercased()
            let nft = topics.count == 4 ? PageTransaction.decimal(fromHex: topics[3]) : nil
            if kind == transferTopic, owner == account.lowercased() || recipient == account.lowercased() {
                guard nft != nil || data.count == 66 else { continue }
                if measuredTokens.contains(contract) && nft == nil { continue }
                let amount = nft == nil ? PageTransaction.decimal(fromHex: data) : "1"
                let delta = owner == recipient ? "0" : (owner == account.lowercased() ? "-" + amount : amount)
                let key = contract + (nft ?? "")
                changes[key] = DappAssetChange(contract: contract, tokenId: nft, delta: addSigned(changes[key]?.delta ?? "0", delta))
            } else if (kind == approvalTopic || kind == approvalForAllTopic), owner == account.lowercased() {
                guard nft != nil || data.count == 66 else { continue }
                let amount = nft == nil ? PageTransaction.decimal(fromHex: data) : (recipient.dropFirst(2).allSatisfy { $0 == "0" } ? "0" : "1")
                if kind == approvalForAllTopic && amount != "0" && amount != "1" { throw bad }
                let approval = DappApproval(contract: contract, spender: recipient, amount: amount,
                                            tokenId: nft, allTokens: kind == approvalForAllTopic)
                approvals[approval.id] = approval
            }
        }
        // Even a node that returns rolled-back logs cannot imply that effects happened.
        return DappSimulation(success: success, gasUsed: gasUsed,
                              failureReason: success ? nil : (value["failureReason"] as? String ?? String(localized: "The contract refused this transaction.")),
                              nativeDeltaWei: success ? nativeDelta : "0",
                              changes: success ? changes.values.filter { $0.delta != "0" }.sorted { $0.id < $1.id } : [],
                              approvals: success ? approvals.values.sorted { $0.id < $1.id } : [], output: output, balancesMeasured: measured)
    }

    static func hex(_ value: String) -> Bool {
        value.hasPrefix("0x") && value.count % 2 == 0 && value.dropFirst(2).allSatisfy { $0.isASCII && $0.isHexDigit }
    }

    static func signedDecimal(_ value: String) -> Bool {
        let body = value.hasPrefix("-") ? value.dropFirst() : value[...]
        return !body.isEmpty && body.count <= 78 && body.allSatisfy { $0.isASCII && $0.isNumber }
    }

    static func topicAddress(_ value: String) -> String? {
        guard value.count == 66, value.dropFirst(2).prefix(24).allSatisfy({ $0 == "0" }) else { return nil }
        return "0x" + value.suffix(40).lowercased()
    }

    static func addSigned(_ a: String, _ b: String) -> String {
        let an = a.hasPrefix("-"), bn = b.hasPrefix("-")
        let av = an ? String(a.dropFirst()) : a, bv = bn ? String(b.dropFirst()) : b
        if an == bn { let sum = WeiMath.add(av, bv); return sum == "0" ? "0" : (an ? "-" : "") + sum }
        let largerA = WeiMath.compare(av, bv) >= 0
        let result = largerA ? WeiMath.subtract(av, bv) : WeiMath.subtract(bv, av)
        return result == "0" ? "0" : ((largerA ? an : bn) ? "-" : "") + result
    }
}

struct SimulatedPageTransaction: Equatable {
    let transaction: PageTransaction
    let context: DappRequestContext
    let result: DappSimulation
}

struct TypedMessageField: Equatable, Identifiable {
    let path: String
    let value: String
    let detail: String?
    var id: String { path }
}

struct TypedMessageFields: Equatable {
    let primaryType: String
    let domain: [TypedMessageField]
    let message: [TypedMessageField]

    /// The cryptographic core validates the schema first. Render its canonical
    /// payload, including every signed field, rather than the dApp's original text.
    static func parse(_ json: String) throws -> TypedMessageFields {
        let bad = ProviderError(code: ProviderErrorCode.params, message: String(localized: "This message cannot be shown safely. Ask the site to try again."))
        guard json.utf8.count <= 65_536, let data = json.data(using: .utf8),
              let value = try JSONSerialization.jsonObject(with: data) as? [String: Any],
              let primary = value["primaryType"] as? String,
              let domain = value["domain"] as? [String: Any], let message = value["message"] as? [String: Any] else { throw bad }
        let types = value["types"] as? [String: [[String: String]]] ?? [:]
        func flatten(_ object: Any, path: String, type: String?, depth: Int) throws -> [TypedMessageField] {
            guard depth <= 16 else { throw bad }
            if let object = object as? [String: Any] {
                if object.isEmpty { return [.init(path: path, value: String(localized: "No fields"), detail: nil)] }
                var fields: [TypedMessageField] = []
                for key in object.keys.sorted() {
                    let childType = type.flatMap { types[$0]?.first { $0["name"] == key }?["type"] }
                    fields += try flatten(object[key]!, path: path.isEmpty ? key : path + "." + key, type: childType, depth: depth + 1)
                    guard fields.count <= 512 else { throw bad }
                }
                return fields
            }
            if let values = object as? [Any] {
                guard values.count <= 128 else { throw bad }
                if values.isEmpty { return [.init(path: path, value: String(localized: "Empty list"), detail: nil)] }
                let elementType = type.flatMap { name in name.lastIndex(of: "[").map { String(name.prefix(upTo: $0)) } }
                return try values.enumerated().flatMap { try flatten($0.element, path: "\(path)[\($0.offset)]", type: elementType, depth: depth + 1) }
            }
            if let number = object as? NSNumber {
                let bool = CFGetTypeID(number) == CFBooleanGetTypeID()
                return [.init(path: path, value: bool ? (number.boolValue ? String(localized: "Yes") : String(localized: "No")) : number.stringValue, detail: nil)]
            }
            guard let string = object as? String else { throw bad }
            guard !TypedMessageRequest.hasInvisibleCharacters(string) else { throw bad }
            if type?.hasPrefix("bytes") == true, DappSimulation.hex(string) {
                return [.init(path: path, value: String(localized: "\((string.count - 2) / 2) bytes"), detail: string)]
            }
            let numeric = type?.hasPrefix("uint") == true || type?.hasPrefix("int") == true
            let negative = string.hasPrefix("-")
            let magnitude = negative ? String(string.dropFirst()) : string
            let readable = numeric && magnitude.hasPrefix("0x") ? (negative ? "-" : "") + PageTransaction.decimal(fromHex: magnitude) : string
            return [.init(path: path, value: readable, detail: nil)]
        }
        return try .init(primaryType: primary, domain: flatten(domain, path: "", type: "EIP712Domain", depth: 0),
                         message: flatten(message, path: "", type: primary, depth: 0))
    }
}

enum TypedMessageRequest {
    static func hasInvisibleCharacters(_ value: String) -> Bool {
        value.unicodeScalars.contains {
            (0x200b...0x200f).contains($0.value) || (0x202a...0x202e).contains($0.value)
                || (0x2060...0x206f).contains($0.value) || $0.value == 0xfeff || $0.value == 0x00ad
                || ($0.value < 32 && $0.value != 9 && $0.value != 10 && $0.value != 13)
        }
    }
    static func parse(_ params: [Any], context: DappRequestContext) throws -> String {
        guard params.count == 2, let address = params[0] as? String,
              address.lowercased() == context.account.lowercased() else {
            throw ProviderError(code: ProviderErrorCode.locked, message: String(localized: "The message requests a different account."))
        }
        let json: String
        if let text = params[1] as? String { json = text }
        else if let object = params[1] as? [String: Any], let data = try? JSONSerialization.data(withJSONObject: object),
                let text = String(data: data, encoding: .utf8) { json = text }
        else { throw ProviderError(code: ProviderErrorCode.params, message: String(localized: "This message cannot be shown safely. Ask the site to try again.")) }
        guard json.utf8.count <= 65_536, let data = json.data(using: .utf8),
              let value = try JSONSerialization.jsonObject(with: data) as? [String: Any],
              let domain = value["domain"] as? [String: Any],
              let name = domain["name"] as? String, name.count <= 256,
              !name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty, !hasInvisibleCharacters(name) else {
            throw ProviderError(code: ProviderErrorCode.params, message: String(localized: "This message cannot be shown safely. Ask the site to try again."))
        }
        guard case .success(let chain) = PageTransaction.quantity(domain["chainId"], what: "chainId"),
              UInt64(chain) == context.chainId else {
            throw ProviderError(code: 4901, message: String(localized: "The message is for a different network. Nothing was signed."))
        }
        if let verifier = domain["verifyingContract"], !(verifier is String && PageTransaction.isAddress(verifier as! String)) {
            throw ProviderError(code: ProviderErrorCode.params, message: String(localized: "This message cannot be shown safely. Ask the site to try again."))
        }
        return json
    }
}

enum AccountMigration {
    static func validDestination(_ value: String, current: String) -> Bool {
        PageTransaction.isAddress(value) && value.lowercased() != current.lowercased()
            && value.dropFirst(2).contains { $0 != "0" }
    }
}
