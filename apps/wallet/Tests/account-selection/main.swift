// Account-switcher behavior. Handles contain opaque fixture text, never keys.
import Foundation
import Combine
import Darwin

var checks = 0
var failures = 0
func check(_ ok: @autoclosure () throws -> Bool, _ label: String) {
    checks += 1
    do {
        if try ok() { print("ok   \(label)") }
        else { failures += 1; print("FAIL account-selection: \(label)") }
    } catch { failures += 1; print("FAIL account-selection: \(label): \(error)") }
}

enum FixtureError: Error { case invalidHandle }
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
func lockAvailable(_ directory: URL) -> Bool {
    let fd = Darwin.open(directory.appendingPathComponent("accounts.lock").path, O_RDONLY | O_NOFOLLOW)
    guard fd >= 0 else { return false }
    defer { Darwin.close(fd) }
    guard flock(fd, LOCK_EX | LOCK_NB) == 0 else { return false }
    flock(fd, LOCK_UN)
    return true
}

@MainActor
func runTests() throws {
    guard let temp = ProcessInfo.processInfo.environment["AETHER_AGENT_TEST_TMP"], !temp.isEmpty else {
        print("FAIL account-selection: task-owned temporary directory is required"); exit(1)
    }
    let directory = URL(fileURLWithPath: temp, isDirectory: true)
        .appendingPathComponent("account-selection-\(UUID().uuidString)", isDirectory: true)
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: directory) }
    var created = 0
    let keys = AccountStore.KeyAccess(open: openFixtureHandle, create: { url in
        created += 1
        try Data("fixture-account-\(created)".utf8).write(to: url, options: .withoutOverwriting)
        return address(created)
    })
    let store = AccountStore(directory: directory, keys: keys,
                             balances: { _ in .init(nativeWei: "0", tokenBalances: []) })
    var emittedIDs: [Int] = []
    var observersSeeCommittedState: [Bool] = []
    var observersSeeUnlockedIndex: [Bool] = []
    let subscription = store.activeAccountPublisher.sink { account in
        guard let account else { return }
        emittedIDs.append(account.id)
        let data = try? Data(contentsOf: directory.appendingPathComponent("accounts.json"))
        let json = data.flatMap { try? JSONSerialization.jsonObject(with: $0) } as? [String: Any]
        observersSeeCommittedState.append(store.state == .ready && store.activeAccount == account
            && store.list().contains(account) && json?["activeID"] as? Int == account.id)
        observersSeeUnlockedIndex.append(lockAvailable(directory))
    }
    try store.load()
    let first = store.activeAccount!
    check(first.id == 1, "a new wallet selects account 1")
    check(store.payoutAddress == first.address, "node payout defaults to account 1")

    let second = try store.createAndSelect(name: "Savings", color: "blue")
    check(second.id == 2 && second.address != first.address, "creating an account gives it a distinct address")
    check(store.activeAccount == second, "creating in the switcher selects the new account")
    check(second.name == "Savings" && second.color == "blue", "creation keeps the requested name and color")
    check(emittedIDs.last == second.id, "the committed active-account publisher sends the newly selected account")
    check(store.payoutAddress == first.address, "creating and selecting cannot redirect node payout")

    try store.rename(second.id, name: "  Travel  ")
    check(store.activeAccount?.id == second.id && store.activeAccount?.name == "Travel",
          "renaming updates the selected account without changing its identity")
    try store.reorder([second.id, first.id])
    check(store.list().map(\.id) == [second.id, first.id], "the switcher preserves the requested account order")
    check(store.activeAccount?.id == second.id, "reordering keeps the selected permanent handle number")
    let reopened = AccountStore(directory: directory, keys: keys,
                                balances: { _ in .init(nativeWei: "0", tokenBalances: []) })
    try reopened.load()
    check(reopened.activeAccount == store.activeAccount && reopened.list() == store.list(),
          "selection, rename, and order survive reopening the account index")
    check(reopened.payoutAddress == first.address, "reopening keeps the default payout independently of the selected row")

    try store.setPayoutAccount(second.id)
    check(store.payoutAddress == second.address, "an explicit Settings choice changes node payout")
    try store.select(first.id)
    check(store.activeAccount?.id == first.id && store.payoutAddress == second.address,
          "switching accounts keeps the explicitly chosen payout")
    try store.reorder([first.id, second.id])
    try store.rename(first.id, name: "Everyday")
    check(store.payoutAddress == second.address, "renaming and reordering cannot redirect the explicit payout")
    try reopened.load()
    check(reopened.activeAccount?.id == first.id && reopened.payoutAddress == second.address,
          "selection and explicit payout survive reopening independently")

    store.canChangeAccount = { false }
    let handlesBeforeBusyCreation = created
    do {
        _ = try store.createAndSelect()
        check(false, "an approval in progress blocks switcher creation")
    } catch AccountStore.Failure.operationInProgress {
        check(created == handlesBeforeBusyCreation && store.activeAccount?.id == first.id,
              "an approval in progress blocks creation before a handle or selection changes")
    } catch { check(false, "busy creation reports the operation gate: \(error)") }
    check(!observersSeeCommittedState.isEmpty && observersSeeCommittedState.allSatisfy { $0 },
          "every published selection observes the committed in-memory and durable index")
    check(observersSeeUnlockedIndex.allSatisfy { $0 }, "selection callbacks run after the account file lock is released")
    withExtendedLifetime(subscription) {}
    print("account-selection: \(checks) checks, \(failures) failures")
    exit(failures == 0 ? 0 : 1)
}

MainActor.assumeIsolated {
    do { try runTests() }
    catch { print("FAIL account-selection: unexpected error: \(error)"); exit(1) }
}
