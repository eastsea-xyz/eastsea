import Foundation
import Combine
import Darwin

/// Non-secret metadata. The id is the permanent handle number, not the row's
/// position; reordering and deleting rows never reuse a Secure Enclave handle.
struct WalletAccount: Codable, Equatable, Identifiable {
    let id: Int
    let address: String
    var name: String
    var color: String
    var isHidden: Bool = false
}

/// One serialized owner of the wallet's account index. KeyAccess is the only
/// boundary to Secure Enclave/FFI; tests use opaque fixture bytes instead.
/// Nothing in accounts.json can sign a transaction.
@MainActor
final class AccountStore: ObservableObject {
    struct KeyAccess {
        let open: (URL) throws -> String
        let create: (URL) throws -> String
    }

    struct Balances {
        let nativeWei: String
        let tokenBalances: [String]
        /// Unknown/malformed readings fail closed, including an empty string.
        var isZero: Bool {
            ([nativeWei] + tokenBalances).allSatisfy { !$0.isEmpty && $0.allSatisfy { $0 == "0" } }
        }
    }

    enum State: Equatable { case unloaded, ready, waitingForUnlock, failed(String) }
    enum Checkpoint: Equatable { case handleStaged, handleWritten, indexWritten }
    enum Failure: Error, LocalizedError {
        case waitingForUnlock, invalidIndex, handleMismatch, accountNotFound
        case invalidOrder, invalidName, invalidAddress, balanceNotZero, balanceUnavailable
        case lastAccount, payoutAccount, operationInProgress
        var errorDescription: String? {
            switch self {
            case .waitingForUnlock: return String(localized: "Unlock this device to open your wallet. Your wallet is safe.")
            case .invalidIndex: return String(localized: "The account index cannot be read. The wallet keys have been kept.")
            case .handleMismatch: return String(localized: "The wallet handle does not match its account. The keys have been kept.")
            case .accountNotFound: return String(localized: "This account is not available.")
            case .invalidOrder: return String(localized: "The order must include each visible account exactly once.")
            case .invalidName: return String(localized: "Give this account a name.")
            case .invalidAddress: return String(localized: "Choose a valid payout address.")
            case .balanceNotZero: return String(localized: "Move all funds before removing or hiding this account.")
            case .balanceUnavailable: return String(localized: "The balance could not be checked. The account has been kept.")
            case .lastAccount: return String(localized: "Keep at least one visible account.")
            case .payoutAccount: return String(localized: "Choose another payout account before removing this one.")
            case .operationInProgress: return String(localized: "Finish the pending wallet operation before changing accounts.")
            }
        }
    }

    private struct Index: Codable {
        var version = 1
        var accounts: [WalletAccount]
        var activeID: Int
        var payoutAddress: String
        var nextID: Int
        /// Retired handle numbers stay recoverable but cannot be rediscovered
        /// as newly created accounts or reused for another address.
        var deletedIDs: [Int] = []
    }

    @Published private(set) var accounts: [WalletAccount] = []
    @Published private(set) var retiredAccountIDs: [Int] = []
    @Published private(set) var activeAccount: WalletAccount?
    @Published private(set) var payoutAddress = ""
    @Published private(set) var state = State.unloaded
    // @Published sends in willSet. Consumers loading keys need the entire
    // committed index, and an unlocked file lock, before they are notified.
    private let activeSubject = CurrentValueSubject<WalletAccount?, Never>(nil)
    private let payoutSubject = CurrentValueSubject<String, Never>("")
    var activeAccountPublisher: AnyPublisher<WalletAccount?, Never> { activeSubject.eraseToAnyPublisher() }
    var payoutAddressPublisher: AnyPublisher<String, Never> { payoutSubject.eraseToAnyPublisher() }

    /// The wallet installs this gate so sheets/signatures cannot change
    /// account underneath an approval. Pure tests do not need a UI.
    var canChangeAccount: () -> Bool = { true }
    /// Must fetch balances afresh; missing providers never authorize deletion.
    var readBalances: (WalletAccount) throws -> Balances
    let directory: URL
    private let keys: KeyAccess
    private let legacyHandleName: String
    private let handlePrefix: String
    private let legacyPayoutAddress: () -> String?
    private let prepare: () throws -> Void
    private let checkpoint: (Checkpoint) throws -> Void
    private var indexURL: URL { directory.appendingPathComponent("accounts.json") }

    init(directory: URL, keys: KeyAccess, legacyHandleName: String = "enclave-key.dat",
         handlePrefix: String = "enclave-key", legacyPayoutAddress: @escaping () -> String? = { nil },
         prepare: @escaping () throws -> Void = {},
         balances: @escaping (WalletAccount) throws -> Balances = { _ in throw Failure.balanceUnavailable },
         checkpoint: @escaping (Checkpoint) throws -> Void = { _ in }) {
        self.directory = directory
        self.keys = keys
        self.legacyHandleName = legacyHandleName
        self.handlePrefix = handlePrefix
        self.legacyPayoutAddress = legacyPayoutAddress
        self.prepare = prepare
        self.readBalances = balances
        self.checkpoint = checkpoint
    }

    func list() -> [WalletAccount] { accounts.filter { !$0.isHidden } }
    func handleURL(for id: Int) -> URL { directory.appendingPathComponent("\(handlePrefix)-\(id).dat") }

    /// Retryable after lock or termination. The legacy handle is copied,
    /// compared byte-for-byte, and retained even after the index is durable.
    /// A missing/corrupt indexed key never falls through to key generation.
    func load() throws {
        do {
            try prepare()
            let loaded = try withLock {
                var index: Index
                if FileManager.default.fileExists(atPath: indexURL.path) {
                    index = try readIndex()
                } else {
                    let old = directory.appendingPathComponent(legacyHandleName)
                    if FileManager.default.fileExists(atPath: old.path) {
                        let bytes = try Data(contentsOf: old)
                        let new = handleURL(for: 1)
                        if FileManager.default.fileExists(atPath: new.path) {
                            guard try Data(contentsOf: new) == bytes else { throw Failure.handleMismatch }
                        } else {
                            try AccountHandleFile.writeNew(bytes, to: new) { try self.checkpoint(.handleStaged) }
                            try checkpoint(.handleWritten)
                        }
                    }
                    let found = try handleIDs()
                    if found.isEmpty {
                        _ = try keys.create(handleURL(for: 1))
                        try syncFile(handleURL(for: 1))
                        try syncDirectory()
                        try checkpoint(.handleWritten)
                    }
                    let ids = try handleIDs()
                    guard let firstID = ids.first else { throw Failure.invalidIndex }
                    let recovered = try ids.map { try record($0) }
                    let first = recovered.first!
                    let payout = legacyPayoutAddress().flatMap { Self.validAddress($0) ? $0.lowercased() : nil } ?? first.address
                    index = Index(accounts: recovered, activeID: firstID, payoutAddress: payout, nextID: (ids.last ?? 0) + 1)
                    try writeIndex(index)
                }
                // Creating a key and committing its index cannot be one
                // filesystem operation. Recover any unindexed handle left
                // at that boundary rather than generating its replacement.
                let known = Set(index.accounts.map(\.id) + index.deletedIDs)
                let orphanIDs = try handleIDs().filter { !known.contains($0) }
                if !orphanIDs.isEmpty {
                    index.accounts += try orphanIDs.map { try record($0) }
                    index.nextID = max(index.nextID, (orphanIDs.max() ?? 0) + 1)
                    try writeIndex(index)
                }
                try verifyActive(index)
                return index
            }
            state = .ready
            publish(loaded)
        } catch {
            if Self.isLocked(error) { state = .waitingForUnlock; throw Failure.waitingForUnlock }
            state = .failed(error.localizedDescription)
            throw error
        }
    }

    @discardableResult
    func create(name: String? = nil, color: String = "violet") throws -> WalletAccount {
        try requireChange()
        let requestedName = try name.map { try cleanName($0) }
        var created: WalletAccount!
        try mutate { index in
            let id = index.nextID
            guard !FileManager.default.fileExists(atPath: handleURL(for: id).path) else { throw Failure.invalidIndex }
            let addr = try keys.create(handleURL(for: id)).lowercased()
            guard Self.validAddress(addr) else { throw Failure.invalidAddress }
            try syncFile(handleURL(for: id))
            try syncDirectory()
            try checkpoint(.handleWritten)
            created = WalletAccount(id: id, address: addr, name: requestedName ?? String(localized: "Account \(id)"), color: color)
            index.accounts.append(created)
            index.nextID += 1
        }
        return created
    }

    func rename(_ id: Int, name: String) throws {
        let clean = try cleanName(name)
        try mutate { index in
            guard let row = index.accounts.firstIndex(where: { $0.id == id }) else { throw Failure.accountNotFound }
            index.accounts[row].name = clean
        }
    }

    func setColor(_ id: Int, color: String) throws {
        try mutate { index in
            guard let row = index.accounts.firstIndex(where: { $0.id == id }) else { throw Failure.accountNotFound }
            index.accounts[row].color = color
        }
    }

    func reorder(_ ids: [Int]) throws {
        try mutate { index in
            let visible = index.accounts.filter { !$0.isHidden }
            guard ids.count == visible.count, Set(ids) == Set(visible.map(\.id)) else { throw Failure.invalidOrder }
            index.accounts = ids.map { id in visible.first { $0.id == id }! } + index.accounts.filter(\.isHidden)
        }
    }

    func select(_ id: Int) throws {
        try requireChange()
        try mutate { index in
            guard let row = index.accounts.first(where: { $0.id == id && !$0.isHidden }) else { throw Failure.accountNotFound }
            try verify(row)
            index.activeID = id
        }
    }

    func setPayoutAccount(_ id: Int) throws {
        try mutate { index in
            guard let row = index.accounts.first(where: { $0.id == id && !$0.isHidden }) else { throw Failure.accountNotFound }
            try verify(row)
            index.payoutAddress = row.address
        }
    }

    /// An existing explicit node payout (which may be an external wallet) is
    /// preserved during migration. Subsequent changes use this explicit API.
    func setPayoutAddress(_ address: String) throws {
        guard Self.validAddress(address) else { throw Failure.invalidAddress }
        try mutate { $0.payoutAddress = address.lowercased() }
    }

    func hide(_ id: Int) throws {
        try requireChange()
        try mutate { index in
            let row = try removalRow(id, in: index)
            try verify(index.accounts[row])
            try requireZero(index.accounts[row])
            index.accounts[row].isHidden = true
            if index.activeID == id { index.activeID = index.accounts.first { !$0.isHidden }!.id }
        }
    }

    func unhide(_ id: Int) throws {
        try mutate { index in
            guard let row = index.accounts.firstIndex(where: { $0.id == id }) else { throw Failure.accountNotFound }
            try verify(index.accounts[row])
            index.accounts[row].isHidden = false
        }
    }

    /// A fresh zero check permits retiring the account from the list. Its
    /// handle is retained: zero on the currently reachable chain cannot prove
    /// zero on every chain/token this address could hold. A Secure Enclave
    /// key cannot be exported, so retirement must remain recoverable.
    func delete(_ id: Int) throws {
        try requireChange()
        let updated = try withLock {
            var index = try readIndex()
            let row = try removalRow(id, in: index)
            try verify(index.accounts[row])
            try requireZero(index.accounts[row])
            index.accounts.remove(at: row)
            index.deletedIDs.append(id)
            if index.activeID == id { index.activeID = index.accounts.first { !$0.isHidden }!.id }
            try writeIndex(index)
            return index
        }
        publish(updated)
    }

    func restoreDeleted(_ id: Int) throws {
        try mutate { index in
            guard index.deletedIDs.contains(id) else { throw Failure.accountNotFound }
            index.accounts.append(try record(id))
            index.deletedIDs.removeAll { $0 == id }
        }
    }

    private func removalRow(_ id: Int, in index: Index) throws -> Int {
        guard let row = index.accounts.firstIndex(where: { $0.id == id }) else { throw Failure.accountNotFound }
        if !index.accounts[row].isHidden && index.accounts.filter({ !$0.isHidden }).count <= 1 { throw Failure.lastAccount }
        guard index.accounts[row].address != index.payoutAddress else { throw Failure.payoutAccount }
        return row
    }
    private func requireZero(_ account: WalletAccount) throws {
        let balances: Balances
        do { balances = try readBalances(account) }
        catch { throw Failure.balanceUnavailable }
        guard balances.isZero else { throw Failure.balanceNotZero }
    }
    private func requireChange() throws {
        guard canChangeAccount() else { throw Failure.operationInProgress }
    }
    private func cleanName(_ name: String) throws -> String {
        let clean = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !clean.isEmpty else { throw Failure.invalidName }
        return clean
    }
    private func record(_ id: Int) throws -> WalletAccount {
        let addr = try keys.open(handleURL(for: id)).lowercased()
        guard Self.validAddress(addr) else { throw Failure.invalidAddress }
        return WalletAccount(id: id, address: addr, name: String(localized: "Account \(id)"), color: "violet")
    }
    private func verify(_ account: WalletAccount) throws {
        guard try keys.open(handleURL(for: account.id)).lowercased() == account.address else { throw Failure.handleMismatch }
    }
    private func verifyActive(_ index: Index) throws {
        guard let active = index.accounts.first(where: { $0.id == index.activeID && !$0.isHidden }) else { throw Failure.invalidIndex }
        try verify(active)
    }
    private func publish(_ index: Index) {
        accounts = index.accounts
        retiredAccountIDs = index.deletedIDs
        payoutAddress = index.payoutAddress
        activeAccount = index.accounts.first { $0.id == index.activeID }
        activeSubject.send(activeAccount)
        payoutSubject.send(payoutAddress)
    }
    private func mutate(_ body: (inout Index) throws -> Void) throws {
        guard state == .ready else { throw Failure.accountNotFound }
        let updated = try withLock {
            var index = try readIndex()
            try body(&index)
            try writeIndex(index)
            return index
        }
        publish(updated)
    }

    private func readIndex() throws -> Index {
        let data = try Data(contentsOf: indexURL)
        guard let index = try? JSONDecoder().decode(Index.self, from: data), index.version == 1,
              !index.accounts.isEmpty, index.accounts.contains(where: { $0.id == index.activeID && !$0.isHidden }),
              Set(index.accounts.map(\.id)).count == index.accounts.count,
              Set(index.accounts.map(\.address)).count == index.accounts.count,
              index.accounts.allSatisfy({ $0.id > 0 && Self.validAddress($0.address) && $0.address == $0.address.lowercased() }),
              index.deletedIDs.allSatisfy({ $0 > 0 }), Set(index.deletedIDs).isDisjoint(with: index.accounts.map(\.id)),
              index.nextID > ((index.accounts.map(\.id) + index.deletedIDs).max() ?? 0), index.nextID < Int.max,
              Self.validAddress(index.payoutAddress) else { throw Failure.invalidIndex }
        // An indexed handle disappearing is an error, never a fresh install.
        guard index.accounts.allSatisfy({ FileManager.default.fileExists(atPath: handleURL(for: $0.id).path) }) else {
            throw Failure.handleMismatch
        }
        return index
    }

    private func handleIDs() throws -> [Int] {
        let prefix = handlePrefix + "-"
        return try FileManager.default.contentsOfDirectory(atPath: directory.path).compactMap { name in
            guard name.hasPrefix(prefix), name.hasSuffix(".dat"),
                  let n = Int(name.dropFirst(prefix.count).dropLast(4)), n > 0, n < Int.max - 1,
                  name == "\(prefix)\(n).dat" else { return nil }
            return n
        }.sorted()
    }

    private func writeIndex(_ index: Index) throws {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys, .prettyPrinted]
        let pending = directory.appendingPathComponent(".accounts-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: pending) }
        try encoder.encode(index).write(to: pending, options: .withoutOverwriting)
        try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: pending.path)
        try syncFile(pending)
        guard Darwin.rename(pending.path, indexURL.path) == 0 else { throw posixError() }
        try syncDirectory()
        try checkpoint(.indexWritten)
    }

    private func withLock<T>(_ body: () throws -> T) throws -> T {
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let fd = Darwin.open(directory.appendingPathComponent("accounts.lock").path, O_CREAT | O_RDWR | O_NOFOLLOW, 0o600)
        guard fd >= 0 else { throw posixError() }
        defer { Darwin.close(fd) }
        guard flock(fd, LOCK_EX) == 0 else { throw posixError() }
        defer { flock(fd, LOCK_UN) }
        return try body()
    }
    private func syncFile(_ url: URL) throws {
        let fd = Darwin.open(url.path, O_RDONLY | O_NOFOLLOW)
        guard fd >= 0 else { throw posixError() }
        defer { Darwin.close(fd) }
        guard fsync(fd) == 0 else { throw posixError() }
    }
    private func syncDirectory() throws {
        let fd = Darwin.open(directory.path, O_RDONLY)
        guard fd >= 0 else { throw posixError() }
        defer { Darwin.close(fd) }
        guard fsync(fd) == 0 else { throw posixError() }
    }
    private func posixError() -> NSError { NSError(domain: NSPOSIXErrorDomain, code: Int(errno)) }
    private static func isLocked(_ error: Error) -> Bool {
        if case Failure.waitingForUnlock = error { return true }
        let e = error as NSError
        return (e.domain == NSPOSIXErrorDomain && (e.code == Int(EACCES) || e.code == Int(EPERM)))
            || (e.domain == NSCocoaErrorDomain && (e.code == NSFileReadNoPermissionError || e.code == NSFileWriteNoPermissionError))
    }
    private static func validAddress(_ address: String) -> Bool {
        address.count == 42 && address.lowercased().hasPrefix("0x")
            && address.dropFirst(2).allSatisfy { $0.isASCII && $0.isHexDigit }
    }
}

/// Atomically install a fully written handle without replacing an existing
/// one. Killing the process while staging leaves no partial numbered handle;
/// killing it after link leaves a complete handle recoverable by the index.
enum AccountHandleFile {
    static func writeNew(_ bytes: Data, to url: URL, staged: () throws -> Void = {}) throws {
        let directory = url.deletingLastPathComponent()
        let pending = directory.appendingPathComponent(".handle-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: pending) }
        try bytes.write(to: pending, options: [.withoutOverwriting, .completeFileProtection])
        try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: pending.path)
        let fd = Darwin.open(pending.path, O_RDONLY | O_NOFOLLOW)
        guard fd >= 0 else { throw NSError(domain: NSPOSIXErrorDomain, code: Int(errno)) }
        defer { Darwin.close(fd) }
        guard fsync(fd) == 0 else { throw NSError(domain: NSPOSIXErrorDomain, code: Int(errno)) }
        try staged()
        guard Darwin.link(pending.path, url.path) == 0 else { throw NSError(domain: NSPOSIXErrorDomain, code: Int(errno)) }
        let parent = Darwin.open(directory.path, O_RDONLY)
        guard parent >= 0 else { throw NSError(domain: NSPOSIXErrorDomain, code: Int(errno)) }
        defer { Darwin.close(parent) }
        guard fsync(parent) == 0 else { throw NSError(domain: NSPOSIXErrorDomain, code: Int(errno)) }
    }
}
