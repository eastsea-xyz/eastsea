// B5 review round 2, finding 5: what an activity row (and a payment-link
// callback) does with one status answer. A node-local drop, or a node that
// has never heard of the hash, is "not on chain yet" — never a failure. Only
// chain facts settle a row: a receipt, or the nonce used by another tx.
// Pure Foundation: no RPC, no FFI, no SwiftUI.
import Foundation

func check(_ name: String, _ ok: Bool) {
    if ok { print("ok   \(name)") } else { print("FAIL \(name)"); exit(1) }
}

check("a receipt that succeeded is done", TxTrack.row(state: "included", success: true, unknownFor: 0) == .done)
check("a receipt that failed is failed", TxTrack.row(state: "included", success: false, unknownFor: 0) == .failed)
check("a node-local drop is not a failure", TxTrack.row(state: "dropped", success: nil, unknownFor: 0) == .notIncluded)
check("still pending waits", TxTrack.row(state: "pending", success: nil, unknownFor: 0) == .pending)
check("a node without a record never settles the row", TxTrack.row(state: "unknown", success: nil, unknownFor: 600) == .pending)
check("no answer at all never settles the row", TxTrack.row(state: nil, success: nil, unknownFor: 600) == .pending)
check("the nonce used by another tx is a chain fact", TxTrack.row(state: "replaced", success: nil, unknownFor: 0) == .failed)
check("the callback says success only on a receipt", TxTrack.callbackStatus(.done) == "success")
check("the callback never reports a drop as failed", TxTrack.callbackStatus(.notIncluded) == "not_included")
check("the callback reports a pending tx as pending", TxTrack.callbackStatus(.pending) == "pending")
check("the callback's failed is a chain fact", TxTrack.callbackStatus(.failed) == "failed")
check("only chain facts are final", TxTrack.isFinal(.done) && TxTrack.isFinal(.failed) && !TxTrack.isFinal(.notIncluded) && !TxTrack.isFinal(.pending))
// In the bundle's language: a bare test binary is English.
check("the words say not recorded yet", TxTrack.notIncludedNote == "Not processed (not on chain yet)")
print("tx-track: all ok")
