// History failure classification (Task: the history view must say when the
// node cannot give the full list, instead of silently showing a stale one).
// Pure Foundation: no RPC, no FFI, no SwiftUI.
import Foundation

func expect(_ name: String, _ got: String, contains needle: String) {
    if got.localizedCaseInsensitiveContains(needle) { print("ok   \(name)") }
    else { print("FAIL \(name): \(got) lacks \(needle)"); exit(1) }
}

// An old node predates aether_accountHistory: the JSON-RPC layer answers
// "method not found" and the FFI surfaces it as WalletError.Rejected(message).
let oldNode = HistoryFailure.classify(message: "Rejected(\"method not found: aether_accountHistory\")")
if oldNode != .unsupportedNode { print("FAIL old node classified as \(oldNode)"); exit(1) }
print("ok   method not found → unsupportedNode")

// The message alone (WalletModel.ffiMessage unwraps the case) classifies the same.
if HistoryFailure.classify(message: "method not found: aether_accountHistory") != .unsupportedNode { print("FAIL bare method-not-found"); exit(1) }
print("ok   bare message classifies the same")

// A local node that is not running: connect refused surfaces as Network("local node: ...").
let refused = HistoryFailure.classify(message: "Network(\"local node: Connection refused (os error 61)\")")
if refused != .unreachable { print("FAIL local node refused classified as \(refused)"); exit(1) }
print("ok   local node connect refused → unreachable")

// Timeout wording from the remote path ("timed out", "connect").
if HistoryFailure.classify(message: "request timed out") != .unreachable { print("FAIL timed out"); exit(1) }
if HistoryFailure.classify(message: "Network(\"connect error: no route\")") != .unreachable { print("FAIL connect error"); exit(1) }
print("ok   timeouts and connect errors → unreachable")

// Anything else the node said: a real answer, just not one we can use.
if HistoryFailure.classify(message: "Rejected(\"history pruned at 100\")") != .refused { print("FAIL refused"); exit(1) }
if HistoryFailure.classify(message: "limit must be 1..200") != .refused { print("FAIL local validation counts as refused"); exit(1) }
print("ok   other node answers → refused")

// Every case speaks in plain words, and the old-node one says what to do
// (update or switch), as the bug report asked.
expect("unsupportedNode notice says update or switch", HistoryFailure.unsupportedNode.notice, contains: "Update the node or switch to another node")
expect("unsupportedNode notice says the list is limited", HistoryFailure.unsupportedNode.notice, contains: "cannot show your full history")
for f in [HistoryFailure.unreachable, .refused] {
    expect("\(f) notice says the list may be incomplete", f.notice, contains: "incomplete")
}

// A successful read clears the notice: nil is the "node answered" state.
if HistoryFailure.classify(message: "") != .refused { print("FAIL empty message"); exit(1) }
print("ok   empty message still classifies (never crashes)")

print("history-notice: all ok")
