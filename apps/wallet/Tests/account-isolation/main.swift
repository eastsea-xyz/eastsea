// Exercise selected-account storage with dictionary-only preferences and fake handles.
import Foundation

final class MemoryDefaults: UserDefaults, @unchecked Sendable {
    private var values: [String: Any] = [:]
    override func object(forKey key: String) -> Any? { values[key] }
    override func set(_ value: Any?, forKey key: String) { values[key] = value }
    override func set(_ value: Bool, forKey key: String) { set(value as Any, forKey: key) }
    override func removeObject(forKey key: String) { values.removeValue(forKey: key) }
    override func dictionaryRepresentation() -> [String: Any] { values }
}

var checks = 0
var failures = 0
func check(_ ok: @autoclosure () throws -> Bool, _ label: String) {
    checks += 1
    do {
        if try ok() { print("ok   \(label)") }
        else { failures += 1; print("FAIL account-isolation: \(label)") }
    } catch { failures += 1; print("FAIL account-isolation: \(label): \(error)") }
}
enum FixtureError: Error { case invalidHandle, missingActiveStore }
struct ActivityRecord: Codable, Equatable { let hash: String; let amountWei: String }
struct TokenRecord: Codable, Equatable { let address: String; let balanceAtoms: String }
func address(_ id: Int) -> String {
    let hex = String(id, radix: 16)
    return "0x" + String(repeating: "0", count: 40 - hex.count) + hex
}
func openFixtureHandle(_ url: URL) throws -> String {
    let text = String(decoding: try Data(contentsOf: url), as: UTF8.self)
    guard text.hasPrefix("fixture-account-"), let id = Int(text.dropFirst("fixture-account-".count)) else {
        throw FixtureError.invalidHandle
    }
    return address(id)
}
@MainActor
func activeStore(_ accounts: AccountStore, chain: UInt64, defaults: MemoryDefaults) throws -> AccountDataStore {
    guard let store = accounts.activeDataStore(chainID: chain, defaults: defaults) else {
        throw FixtureError.missingActiveStore
    }
    return store
}
func saveRecords(_ store: AccountDataStore, marker: Int) throws {
    try store.save([ActivityRecord(hash: "fixture-tx-\(marker)", amountWei: "\(marker)")], to: .activity)
    try store.save([TokenRecord(address: address(100 + marker), balanceAtoms: "\(marker)")], to: .tokenHoldings)
    try store.save([WalletContact(id: UUID(uuidString: String(format: "00000000-0000-0000-0000-%012d", marker))!,
                                  name: "Contact \(marker)", address: address(200 + marker))], to: .contacts)
}
func checkRecords(_ store: AccountDataStore, marker: Int, context: String) {
    check(store.load(.activity, as: [ActivityRecord].self)
          == [ActivityRecord(hash: "fixture-tx-\(marker)", amountWei: "\(marker)")], "\(context): activity follows selection")
    check(store.load(.tokenHoldings, as: [TokenRecord].self)
          == [TokenRecord(address: address(100 + marker), balanceAtoms: "\(marker)")], "\(context): token holdings follow selection")
    check(store.load(.contacts, as: [WalletContact].self)
          == [WalletContact(id: UUID(uuidString: String(format: "00000000-0000-0000-0000-%012d", marker))!,
                            name: "Contact \(marker)", address: address(200 + marker))], "\(context): contacts follow selection")
}
func checkEmpty(_ store: AccountDataStore, context: String) {
    check(store.load(.activity, as: [ActivityRecord].self) == nil, "\(context): activity starts empty")
    check(store.load(.tokenHoldings, as: [TokenRecord].self) == nil, "\(context): token holdings start empty")
    check(store.load(.contacts, as: [WalletContact].self) == nil, "\(context): contacts start empty")
}

@MainActor
func runTests() throws {
    guard let temp = ProcessInfo.processInfo.environment["AETHER_AGENT_TEST_TMP"], !temp.isEmpty else {
        print("FAIL account-isolation: task-owned temporary directory is required"); exit(1)
    }
    let directory = URL(fileURLWithPath: temp, isDirectory: true)
        .appendingPathComponent("account-isolation-\(UUID().uuidString)", isDirectory: true)
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: directory) }
    var created = 0
    let keys = AccountStore.KeyAccess(open: openFixtureHandle, create: { url in
        created += 1
        try Data("fixture-account-\(created)".utf8).write(to: url, options: .withoutOverwriting)
        return address(created)
    })
    let defaults = MemoryDefaults()
    let accounts = AccountStore(directory: directory, keys: keys,
                                balances: { _ in .init(nativeWei: "0", tokenBalances: []) })
    check(accounts.activeDataStore(chainID: 7780, defaults: defaults) == nil,
          "an unloaded wallet cannot choose an account data namespace")
    check(defaults.dictionaryRepresentation().isEmpty, "asking an unloaded wallet for data writes no preferences")
    try accounts.load()
    let first = accounts.activeAccount!
    let firstData = try activeStore(accounts, chain: 7780, defaults: defaults)
    check(firstData.address == first.address && firstData.chainID == 7780, "the ready wallet scopes data to its selected address and chain")
    try saveRecords(firstData, marker: 1)

    let second = try accounts.createAndSelect(name: "Savings")
    let secondData = try activeStore(accounts, chain: 7780, defaults: defaults)
    check(secondData.address == second.address, "creating and selecting changes the account data owner")
    checkEmpty(secondData, context: "second account, first chain")
    try saveRecords(secondData, marker: 2)
    try accounts.select(first.id)
    checkRecords(try activeStore(accounts, chain: 7780, defaults: defaults), marker: 1, context: "first account, first chain")
    try accounts.select(second.id)
    checkRecords(try activeStore(accounts, chain: 7780, defaults: defaults), marker: 2, context: "second account, first chain")

    try accounts.select(first.id)
    let firstOtherChain = try activeStore(accounts, chain: 7781, defaults: defaults)
    checkEmpty(firstOtherChain, context: "first account, other chain")
    try saveRecords(firstOtherChain, marker: 3)
    try accounts.select(second.id)
    let secondOtherChain = try activeStore(accounts, chain: 7781, defaults: defaults)
    checkEmpty(secondOtherChain, context: "second account, other chain")
    try saveRecords(secondOtherChain, marker: 4)
    checkRecords(try activeStore(accounts, chain: 7780, defaults: defaults), marker: 2, context: "returning to second account's first chain")
    try accounts.select(first.id)
    checkRecords(try activeStore(accounts, chain: 7781, defaults: defaults), marker: 3, context: "returning to first account's other chain")
    checkRecords(try activeStore(accounts, chain: 7780, defaults: defaults), marker: 1, context: "returning to first account's first chain")
    try accounts.select(second.id)
    checkRecords(try activeStore(accounts, chain: 7781, defaults: defaults), marker: 4, context: "returning to second account's other chain")
    try accounts.rename(second.id, name: "Travel")
    try accounts.reorder([second.id, first.id])
    checkRecords(try activeStore(accounts, chain: 7780, defaults: defaults), marker: 2, context: "renamed and reordered selected account")

    let reopened = AccountStore(directory: directory, keys: keys)
    try reopened.load()
    checkRecords(try activeStore(reopened, chain: 7780, defaults: defaults), marker: 2, context: "reopened selected account")
    check(defaults.object(forKey: "contacts") == nil && defaults.object(forKey: "activity") == nil
          && defaults.object(forKey: "tokenHoldings") == nil, "selected-account writes leave unscoped preferences untouched")

    // A failed reload can retain the old published account; state must still gate storage.
    try Data("invalid-fixture-index".utf8).write(to: directory.appendingPathComponent("accounts.json"))
    do { try reopened.load(); check(false, "the corrupt fixture index fails to reload") }
    catch { check(reopened.activeAccount?.id == second.id, "a failed reload fixture retains the previous selected record") }
    check(reopened.activeDataStore(chainID: 7780, defaults: defaults) == nil,
          "a failed wallet cannot use a stale selected account data namespace")

    let lockedDirectory = directory.appendingPathComponent("locked-fixture", isDirectory: true)
    let locked = AccountStore(directory: lockedDirectory, keys: .init(
        open: { _ in throw AccountStore.Failure.waitingForUnlock },
        create: { _ in throw AccountStore.Failure.waitingForUnlock }))
    do { try locked.load(); check(false, "the locked handle fixture waits for unlock") }
    catch { check(locked.state == .waitingForUnlock, "the locked fixture reports waiting for unlock") }
    check(locked.activeDataStore(chainID: 7780, defaults: defaults) == nil,
          "a wallet waiting for unlock cannot choose an account data namespace")
    print("account-isolation: \(checks) checks, \(failures) failures")
    exit(failures == 0 ? 0 : 1)
}

MainActor.assumeIsolated {
    do { try runTests() }
    catch { print("FAIL account-isolation: unexpected error: \(error)"); exit(1) }
}
