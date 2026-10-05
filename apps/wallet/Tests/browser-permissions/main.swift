// Checks the Explore tab's per-origin permission store without an app:
//   swiftc -o ./tmp/browser-permissions-check apps/wallet/Sources/SitePermissions.swift apps/wallet/Tests/browser-permissions/main.swift && ./tmp/browser-permissions-check
import Foundation
func check(_ c: Bool, _ m: String) { if !c { print("FAIL", m); exit(1) } }

let A = "0x00000000000000000000000000000000000000aa"
let B = "0x00000000000000000000000000000000000000bb"
let origin = "https://dapp.test"

var store = SitePermissionStore()
check(store.sites.isEmpty, "starts empty")
check(store.connectedAddress(origin: origin, current: A) == nil, "nothing granted yet")

// A grant lets exactly that origin see exactly that address.
store.grant(origin: origin, address: A, at: Date(timeIntervalSince1970: 1_000))
check(store.connectedAddress(origin: origin, current: A) == A, "granted address visible")
check(store.connectedAddress(origin: origin, current: A.lowercased()) == A, "address match ignores case")
check(store.connectedAddress(origin: origin, current: B) == nil, "a different account is not connected")
check(store.connectedAddress(origin: "https://other.test", current: A) == nil, "another origin is not")
check(store.sites.first?.grantedAt == Date(timeIntervalSince1970: 1_000), "grant date kept")

// Re-granting replaces (no duplicates); revoking forgets.
store.grant(origin: origin, address: B, at: Date(timeIntervalSince1970: 2_000))
check(store.sites.count == 1, "re-grant replaces")
check(store.connectedAddress(origin: origin, current: B) == B, "new address stands")
store.revoke(origin: origin)
check(store.connectedAddress(origin: origin, current: B) == nil, "revoked")

// Account switch: every grant names an address the wallet no longer holds,
// so nothing stays connected.
store.grant(origin: origin, address: A)
store.grant(origin: "https://two.test", address: A)
store.revokeAll()
check(store.sites.isEmpty, "revokeAll clears")
store.grant(origin: origin, address: A)
check(store.connectedAddress(origin: origin, current: B) == nil, "account switch disconnects")

// Persistence round-trips through UserDefaults (isolated suite).
let suiteName = "browser-permissions-test-\(UUID().uuidString)"
let defaults = UserDefaults(suiteName: suiteName)!
defer { defaults.removePersistentDomain(forName: suiteName) }
var saved = SitePermissionStore()
saved.grant(origin: origin, address: A, at: Date(timeIntervalSince1970: 3_000))
saved.save(defaults: defaults)
var loaded = SitePermissionStore()
check(loaded.load(defaults: defaults) == true, "loads what was saved")
check(loaded == saved, "round-trip equal")
var fresh = SitePermissionStore()
check(fresh.load(defaults: UserDefaults(suiteName: "browser-permissions-missing")!) == false, "missing key loads nothing")

print("browser-permissions OK")
