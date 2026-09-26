import CryptoKit
import Foundation
import Security

/// Where the agent keeps its (Secure Enclave-wrapped) keys, policy and ledger.
enum Paths {
    static let dir: URL = {
        if let d = ProcessInfo.processInfo.environment["AETHER_AGENT_HOME"], !d.isEmpty { return URL(fileURLWithPath: d, isDirectory: true) }
        return FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Application Support/Aether/agent", isDirectory: true)
    }()
    static let agentKey = dir.appendingPathComponent("agent.key")
    static let ownerKey = dir.appendingPathComponent("owner.key")
    static let history = dir.appendingPathComponent("history.json")
    static let network = dir.appendingPathComponent("network.json")

    static func ensure() throws {
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
    }

    static func write(_ data: Data, to url: URL) throws {
        try ensure()
        try data.write(to: url, options: .atomic)
        try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: url.path)
    }
}

/// Two Secure Enclave keys. The files hold only SE-wrapped blobs: useless on any
/// other Mac and never exportable.
///
/// - owner key: the agent account's own key. Every signature needs Touch ID or
///   the login password, so only the human can change the account (limits,
///   recipients, recovery, moving funds out).
/// - agent key: a session key of that account. It signs payments without a
///   prompt, and the account contract caps them (per payment, per 24 h,
///   recipients, expiry). Its own address holds the gas money, which caps gas.
///
/// Swapping key files cannot raise the limits: they are on chain, set by a
/// transaction only the owner key can sign.
enum Keys {
    static func agent(create: Bool = false) throws -> SecureEnclave.P256.Signing.PrivateKey {
        if let blob = try? Data(contentsOf: Paths.agentKey) {
            return try SecureEnclave.P256.Signing.PrivateKey(dataRepresentation: blob)
        }
        guard create else { throw AgentError.notInitialized }
        let k = try SecureEnclave.P256.Signing.PrivateKey()
        try Paths.write(k.dataRepresentation, to: Paths.agentKey)
        return k
    }

    static func owner(create: Bool = false) throws -> SecureEnclave.P256.Signing.PrivateKey {
        if let blob = try? Data(contentsOf: Paths.ownerKey) {
            return try SecureEnclave.P256.Signing.PrivateKey(dataRepresentation: blob)
        }
        guard create else { throw AgentError.notInitialized }
        var err: Unmanaged<CFError>?
        guard let ac = SecAccessControlCreateWithFlags(nil, kSecAttrAccessibleWhenUnlockedThisDeviceOnly, [.privateKeyUsage, .userPresence], &err) else {
            throw AgentError.io("access control: \(String(describing: err?.takeRetainedValue()))")
        }
        let k = try SecureEnclave.P256.Signing.PrivateKey(accessControl: ac)
        try Paths.write(k.dataRepresentation, to: Paths.ownerKey)
        return k
    }

    /// Uncompressed SEC1 public key (65 bytes), as the Rust core expects.
    static func publicKey(_ k: SecureEnclave.P256.Signing.PrivateKey) -> Data {
        k.publicKey.x963Representation
    }
}

enum AgentError: Error, CustomStringConvertible {
    case notInitialized
    case policy(String)
    case input(String)
    case io(String)

    var description: String {
        switch self {
        case .notInitialized: "Agent wallet not set up. A human must run `aether-agent init` once."
        case .policy(let m): "Refused by the account's spending limits: \(m)"
        case .input(let m): "Invalid input: \(m)"
        case .io(let m): m
        }
    }
}
