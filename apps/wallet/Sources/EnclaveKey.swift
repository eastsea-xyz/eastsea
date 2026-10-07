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
        /// Audit 5, A5-7: an unmigrated old Aether key handle is waiting.
        /// Making a fresh key now would give this Mac a second wallet
        /// address and strand the first — finish the data move instead.
        case migrationPending(String)
        var errorDescription: String? {
            switch self {
            case .enclaveUnavailable: return String(localized: "This device has no Secure Enclave to keep the wallet key in.")
            case .keyUnavailable: return String(localized: "The wallet key cannot be opened right now. Unlock this device and try again.")
            case .migrationPending(let why): return why
            }
        }
    }

    static var walletDirectory: URL {
        FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("EastSeaWallet", isDirectory: true)
    }

    /// Legacy preferences must arrive before the model chooses a network or
    /// assigns recovery/history to an account. Also used by direct index loads.
    static func prepareWalletLoad() throws {
        let outcome = DataMigration.ensure()
        if case .waitingForUnlock = outcome { throw AccountStore.Failure.waitingForUnlock }
        if let why = DataMigration.mayCreateFreshWalletKey() { throw KeyError.migrationPending(why) }
    }

    /// Opening an indexed key is deliberately separate from generating one.
    /// Missing, protected, or invalid handles always fail closed.
    static func load(handleURL: URL, requireUserPresence: Bool) throws -> EnclaveAccount {
        #if WALLET_SCREENS
        fatalError("the screens renderer never opens the keychain")
        #endif
        let data: Data
        do { data = try Data(contentsOf: handleURL) }
        catch { throw KeyError.keyUnavailable(error.localizedDescription) }
        #if targetEnvironment(simulator)
        guard let key = try? P256.Signing.PrivateKey(rawRepresentation: data) else {
            throw KeyError.keyUnavailable("unreadable simulator key")
        }
        return EnclaveAccount(key: .software(key), requiresUserPresence: false)
        #else
        guard SecureEnclave.isAvailable else { throw KeyError.enclaveUnavailable }
        do {
            let key = try SecureEnclave.P256.Signing.PrivateKey(dataRepresentation: data)
            return EnclaveAccount(key: .enclave(key), requiresUserPresence: requireUserPresence)
        } catch {
            throw KeyError.keyUnavailable(error.localizedDescription)
        }
        #endif
    }

    static func create(handleURL: URL, requireUserPresence: Bool) throws -> EnclaveAccount {
        #if WALLET_SCREENS
        fatalError("the screens renderer never opens the keychain")
        #endif
        DataMigration.ensure()
        if let why = DataMigration.mayCreateFreshWalletKey() { throw KeyError.migrationPending(why) }
        #if targetEnvironment(simulator)
        let key = P256.Signing.PrivateKey()
        try AccountHandleFile.writeNew(key.rawRepresentation, to: handleURL)
        return EnclaveAccount(key: .software(key), requiresUserPresence: false)
        #else
        guard SecureEnclave.isAvailable else { throw KeyError.enclaveUnavailable }
        var flags: SecAccessControlCreateFlags = [.privateKeyUsage]
        if requireUserPresence { flags.insert(.userPresence) }
        var error: Unmanaged<CFError>?
        guard let access = SecAccessControlCreateWithFlags(nil, kSecAttrAccessibleWhenUnlockedThisDeviceOnly, flags, &error) else {
            throw error!.takeRetainedValue() as Error
        }
        let key = try SecureEnclave.P256.Signing.PrivateKey(accessControl: access)
        // First key only: refuse to replace a handle that appeared meanwhile.
        try AccountHandleFile.writeNew(key.dataRepresentation, to: handleURL)
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

extension AccountStore {
    /// Lazy: constructing the model does not open a key or start migration.
    /// WalletScreens/design previews never call load(). Each numbered handle
    /// uses the same P-256 address derivation and 7702 transaction paths.
    static func wallet() -> AccountStore {
        sharedWallet
    }

    private static let sharedWallet = makeWallet()

    private static func makeWallet() -> AccountStore {
        #if targetEnvironment(simulator)
        let legacy = "simulator-software-key.dat", prefix = "simulator-software-key"
        #else
        let legacy = "enclave-key.dat", prefix = "enclave-key"
        #endif
        return AccountStore(directory: EnclaveAccount.walletDirectory, keys: .init(open: { url in
            do {
                return try accountAddress(p256PublicKey: EnclaveAccount.load(handleURL: url, requireUserPresence: true).publicKey)
            } catch EnclaveAccount.KeyError.keyUnavailable {
                throw Failure.waitingForUnlock
            }
        }, create: { url in
            do {
                return try accountAddress(p256PublicKey: EnclaveAccount.create(handleURL: url, requireUserPresence: true).publicKey)
            } catch {
                if (error as NSError).code == Int(errSecInteractionNotAllowed) { throw Failure.waitingForUnlock }
                throw error
            }
        }), legacyHandleName: legacy, handlePrefix: prefix,
           legacyPayoutAddress: { UserDefaults.standard.string(forKey: "proveAddress") },
           prepare: EnclaveAccount.prepareWalletLoad)
    }
}
