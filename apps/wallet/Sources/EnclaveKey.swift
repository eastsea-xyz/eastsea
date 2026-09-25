import CryptoKit
import Foundation
import LocalAuthentication

/// The wallet key lives in the Secure Enclave. Only an opaque, device-bound
/// handle (`dataRepresentation`) is written to disk; the private key never leaves the chip.
/// The iOS Simulator has no Secure Enclave: there (and only there) a software
/// P-256 key is used and the UI says so.
struct EnclaveAccount {
    private enum Key {
        case enclave(SecureEnclave.P256.Signing.PrivateKey)
        case software(P256.Signing.PrivateKey)
    }
    private let key: Key
    let requiresUserPresence: Bool

    var isSecureEnclave: Bool {
        if case .enclave = key { return true }
        return false
    }

    enum KeyError: LocalizedError {
        case enclaveUnavailable
        var errorDescription: String? { "Secure Enclave is not available on this device" }
    }

    private static var storeURL: URL {
        let dir = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("AetherWallet", isDirectory: true)
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir.appendingPathComponent("enclave-key.dat")
    }

    static func loadOrCreate(requireUserPresence: Bool) throws -> EnclaveAccount {
        #if targetEnvironment(simulator)
        let url = storeURL.deletingLastPathComponent().appendingPathComponent("simulator-software-key.dat")
        if let data = try? Data(contentsOf: url), let k = try? P256.Signing.PrivateKey(rawRepresentation: data) {
            return EnclaveAccount(key: .software(k), requiresUserPresence: false)
        }
        let k = P256.Signing.PrivateKey()
        try k.rawRepresentation.write(to: url, options: [.atomic, .completeFileProtection])
        return EnclaveAccount(key: .software(k), requiresUserPresence: false)
        #else
        guard SecureEnclave.isAvailable else { throw KeyError.enclaveUnavailable }
        if let data = try? Data(contentsOf: storeURL),
           let key = try? SecureEnclave.P256.Signing.PrivateKey(dataRepresentation: data) {
            return EnclaveAccount(key: .enclave(key), requiresUserPresence: requireUserPresence)
        }
        var flags: SecAccessControlCreateFlags = [.privateKeyUsage]
        if requireUserPresence { flags.insert(.userPresence) }
        var error: Unmanaged<CFError>?
        guard let access = SecAccessControlCreateWithFlags(nil, kSecAttrAccessibleWhenUnlockedThisDeviceOnly, flags, &error) else {
            throw error!.takeRetainedValue() as Error
        }
        let key = try SecureEnclave.P256.Signing.PrivateKey(accessControl: access)
        try key.dataRepresentation.write(to: storeURL, options: [.atomic, .completeFileProtection])
        return EnclaveAccount(key: .enclave(key), requiresUserPresence: requireUserPresence)
        #endif
    }

    /// 33-byte compressed SEC1 public key.
    var publicKey: Data {
        switch key {
        case .enclave(let k): return k.publicKey.compressedRepresentation
        case .software(let k): return k.publicKey.compressedRepresentation
        }
    }

    /// ECDSA over SHA-256(message), raw r‖s (64 bytes). May prompt Touch ID / Face ID / password.
    func sign(_ message: Data) throws -> Data {
        switch key {
        case .enclave(let k): return try k.signature(for: message).rawRepresentation
        case .software(let k): return try k.signature(for: message).rawRepresentation
        }
    }
}
