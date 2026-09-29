import CryptoKit
import Foundation
import Security

/// Where the signing Mac keeps the registrar key.
///
/// Only the Secure Enclave handle (`dataRepresentation`) is written: the private
/// key itself never leaves the chip, cannot be exported and is useless on any
/// other Mac. The node's data directory holds no registrar key at all when the
/// node runs with `--registrar-signer` (docs/ops/registrar.md).
enum Paths {
    static let dir: URL = {
        if let d = ProcessInfo.processInfo.environment["AETHER_REGISTRAR_HOME"], !d.isEmpty {
            return URL(fileURLWithPath: d, isDirectory: true)
        }
        return FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent("Library/Application Support/Aether/registrar", isDirectory: true)
    }()
    static let key = dir.appendingPathComponent("enclave.key")
    static let socket = dir.appendingPathComponent("signer.sock")

    static func ensure() throws {
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
    }
}

enum SignerError: Error, CustomStringConvertible {
    case notInitialized
    case alreadyInitialized(String)
    case input(String)
    case io(String)

    var description: String {
        switch self {
        case .notInitialized:
            "no registrar key on this Mac: the owner runs `aether-registrar-signer init` once, behind Touch ID"
        case .alreadyInitialized(let path):
            "a registrar key already exists (\(path)): moving the registrar to a new key is a committee-signed upgrade, not a local operation. Delete the file only if this key was never put on chain."
        case .input(let m): m
        case .io(let m): m
        }
    }
}

/// The registrar's P-256 key, in the Secure Enclave of this Mac.
enum Key {
    /// The key, or an error that tells the operator what to run.
    static func load() throws -> SecureEnclave.P256.Signing.PrivateKey {
        guard let blob = try? Data(contentsOf: Paths.key) else { throw SignerError.notInitialized }
        do {
            return try SecureEnclave.P256.Signing.PrivateKey(dataRepresentation: blob)
        } catch {
            throw SignerError.io("\(Paths.key.path) does not open in this Mac's Secure Enclave: \(error)")
        }
    }

    /// Create the key.
    ///
    /// Without `touchID` the key is created `AfterFirstUnlock`: it keeps signing
    /// while nobody is at the screen (the node re-attests every period, and a
    /// signing Mac restarts without a human in front of it), and it stops signing
    /// after a reboot until someone logs in once. `WhenUnlocked` is
    /// refused by the Secure Enclave whenever the display is asleep —
    /// errSecInteractionNotAllowed, the error the wallet shows as "Unlock this
    /// device" — which a timer-driven registrar cannot live with.
    ///
    /// With `touchID` every signature asks for the user's presence
    /// (`.userPresence` needs an unlocked Mac anyway, so `WhenUnlocked` is the
    /// stricter of the two and is what is used).
    @discardableResult
    static func create(touchID: Bool) throws -> SecureEnclave.P256.Signing.PrivateKey {
        if FileManager.default.fileExists(atPath: Paths.key.path) {
            throw SignerError.alreadyInitialized(Paths.key.path)
        }
        var err: Unmanaged<CFError>?
        var flags: SecAccessControlCreateFlags = [.privateKeyUsage]
        if touchID { flags.insert(.userPresence) }
        let accessible: CFString = touchID ? kSecAttrAccessibleWhenUnlockedThisDeviceOnly : kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
        guard let ac = SecAccessControlCreateWithFlags(nil, accessible, flags, &err) else {
            throw SignerError.io("access control: \(String(describing: err?.takeRetainedValue()))")
        }
        let k = try SecureEnclave.P256.Signing.PrivateKey(accessControl: ac)
        try Paths.ensure()
        try k.dataRepresentation.write(to: Paths.key, options: .atomic)
        try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: Paths.key.path)
        return k
    }

    /// Uncompressed SEC1 public key (65 bytes, `04‖x‖y`), as Rust expects.
    static func sec1(_ k: SecureEnclave.P256.Signing.PrivateKey) -> Data {
        k.publicKey.x963Representation
    }

    /// x‖y hex (lowercase): what the registry holds in slots 0 and 1, and what
    /// goes into `aether network --registrar`.
    static func publicHex(_ k: SecureEnclave.P256.Signing.PrivateKey) -> String {
        sec1(k).dropFirst().map { String(format: "%02x", $0) }.joined()
    }

    /// Sign an attestation: ECDSA P-256 over SHA-256(message), low-s, raw r‖s —
    /// the convention of P256VERIFY (EIP-7951) and `aether_crypto`.
    ///
    /// The Secure Enclave does not normalize: `signature(for:)` returns whichever
    /// of s and n−s it happened to compute, about half the time the high one, and
    /// the node and the chain both reject high-s (a malleable signature, and
    /// `aether_crypto` returns `CryptoError::HighS`). So s is flipped here — both
    /// forms verify under the public key, and the registry only ever sees low-s.
    static func sign(_ k: SecureEnclave.P256.Signing.PrivateKey, message: Data) throws -> Data {
        var raw = Array(try k.signature(for: message).rawRepresentation)
        guard raw.count == 64 else { throw SignerError.io("the Secure Enclave returned \(raw.count) signature bytes, not 64") }
        let s = Array(raw[32...])
        if isGreater(s, halfOrder) {
            raw.replaceSubrange(32..., with: subtract(order, s))
        }
        return Data(raw)
    }

    /// The order of the P-256 group, n.
    private static let order: [UInt8] = [
        0xFF, 0xFF, 0xFF, 0xFF, 0x00, 0x00, 0x00, 0x00,
        0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
        0xBC, 0xE6, 0xFA, 0xAD, 0xA7, 0x17, 0x9E, 0x84,
        0xF3, 0xB9, 0xCA, 0xC2, 0xFC, 0x63, 0x25, 0x51,
    ]

    /// n/2, the low-s boundary: s above it is the malleable twin of n−s.
    private static let halfOrder: [UInt8] = {
        var out = order
        var carry: UInt8 = 0
        for i in 0..<out.count {
            let bit = out[i] & 0x01
            out[i] = (out[i] >> 1) | (carry << 7)
            carry = bit
        }
        return out
    }()

    /// Big-endian `a > b`, same length.
    private static func isGreater(_ a: [UInt8], _ b: [UInt8]) -> Bool {
        for (x, y) in zip(a, b) where x != y { return x > y }
        return false
    }

    /// Big-endian `a − b`, same length, `a > b`.
    private static func subtract(_ a: [UInt8], _ b: [UInt8]) -> [UInt8] {
        var out = [UInt8](repeating: 0, count: a.count)
        var borrow = 0
        for i in (0..<a.count).reversed() {
            let d = Int(a[i]) - Int(b[i]) - borrow
            out[i] = UInt8(truncatingIfNeeded: d)
            borrow = d < 0 ? 1 : 0
        }
        return out
    }
}
