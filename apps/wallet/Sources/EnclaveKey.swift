import CryptoKit
import Foundation
import LocalAuthentication

/// The wallet key lives in the Secure Enclave. Only an opaque, device-bound
/// handle (`dataRepresentation`) is written to disk; the private key never leaves the chip.
struct EnclaveAccount {
    let key: SecureEnclave.P256.Signing.PrivateKey
    let requiresUserPresence: Bool

    enum KeyError: LocalizedError {
        case enclaveUnavailable
        var errorDescription: String? { "Secure Enclave is not available on this Mac" }
    }

    private static var storeURL: URL {
        let dir = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("AetherWallet", isDirectory: true)
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir.appendingPathComponent("enclave-key.dat")
    }

    static func loadOrCreate(requireUserPresence: Bool) throws -> EnclaveAccount {
        guard SecureEnclave.isAvailable else { throw KeyError.enclaveUnavailable }
        if let data = try? Data(contentsOf: storeURL),
           let key = try? SecureEnclave.P256.Signing.PrivateKey(dataRepresentation: data) {
            return EnclaveAccount(key: key, requiresUserPresence: requireUserPresence)
        }
        var flags: SecAccessControlCreateFlags = [.privateKeyUsage]
        if requireUserPresence { flags.insert(.userPresence) }
        var error: Unmanaged<CFError>?
        guard let access = SecAccessControlCreateWithFlags(nil, kSecAttrAccessibleWhenUnlockedThisDeviceOnly, flags, &error) else {
            throw error!.takeRetainedValue() as Error
        }
        let key = try SecureEnclave.P256.Signing.PrivateKey(accessControl: access)
        try key.dataRepresentation.write(to: storeURL, options: [.atomic, .completeFileProtection])
        return EnclaveAccount(key: key, requiresUserPresence: requireUserPresence)
    }

    /// 33-byte compressed SEC1 public key.
    var publicKey: Data { key.publicKey.compressedRepresentation }

    /// ECDSA over SHA-256(message), raw r‖s (64 bytes). May prompt Touch ID / password.
    func sign(_ message: Data) throws -> Data {
        try key.signature(for: message).rawRepresentation
    }
}
