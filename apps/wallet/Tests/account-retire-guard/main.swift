// Guard preview is advisory; AccountStore.delete always performs the final fresh check.
import Foundation

var checks = 0
var failures = 0
func check(_ ok: @autoclosure () throws -> Bool, _ label: String) {
    checks += 1
    do {
        if try ok() { print("ok   \(label)") }
        else { failures += 1; print("FAIL account-retire-guard: \(label)") }
    } catch { failures += 1; print("FAIL account-retire-guard: \(label): \(error)") }
}
enum FixtureError: Error { case invalidHandle, offline }
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
func reason(_ failure: AccountStore.Failure?) -> String {
    guard let failure else { return "allowed" }
    switch failure {
    case .accountNotFound: return "accountNotFound"
    case .operationInProgress: return "operationInProgress"
    case .lastAccount: return "lastAccount"
    case .payoutAccount: return "payoutAccount"
    case .balanceNotZero: return "balanceNotZero"
    case .balanceUnavailable: return "balanceUnavailable"
    default: return "unexpected: \(failure)"
    }
}

@MainActor
func runTests() throws {
    guard let temp = ProcessInfo.processInfo.environment["AETHER_AGENT_TEST_TMP"], !temp.isEmpty else {
        print("FAIL account-retire-guard: task-owned temporary directory is required"); exit(1)
    }
    let directory = URL(fileURLWithPath: temp, isDirectory: true)
        .appendingPathComponent("account-retire-guard-\(UUID().uuidString)", isDirectory: true)
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: directory) }
    var created = 0
    let keys = AccountStore.KeyAccess(open: openFixtureHandle, create: { url in
        created += 1
        try Data("fixture-account-\(created)".utf8).write(to: url, options: .withoutOverwriting)
        return address(created)
    })
    var balances = AccountStore.Balances(nativeWei: "0", tokenBalances: [])
    var balanceReadFails = false
    var balanceReadIDs: [Int] = []
    let store = AccountStore(directory: directory, keys: keys, balances: { account in
        balanceReadIDs.append(account.id)
        if balanceReadFails { throw FixtureError.offline }
        return balances
    })
    check(reason(store.retirementFailure(for: 1)) == "accountNotFound", "an unloaded account cannot be retired")
    try store.load()
    let first = store.activeAccount!
    check(reason(store.retirementFailure(for: first.id)) == "lastAccount", "the last visible account cannot be retired")
    let second = try store.createAndSelect(name: "Savings")
    try store.select(first.id)
    let handle = store.handleURL(for: second.id)
    let originalHandle = try Data(contentsOf: handle)
    let index = directory.appendingPathComponent("accounts.json")
    let originalIndex = try Data(contentsOf: index)

    check(reason(store.retirementFailure(for: first.id)) == "payoutAccount", "the node payout account must be changed before retirement")
    check(reason(store.retirementFailure(for: 999)) == "accountNotFound", "an unknown account cannot be retired")
    check(balanceReadIDs.isEmpty, "account existence, last-account, and payout guards do not read balances")
    store.canChangeAccount = { false }
    check(reason(store.retirementFailure(for: second.id)) == "operationInProgress", "a pending approval blocks retirement preview")
    check(balanceReadIDs.isEmpty, "a busy wallet is refused before a balance query")
    store.canChangeAccount = { true }

    for value in ["1", "0000001", "115792089237316195423570985008687907853269984665640564039457584007913129639935"] {
        balances = .init(nativeWei: value, tokenBalances: [])
        check(reason(store.retirementFailure(for: second.id)) == "balanceNotZero", "native balance \(value) blocks retirement")
    }
    balances = .init(nativeWei: "0", tokenBalances: ["0", "1", "0"])
    check(reason(store.retirementFailure(for: second.id)) == "balanceNotZero", "one token atom blocks retirement even when native balance is zero")
    check(balanceReadIDs.last == second.id, "retirement checks the requested inactive account rather than the selected account")

    for raw in ["", "unknown", "0x0", "-1", "0.0", " 0", "0\n"] {
        balances = .init(nativeWei: raw, tokenBalances: [])
        check(store.retirementFailure(for: second.id) != nil, "malformed or missing native reading \(String(reflecting: raw)) fails closed")
        balances = .init(nativeWei: "0", tokenBalances: [raw])
        check(store.retirementFailure(for: second.id) != nil, "malformed or missing token reading \(String(reflecting: raw)) fails closed")
    }
    balanceReadFails = true
    check(reason(store.retirementFailure(for: second.id)) == "balanceUnavailable", "an unsuccessful balance read keeps the account")
    balanceReadFails = false
    let missingProvider = AccountStore(directory: directory, keys: keys)
    try missingProvider.load()
    check(reason(missingProvider.retirementFailure(for: second.id)) == "balanceUnavailable", "a missing balance provider cannot authorize retirement")

    balances = .init(nativeWei: "000", tokenBalances: ["0", "0000"])
    let readsBeforeZero = balanceReadIDs.count
    check(store.retirementFailure(for: second.id) == nil, "fresh zero native and token readings permit a retirement preview")
    check(balanceReadIDs.count == readsBeforeZero + 1, "the zero preview fetches a new balance reading")
    balances = .init(nativeWei: "0", tokenBalances: ["1"])
    check(reason(store.retirementFailure(for: second.id)) == "balanceNotZero", "a later funded reading invalidates the earlier zero preview")
    check(try Data(contentsOf: index) == originalIndex, "blocked and allowed previews never mutate the account index")
    check(try Data(contentsOf: handle) == originalHandle, "blocked and allowed previews never mutate the exact handle")

    balances = .init(nativeWei: "0", tokenBalances: [])
    check(store.retirementFailure(for: second.id) == nil, "a fresh zero preview can be shown before confirmation")
    balances = .init(nativeWei: "1", tokenBalances: [])
    let readsBeforeDelete = balanceReadIDs.count
    do { try store.delete(second.id); check(false, "confirmation rechecks funds after the preview") }
    catch AccountStore.Failure.balanceNotZero { check(true, "confirmation blocks funds received after the zero preview") }
    catch { check(false, "funded confirmation reports the zero guard: \(error)") }
    check(balanceReadIDs.count == readsBeforeDelete + 1, "final deletion always fetches balances independently of the preview")
    check(store.list().contains(where: { $0.id == second.id }), "a blocked final deletion retains the visible account")
    check(try Data(contentsOf: handle) == originalHandle, "a blocked final deletion retains the exact handle")

    balances = .init(nativeWei: "0", tokenBalances: [])
    check(store.retirementFailure(for: second.id) == nil, "a second fresh zero preview can be shown")
    balanceReadFails = true
    do { try store.delete(second.id); check(false, "confirmation handles a balance read becoming unavailable") }
    catch AccountStore.Failure.balanceUnavailable { check(true, "confirmation keeps the account when its fresh read fails") }
    catch { check(false, "unavailable confirmation reports the balance guard: \(error)") }
    balanceReadFails = false

    let hidden = try store.createAndSelect(name: "Hidden fixture")
    try store.hide(hidden.id)
    check(reason(store.retirementFailure(for: hidden.id)) == "accountNotFound", "the visible switcher cannot retire a hidden row")
    try store.select(second.id)
    check(store.retirementFailure(for: second.id) == nil, "a zero active account can be retired while another visible account remains")
    let readsBeforeSuccess = balanceReadIDs.count
    try store.delete(second.id)
    check(balanceReadIDs.count == readsBeforeSuccess + 1, "successful final deletion also performs a new balance check")
    check(store.activeAccount?.id == first.id && store.list().map(\.id) == [first.id],
          "retiring the selected row switches to a visible survivor")
    check(store.retiredAccountIDs.contains(second.id), "retirement records the permanent handle number")
    check(try Data(contentsOf: handle) == originalHandle, "retirement keeps the exact handle recoverable for other chains")
    check(reason(store.retirementFailure(for: first.id)) == "lastAccount", "a hidden row does not count as a visible retirement survivor")
    let reopened = AccountStore(directory: directory, keys: keys)
    try reopened.load()
    check(reopened.retiredAccountIDs.contains(second.id) && !reopened.list().contains(where: { $0.id == second.id }),
          "reopening does not rediscover a retired handle as a new account")
    print("account-retire-guard: \(checks) checks, \(failures) failures")
    exit(failures == 0 ? 0 : 1)
}

MainActor.assumeIsolated {
    do { try runTests() }
    catch { print("FAIL account-retire-guard: unexpected error: \(error)"); exit(1) }
}
