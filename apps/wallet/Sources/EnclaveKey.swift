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
        /// The key's handle exists but cannot be read or restored right now
        /// (device locked, a Keychain hiccup after an OS update). Never a
        /// reason to make a new key: that would orphan the wallet's address.
        case keyUnavailable(String)
        var errorDescription: String? {
            switch self {
            case .enclaveUnavailable: return "Secure Enclave is not available on this device"
            case .keyUnavailable(let why): return "The wallet key cannot be opened right now (\(why)). Unlock this device and try again."
            }
        }
    }

    private static var storeURL: URL {
        DataMigration.ensure()
        let dir = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("EastSeaWallet", isDirectory: true)
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir.appendingPathComponent("enclave-key.dat")
    }

    static func loadOrCreate(requireUserPresence: Bool) throws -> EnclaveAccount {
        #if targetEnvironment(simulator)
        let url = storeURL.deletingLastPathComponent().appendingPathComponent("simulator-software-key.dat")
        if FileManager.default.fileExists(atPath: url.path) {
            guard let data = try? Data(contentsOf: url), let k = try? P256.Signing.PrivateKey(rawRepresentation: data) else {
                throw KeyError.keyUnavailable("unreadable simulator key")
            }
            return EnclaveAccount(key: .software(k), requiresUserPresence: false)
        }
        let k = P256.Signing.PrivateKey()
        try k.rawRepresentation.write(to: url, options: [.withoutOverwriting, .completeFileProtection])
        return EnclaveAccount(key: .software(k), requiresUserPresence: false)
        #else
        guard SecureEnclave.isAvailable else { throw KeyError.enclaveUnavailable }
        // An existing handle is the wallet: if it cannot be read or restored
        // now, fail and let the caller retry — never fall through to making a
        // new key, which would overwrite the handle and orphan the address
        // (red team 2026-09-29, self-healing review #8).
        if FileManager.default.fileExists(atPath: storeURL.path) {
            let data: Data
            do { data = try Data(contentsOf: storeURL) } catch { throw KeyError.keyUnavailable(error.localizedDescription) }
            do {
                let key = try SecureEnclave.P256.Signing.PrivateKey(dataRepresentation: data)
                return EnclaveAccount(key: .enclave(key), requiresUserPresence: requireUserPresence)
            } catch {
                throw KeyError.keyUnavailable(error.localizedDescription)
            }
        }
        var flags: SecAccessControlCreateFlags = [.privateKeyUsage]
        if requireUserPresence { flags.insert(.userPresence) }
        var error: Unmanaged<CFError>?
        guard let access = SecAccessControlCreateWithFlags(nil, kSecAttrAccessibleWhenUnlockedThisDeviceOnly, flags, &error) else {
            throw error!.takeRetainedValue() as Error
        }
        let key = try SecureEnclave.P256.Signing.PrivateKey(accessControl: access)
        // First key only: refuse to replace a handle that appeared meanwhile.
        try key.dataRepresentation.write(to: storeURL, options: [.withoutOverwriting, .completeFileProtection])
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
