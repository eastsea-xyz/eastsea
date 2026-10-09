import Foundation
import Combine
import Darwin

var checks = 0
var failures = 0
func check(_ ok: @autoclosure () throws -> Bool, _ label: String) {
    checks += 1
    do {
        if try ok() { print("ok   \(label)") }
        else { failures += 1; print("FAIL \(label)") }
    } catch { failures += 1; print("FAIL \(label): \(error)") }
}

enum TestError: Error { case interrupted, locked, offline }
let fm = FileManager.default
let testRoot = URL(fileURLWithPath: ProcessInfo.processInfo.environment["AETHER_AGENT_TEST_TMP"]
    ?? URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent()
        .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent().appendingPathComponent("tmp").path)
let handles = [Data("first-handle".utf8), Data("second-handle".utf8), Data("third-handle".utf8)]
func address(_ data: Data) -> String {
    "0x" + String(repeating: data == handles[0] ? "11" : (data == handles[1] ? "22" : "33"), count: 20)
}
func directory() throws -> URL {
    let dir = testRoot.appendingPathComponent("multi-account-\(UUID().uuidString)", isDirectory: true)
    try fm.createDirectory(at: dir, withIntermediateDirectories: true)
    return dir
}

@MainActor
func runTests() throws {
    var created = 0
    let keys = AccountStore.KeyAccess(open: { url in address(try Data(contentsOf: url)) }, create: { url in
        let data = handles[min(created, handles.count - 1)]
        created += 1
        try data.write(to: url, options: .withoutOverwriting)
        return address(data)
    })
    let zero = AccountStore.Balances(nativeWei: "0", tokenBalances: [])

    // Restart after every durable migration boundary, including an actual kill.
    for boundary in [AccountStore.Checkpoint.handleStaged, .handleWritten, .indexWritten] {
        let dir = try directory()
        defer { try? fm.removeItem(at: dir) }
        let old = dir.appendingPathComponent("enclave-key.dat")
        try handles[0].write(to: old)
        let interrupted = AccountStore(directory: dir, keys: keys, balances: { _ in zero }, checkpoint: { stage in
            if stage == boundary { throw TestError.interrupted }
        })
        do { try interrupted.load(); check(false, "migration interruption propagates at \(boundary)") }
        catch { check(true, "migration interruption propagates at \(boundary)") }
        check(try Data(contentsOf: old) == handles[0], "old handle survives \(boundary)")
        let resumed = AccountStore(directory: dir, keys: keys, balances: { _ in zero })
        try resumed.load()
        check(resumed.list().count == 1 && resumed.activeAccount?.id == 1, "migration resumes at \(boundary)")
        check(resumed.activeAccount?.address == address(handles[0]), "migration keeps address at \(boundary)")
        check(try Data(contentsOf: resumed.handleURL(for: 1)) == handles[0], "migration keeps exact handle at \(boundary)")
        try resumed.load()
        check(resumed.list().count == 1 && created == 0, "migration is idempotent at \(boundary)")
    }

    for boundary in ["handleStaged", "handleWritten", "indexWritten"] {
        let dir = try directory()
        defer { try? fm.removeItem(at: dir) }
        try handles[0].write(to: dir.appendingPathComponent("enclave-key.dat"))
        let child = Process()
        child.executableURL = URL(fileURLWithPath: CommandLine.arguments[0])
        child.arguments = ["kill-migration", dir.path, boundary]
        try child.run()
        child.waitUntilExit()
        check(child.terminationReason == .uncaughtSignal && child.terminationStatus == SIGKILL, "migration process is killed at \(boundary)")
        let resumed = AccountStore(directory: dir, keys: keys, balances: { _ in zero })
        try resumed.load()
        check(resumed.activeAccount?.address == address(handles[0]), "killed migration resumes with original address")
        check(try Data(contentsOf: dir.appendingPathComponent("enclave-key.dat")) == handles[0], "killed migration leaves original handle intact")
    }

    do {
        let dir = try directory()
        defer { try? fm.removeItem(at: dir) }
        try handles[0].write(to: dir.appendingPathComponent("enclave-key.dat"))
        var locked = true
        let store = AccountStore(directory: dir, keys: .init(open: { url in
            if locked { throw AccountStore.Failure.waitingForUnlock }
            return address(try Data(contentsOf: url))
        }, create: keys.create), balances: { _ in zero })
        do { try store.load(); check(false, "locked migration waits") }
        catch { check(store.state == .waitingForUnlock && store.list().isEmpty, "locked migration waits") }
        check(!fm.fileExists(atPath: dir.appendingPathComponent("accounts.json").path), "locked migration does not commit index")
        check(created == 0, "locked migration never creates replacement key")
        locked = false
        try store.load()
        check(store.activeAccount?.address == address(handles[0]), "unlock resumes the original account")
    }

    do {
        let dir = try directory()
        defer { try? fm.removeItem(at: dir) }
        created = 0
        var balance = zero
        let store = AccountStore(directory: dir, keys: keys, balances: { _ in balance })
        var published: [Int?] = []
        var observedStored: [Int?] = []
        let subscription = store.activeAccountPublisher.sink {
            published.append($0?.id)
            observedStored.append(store.activeAccount?.id)
        }
        try store.load()
        let first = store.activeAccount!
        let second = try store.create(name: "Trading", color: "blue")
        check(first.id == 1 && second.id == 2 && first.address != second.address, "creation gives independent addresses")
        check(try Data(contentsOf: store.handleURL(for: 1)) != Data(contentsOf: store.handleURL(for: 2)), "creation writes independent handles")
        check(store.activeAccount?.id == first.id, "creation does not change selection")
        try store.select(second.id)
        check(store.activeAccount?.id == second.id && published.last! == second.id, "selection publishes active account")
        check(observedStored == published, "active publisher observes committed selection")
        check(store.payoutAddress == first.address, "switching keeps payout on account 1")
        try store.rename(second.id, name: "Savings")
        try store.reorder([second.id, first.id])
        check(store.list().map(\.id) == [second.id, first.id] && store.list()[0].name == "Savings", "rename and reorder persist metadata")
        check(store.payoutAddress == first.address, "reordering cannot redirect payout")
        let reopened = AccountStore(directory: dir, keys: keys, balances: { _ in balance })
        try reopened.load()
        check(reopened.list() == store.list() && reopened.activeAccount?.id == second.id, "selection and metadata survive restart")
        check(reopened.payoutAddress == first.address, "restart retains account 1 payout")
        balance = .init(nativeWei: "1", tokenBalances: [])
        do { try store.delete(second.id); check(false, "funded account deletion blocked") }
        catch { check(fm.fileExists(atPath: store.handleURL(for: second.id).path) && store.list().contains(where: { $0.id == second.id }), "funded account deletion blocked") }
        balance = .init(nativeWei: "0", tokenBalances: ["900000000000000000000000000000000000"])
        do { try store.hide(second.id); check(false, "token funds block hiding") }
        catch { check(store.list().contains(where: { $0.id == second.id }), "token funds block hiding") }
        balance = .init(nativeWei: "unknown", tokenBalances: [])
        do { try store.delete(second.id); check(false, "unknown balance blocks deletion") }
        catch { check(fm.fileExists(atPath: store.handleURL(for: second.id).path), "unknown balance blocks deletion") }
        balance = zero
        try store.setPayoutAccount(second.id)
        check(store.payoutAddress == second.address, "payout changes only when explicitly chosen")
        do { try store.delete(second.id); check(false, "payout account deletion blocked") }
        catch { check(fm.fileExists(atPath: store.handleURL(for: second.id).path), "payout account deletion blocked") }
        try store.setPayoutAccount(first.id)
        try store.delete(second.id)
        check(!store.list().contains(where: { $0.id == second.id }) && store.activeAccount?.id == first.id, "zero balance permits guarded deletion and selects survivor")
        check(fm.fileExists(atPath: store.handleURL(for: second.id).path) && store.retiredAccountIDs.contains(second.id), "deletion retains key for funds on other chains")
        let third = try store.create(name: "Third", color: "green")
        check(third.id == 3, "deleted account numbers are never reused")
        try store.hide(third.id)
        check(!store.list().contains(where: { $0.id == third.id }) && fm.fileExists(atPath: store.handleURL(for: third.id).path), "hide retains zero-balance handle")
        try store.unhide(third.id)
        check(store.list().contains(where: { $0.id == third.id }), "hidden account can be restored")
        try store.restoreDeleted(second.id)
        check(store.list().contains(where: { $0.id == second.id && $0.address == second.address }), "retired account can recover funds received later")
        withExtendedLifetime(subscription) {}
    }

    do {
        let dir = try directory()
        defer { try? fm.removeItem(at: dir) }
        created = 0
        let store = AccountStore(directory: dir, keys: keys, balances: { _ in zero })
        try store.load()
        do { _ = try store.create(name: "  "); check(false, "invalid name is refused before key creation") }
        catch { check(created == 1 && !fm.fileExists(atPath: store.handleURL(for: 2).path), "invalid name is refused before key creation") }
    }

    do {
        let dir = try directory()
        defer { try? fm.removeItem(at: dir) }
        created = 0
        let store = AccountStore(directory: dir, keys: keys, balances: { _ in zero })
        try store.load()
        let second = try store.create()
        let file = dir.appendingPathComponent("accounts.json")
        var json = try JSONSerialization.jsonObject(with: Data(contentsOf: file)) as! [String: Any]
        var rows = json["accounts"] as! [[String: Any]]
        rows[1]["address"] = address(handles[2])
        json["accounts"] = rows
        try JSONSerialization.data(withJSONObject: json).write(to: file)
        let reopened = AccountStore(directory: dir, keys: keys, balances: { _ in zero })
        try reopened.load()
        let payout = reopened.payoutAddress
        do { try reopened.setPayoutAccount(second.id); check(false, "payout selection verifies inactive handle") }
        catch { check(reopened.payoutAddress == payout, "payout selection verifies inactive handle") }
        try reopened.setPayoutAddress(payout)
        do { try reopened.delete(second.id); check(false, "delete verifies inactive handle before balance lookup") }
        catch { check(fm.fileExists(atPath: store.handleURL(for: second.id).path) && reopened.list().count == 2, "delete verifies inactive handle before balance lookup") }
    }

    do {
        let dir = try directory()
        defer { try? fm.removeItem(at: dir) }
        created = 0
        let store = AccountStore(directory: dir, keys: keys, balances: { _ in zero })
        try store.load()
        let before = store.list()
        do { try store.reorder([1, 1]); check(false, "invalid order rejected") }
        catch { check(store.list() == before, "invalid order rejected") }
        do { try store.delete(1); check(false, "last account cannot be removed") }
        catch { check(fm.fileExists(atPath: store.handleURL(for: 1).path), "last account cannot be removed") }
        try Data("corrupt-index".utf8).write(to: dir.appendingPathComponent("accounts.json"))
        let reopened = AccountStore(directory: dir, keys: keys, balances: { _ in zero })
        do { try reopened.load(); check(false, "corrupt index fails closed") }
        catch { check(created == 1 && fm.fileExists(atPath: store.handleURL(for: 1).path), "corrupt index fails closed") }
    }

    do {
        let dir = try directory()
        defer { try? fm.removeItem(at: dir) }
        created = 0
        let store = AccountStore(directory: dir, keys: keys, balances: { _ in zero })
        try store.load()
        let interrupted = AccountStore(directory: dir, keys: keys, balances: { _ in zero }, checkpoint: { stage in
            if stage == .handleWritten { throw TestError.interrupted }
        })
        try interrupted.load()
        do { _ = try interrupted.create(name: "Interrupted"); check(false, "create interruption propagates") }
        catch { check(fm.fileExists(atPath: interrupted.handleURL(for: 2).path), "create interruption preserves new handle") }
        let reopened = AccountStore(directory: dir, keys: keys, balances: { _ in zero })
        try reopened.load()
        check(reopened.list().map(\.id) == [1, 2] && created == 2, "unindexed created handle is recovered without replacement")
        check(reopened.payoutAddress == store.payoutAddress, "orphan recovery keeps payout choice")
    }
    print("\(checks) checks, \(failures) failures")
    exit(failures == 0 ? 0 : 1)
}

// Test-only child dies between copy and index commit. It never accesses the SE.
if CommandLine.arguments.count == 4 && CommandLine.arguments[1] == "kill-migration" {
    MainActor.assumeIsolated {
        let store = AccountStore(directory: URL(fileURLWithPath: CommandLine.arguments[2]), keys: .init(
            open: { url in address(try Data(contentsOf: url)) }, create: { _ in throw TestError.interrupted }),
            balances: { _ in .init(nativeWei: "0", tokenBalances: []) }, checkpoint: { stage in
                if String(describing: stage) == CommandLine.arguments[3] { kill(getpid(), SIGKILL) }
            })
        try! store.load()
    }
} else {
    MainActor.assumeIsolated {
        do { try runTests() }
        catch { print("FAIL unexpected error: \(error)"); exit(1) }
    }
}
