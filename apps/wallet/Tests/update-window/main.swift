// When a downloaded, verified update installs (docs/design/34-silent-updates.md
// §3.2, W1). Pure logic:
//   swiftc -o ./tmp/update-window apps/wallet/Sources/UpdateWindow.swift apps/wallet/Tests/update-window/main.swift && ./tmp/update-window
import Foundation
var failures = 0
func check(_ c: Bool, _ m: String) { if !c { print("FAIL", m); failures += 1 } else { print("ok  ", m) } }

typealias W = UpdateWindow
let idle = W.Moment()

check(W.decide(idle) == .installNow, "a non-validator with nothing open installs right away")

var m = idle
m.sendSheetOpen = true
check(W.decide(m) == .wait(.sendSheet), "an open send sheet holds the install")

m = idle
m.signing = true
check(W.decide(m) == .wait(.signing), "a Touch ID prompt or a signing in flight holds the install")

m = idle
m.migrating = true
check(W.decide(m) == .wait(.migration), "a running data migration holds the install")

m = idle
m.storageMoving = true
check(W.decide(m) == .wait(.storageMove), "a running block-data move holds the install")

m = idle
m.seated = true
m.inOwnSlot = true
check(W.decide(m) == .installNow, "a seated validator in its chain-assigned slot installs")

m.inOwnSlot = false
check(W.decide(m) == .wait(.outOfSlot), "a seated validator outside its slot waits")

m.inOwnSlot = nil
check(W.decide(m) == .wait(.seatedNoSlot), "a seated validator with no slot from the chain waits (installs when it leaves the committee, or at quit)")

m = idle
m.seated = true
m.inOwnSlot = true
m.sendSheetOpen = true
check(W.decide(m) == .wait(.sendSheet), "the slot never overrides an open send sheet")

m = idle
m.inOwnSlot = false
check(W.decide(m) == .installNow, "slot data is ignored for a Mac that is not seated")

check(W.Reason.seatedNoSlot.logLine.contains("waiting for safe moment"), "the waiting line says so")

if failures > 0 { print("\(failures) failed"); exit(1) }
print("all passed")
