import Foundation

var checks = 0
var failures = 0
func check(_ name: String, _ body: () -> Bool) {
    checks += 1
    if body() { print("ok   \(name)") }
    else { failures += 1; print("FAIL account-operations: \(name)") }
}

check("a second concurrent signature is refused") {
    var gate = WalletOperationGate()
    guard gate.begin() != nil else { return false }
    return gate.begin() == nil && gate.blocksAccountChange
}

check("an old tracker cannot permit switching during a new signature") {
    var gate = WalletOperationGate()
    guard let a = gate.begin() else { return false }
    gate.submitted(a)
    guard let b = gate.begin() else { return false }
    let oldReleased = gate.release(a)
    let switchingForbidden = gate.blocksAccountChange
    let currentReleased = gate.release(b)
    return !oldReleased && switchingForbidden && currentReleased && !gate.blocksAccountChange
}

check("submitted B retains its busy ownership when old A completes") {
    var gate = WalletOperationGate()
    guard let a = gate.begin() else { return false }
    gate.submitted(a)
    guard let b = gate.begin() else { return false }
    gate.submitted(b)
    let noSignature = !gate.blocksAccountChange
    let oldReleased = gate.release(a)
    let currentReleased = gate.release(b)
    return noSignature && !oldReleased && currentReleased && !gate.release(b)
}

check("repeated old polling releases cannot clear a new signing guard") {
    var gate = WalletOperationGate()
    guard let a = gate.begin() else { return false }
    gate.submitted(a)
    guard let b = gate.begin() else { return false }
    let releases = [gate.release(a), gate.release(a), gate.release(a)]
    return releases == [false, false, false] && gate.blocksAccountChange && gate.release(b)
}

check("late submission of A cannot remove B's signature guard") {
    var gate = WalletOperationGate()
    guard let a = gate.begin() else { return false }
    gate.submitted(a)
    guard let b = gate.begin() else { return false }
    gate.submitted(a)
    return gate.blocksAccountChange && gate.release(b)
}

check("a submission with an unknown token cannot remove a signature guard") {
    var gate = WalletOperationGate()
    guard let token = gate.begin() else { return false }
    gate.submitted(WalletOperationGate.Token(id: UUID()))
    return gate.blocksAccountChange && gate.release(token)
}

check("a release with an unknown token cannot clear busy ownership") {
    var gate = WalletOperationGate()
    guard let token = gate.begin() else { return false }
    let unknownReleased = gate.release(WalletOperationGate.Token(id: UUID()))
    return !unknownReleased && gate.blocksAccountChange && gate.release(token)
}

check("a signature failure releases only its own guard") {
    var gate = WalletOperationGate()
    guard let failed = gate.begin() else { return false }
    let failedReleased = gate.release(failed)
    let unblocked = !gate.blocksAccountChange
    guard let next = gate.begin() else { return false }
    let repeatedFailure = gate.release(failed)
    return failedReleased && unblocked && !repeatedFailure && gate.blocksAccountChange && gate.release(next)
}

check("a submitted operation releases once without stealing the next operation") {
    var gate = WalletOperationGate()
    guard let a = gate.begin() else { return false }
    gate.submitted(a)
    let firstReleased = gate.release(a)
    let repeatedFirst = gate.release(a)
    guard let b = gate.begin() else { return false }
    gate.submitted(b)
    let staleReleased = gate.release(a)
    let secondReleased = gate.release(b)
    return firstReleased && !repeatedFirst && !staleReleased && secondReleased && !gate.release(b)
}

check("repeated submission cannot affect a later signature") {
    var gate = WalletOperationGate()
    guard let a = gate.begin() else { return false }
    gate.submitted(a)
    gate.submitted(a)
    guard let b = gate.begin() else { return false }
    gate.submitted(a)
    gate.submitted(a)
    return gate.blocksAccountChange && gate.release(b)
}

check("a refused begin preserves the first operation's identity") {
    var gate = WalletOperationGate()
    guard let first = gate.begin() else { return false }
    let refused = gate.begin()
    gate.submitted(first)
    let firstReleased = gate.release(first)
    return refused == nil && firstReleased && !gate.blocksAccountChange && !gate.release(first)
}

check("copied tokens preserve authority without authorizing a later operation") {
    var gate = WalletOperationGate()
    guard let a = gate.begin() else { return false }
    let copied = WalletOperationGate.Token(id: a.id)
    gate.submitted(copied)
    guard let b = gate.begin() else { return false }
    let copiedReleased = gate.release(copied)
    return copied == a && b != a && !copiedReleased && gate.blocksAccountChange && gate.release(b)
}

print("account-operations: \(checks) checks, \(failures) failures")
exit(failures == 0 ? 0 : 1)
