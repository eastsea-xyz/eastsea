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
    static let policy = dir.appendingPathComponent("policy.json")
    static let ledger = dir.appendingPathComponent("ledger.json")
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

/// Two Secure Enclave keys. The files hold only SE-wrapped blobs: they are
/// useless on any other Mac and cannot be turned back into a private key, so an
/// agent that reads its own files still cannot steal them.
///
/// - agent key: signs the agent's payments without a prompt (spending is bounded
///   by the owner-signed policy and by the account balance).
/// - owner key: requires Touch ID / password for every signature; it signs the
///   spending policy, so only the human can raise limits.
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
        case .notInitialized: "Agent wallet not set up. A human must run `aether-agent init` once (asks for Touch ID)."
        case .policy(let m): "Refused by spending policy: \(m)"
        case .input(let m): "Invalid input: \(m)"
        case .io(let m): m
        }
    }
}
