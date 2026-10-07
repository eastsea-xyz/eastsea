// Pure account storage checks; run with AccountDataStore.swift via scripts/test-swift-pure.sh.
import Foundation

/// All operations used by the store stay in this dictionary; no preference suite is created.
final class MemoryDefaults: UserDefaults, @unchecked Sendable {
    private var values: [String: Any] = [:]
    private(set) var writtenKeys: [String] = []
    override func object(forKey key: String) -> Any? { values[key] }
    override func set(_ value: Any?, forKey key: String) {
        values[key] = value
        writtenKeys.append(key)
    }
    override func set(_ value: Bool, forKey key: String) { set(value as Any, forKey: key) }
    override func set(_ value: Double, forKey key: String) { set(value as Any, forKey: key) }
    override func bool(forKey key: String) -> Bool { object(forKey: key) as? Bool ?? false }
    override func removeObject(forKey key: String) { values.removeValue(forKey: key) }
    override func dictionaryRepresentation() -> [String: Any] { values }
}

var assertions = 0
var failures = 0
func check(_ condition: Bool, _ message: String) {
    assertions += 1
    if !condition {
        failures += 1
        print("FAIL account-data: \(message)")
    }
}

struct Payload: Codable, Equatable {
    let marker: String
    let value: Int
}

enum EncodingFailure: Error { case expected }
struct Unencodable: Encodable {
    func encode(to encoder: Encoder) throws { throw EncodingFailure.expected }
}

let A = "0x00000000000000000000000000000000000000aa"
let mixedA = "0x00000000000000000000000000000000000000Aa"
let B = "0x00000000000000000000000000000000000000bb"
let first = Payload(marker: "first account", value: 1)
let second = Payload(marker: "second account", value: 2)
let nextChain = Payload(marker: "other chain", value: 3)
let defaults = MemoryDefaults()
let a = AccountDataStore(chainID: 7780, address: A, defaults: defaults)
let b = AccountDataStore(chainID: 7780, address: B, defaults: defaults)
let otherChain = AccountDataStore(chainID: 7781, address: A, defaults: defaults)
let alias = AccountDataStore(chainID: 7780, address: mixedA, defaults: defaults)
let reopened = AccountDataStore(chainID: 7780, address: A, defaults: defaults)
let buckets: [AccountDataStore.Bucket] = [.balanceHistory, .activity, .tokenHoldings, .tokenChoices,
                                         .contacts, .linkedWallets, .incomingNotice, .pendingRecovery]

for bucket in buckets {
    check(alias.key(bucket) == "\(bucket.rawValue).7780.\(A)", "\(bucket.rawValue) key normalizes address")
}

for bucket in [AccountDataStore.Bucket.balanceHistory, .activity, .tokenHoldings, .tokenChoices, .incomingNotice, .pendingRecovery] {
    try a.save(first, to: bucket)
    try b.save(second, to: bucket)
    try otherChain.save(nextChain, to: bucket)
    check(a.load(bucket, as: Payload.self) == first, "\(bucket.rawValue) retains account A")
    check(b.load(bucket, as: Payload.self) == second, "\(bucket.rawValue) retains account B")
    check(otherChain.load(bucket, as: Payload.self) == nextChain, "\(bucket.rawValue) isolates chains")
    check(alias.load(bucket, as: Payload.self) == first, "\(bucket.rawValue) shares canonical casing")
    check(reopened.load(bucket, as: Payload.self) == first, "\(bucket.rawValue) survives store reopening")
}

let contactA = WalletContact(id: UUID(uuidString: "00000000-0000-0000-0000-000000000001")!, name: "Alice", address: B)
let contactB = WalletContact(id: UUID(uuidString: "00000000-0000-0000-0000-000000000002")!, name: "Bob", address: A)
let contactChain = WalletContact(id: UUID(uuidString: "00000000-0000-0000-0000-000000000003")!, name: "Other chain", address: B)
try a.save([contactA], to: .contacts)
try b.save([contactB], to: .contacts)
try otherChain.save([contactChain], to: .contacts)
check(a.load(.contacts, as: [WalletContact].self) == [contactA], "contacts isolate account A")
check(b.load(.contacts, as: [WalletContact].self) == [contactB], "contacts isolate account B")
check(otherChain.load(.contacts, as: [WalletContact].self) == [contactChain], "contacts isolate chains")
check(alias.load(.contacts, as: [WalletContact].self) == [contactA], "contacts share canonical casing")
check(reopened.load(.contacts, as: [WalletContact].self) == [contactA], "contacts and UUID survive reopening")

a.set(["A linked wallet"], for: .linkedWallets)
b.set(["B linked wallet"], for: .linkedWallets)
otherChain.set(["other chain linked wallet"], for: .linkedWallets)
check(a.object(.linkedWallets) as? [String] == ["A linked wallet"], "raw records isolate account A")
check(b.object(.linkedWallets) as? [String] == ["B linked wallet"], "raw records isolate account B")
check(otherChain.object(.linkedWallets) as? [String] == ["other chain linked wallet"], "raw records isolate chains")
check(alias.object(.linkedWallets) as? [String] == ["A linked wallet"], "raw records share canonical casing")
check(reopened.object(.linkedWallets) as? [String] == ["A linked wallet"], "raw records survive reopening")

a.remove(.activity)
check(a.object(.activity) == nil, "remove clears the selected bucket")
check(b.load(.activity, as: Payload.self) == second, "remove keeps another account")
check(otherChain.load(.activity, as: Payload.self) == nextChain, "remove keeps another chain")
check(alias.object(.activity) == nil, "remove applies across casing aliases")
a.set(nil, for: .pendingRecovery)
check(a.object(.pendingRecovery) == nil, "nil raw value removes the bucket")
a.set(Data("invalid json".utf8), for: .incomingNotice)
check(a.load(.incomingNotice, as: Payload.self) == nil, "invalid JSON loads no record")
do {
    try b.save(Unencodable(), to: .tokenHoldings)
    check(false, "encoding failure is surfaced")
} catch EncodingFailure.expected {
    check(true, "encoding failure is surfaced")
}
check(b.load(.tokenHoldings, as: Payload.self) == second, "encoding failure preserves the saved value")

let legacy = MemoryDefaults()
let primary = AccountDataStore(chainID: 7780, address: mixedA, defaults: legacy)
let old = Payload(marker: "address-only legacy", value: 10)
let scoped = Payload(marker: "chain scoped legacy", value: 20)
let replacement = Payload(marker: "new legacy value", value: 30)
let legacyBuckets: [AccountDataStore.Bucket] = [.balanceHistory, .activity, .tokenHoldings]
let oldData = try JSONEncoder().encode(old)
let scopedData = try JSONEncoder().encode(scoped)
for bucket in legacyBuckets {
    legacy.set(oldData, forKey: "\(bucket.rawValue).\(mixedA)")
    legacy.set(scopedData, forKey: "\(bucket.rawValue).7780.\(mixedA)")
}
legacy.set([B], forKey: "linkedWallets.\(mixedA)")
legacy.set(scopedData, forKey: "tokenChoices.7780")
legacy.set(try JSONEncoder().encode([contactA]), forKey: "contacts")
legacy.set(["lost": A, "to": B], forKey: "pendingRecovery")
legacy.set(100.0, forKey: "incomingNotice.7780.\(mixedA)")

// Simulate an interrupted copy before its completion marker was written.
primary.set(try JSONEncoder().encode(first), for: .activity)
let resumed = AccountDataStore(chainID: 7780, address: A, defaults: legacy)
resumed.migrateLegacy(isPrimary: true)
check(resumed.load(.tokenHoldings, as: Payload.self) == scoped, "interrupted migration fills a missing destination after reopening")
check(resumed.load(.activity, as: Payload.self) == first, "resumed migration keeps an existing destination")
check(legacy.writtenKeys.last == "accountDataMigrated.7780.\(A)", "migration records completion only after every copy")
let firstRecoveryClaim = legacy.object(forKey: "pendingRecoveryMigrationClaim") as? [String: Any]
check(firstRecoveryClaim?["chainID"] as? UInt64 == 7780, "legacy recovery claim retains its original chain")
check(firstRecoveryClaim?["address"] as? String == A, "legacy recovery claim retains its canonical owner")
let claimWrite = legacy.writtenKeys.firstIndex(of: "pendingRecoveryMigrationClaim")
let recoveryWrite = legacy.writtenKeys.firstIndex(of: primary.key(.pendingRecovery))
check((claimWrite ?? Int.max) < (recoveryWrite ?? -1), "legacy recovery is claimed before its scoped copy")

for bucket in legacyBuckets {
    check(primary.load(bucket, as: Payload.self) == (bucket == .activity ? first : scoped), "\(bucket.rawValue) prefers existing or scoped legacy data")
    check(legacy.object(forKey: "\(bucket.rawValue).\(mixedA)") as? Data == oldData, "\(bucket.rawValue) preserves address-only legacy data")
    check(legacy.object(forKey: "\(bucket.rawValue).7780.\(mixedA)") as? Data == scopedData, "\(bucket.rawValue) preserves scoped legacy data")
}
check(primary.object(.linkedWallets) as? [String] == [B], "linked wallets migrate to the chain and account")
check(legacy.object(forKey: "linkedWallets.\(mixedA)") as? [String] == [B], "linked wallet migration preserves legacy data")
check(primary.load(.tokenChoices, as: Payload.self) == scoped, "primary inherits chain token choices")
check(primary.load(.contacts, as: [WalletContact].self) == [contactA], "primary inherits global contacts")
check(primary.object(.pendingRecovery) as? [String: String] == ["lost": A, "to": B], "primary inherits global pending recovery")
check(legacy.object(forKey: "tokenChoices.7780") as? Data == scopedData, "migration preserves global token choices")
check(legacy.object(forKey: "contacts") as? Data == (try JSONEncoder().encode([contactA])), "migration preserves global contacts")
check(legacy.object(forKey: "pendingRecovery") as? [String: String] == ["lost": A, "to": B], "migration preserves global pending recovery")
check(primary.object(.incomingNotice) as? Double == 100.0, "incoming notice casing migrates")
check(legacy.object(forKey: "incomingNotice.7780.\(mixedA)") as? Double == 100.0, "incoming notice legacy data stays intact")

let snapshot = buckets.map { primary.object($0) as? NSObject }
for bucket in legacyBuckets {
    legacy.set(try JSONEncoder().encode(replacement), forKey: "\(bucket.rawValue).7780.\(mixedA)")
}
legacy.set(["changed"], forKey: "linkedWallets.\(mixedA)")
legacy.set(try JSONEncoder().encode(replacement), forKey: "tokenChoices.7780")
legacy.set(try JSONEncoder().encode([contactB]), forKey: "contacts")
legacy.set(["lost": B], forKey: "pendingRecovery")
legacy.set(200.0, forKey: "incomingNotice.7780.\(mixedA)")
primary.migrateLegacy(isPrimary: true)
for (index, bucket) in buckets.enumerated() {
    check((primary.object(bucket) as? NSObject) == snapshot[index], "\(bucket.rawValue) migration is idempotent and never overwrites")
}

check(legacy.object(forKey: "accountDataMigrated.7780.\(A)") as? Bool == true, "completed migration records its chain and account")
primary.remove(.pendingRecovery)
primary.remove(.contacts)
primary.remove(.tokenHoldings)
primary.migrateLegacy(isPrimary: true)
check(primary.object(.pendingRecovery) == nil, "cleared recovery stays cleared when migration reruns")
check(primary.object(.contacts) == nil, "deleted contacts stay deleted when migration reruns")
check(primary.object(.tokenHoldings) == nil, "missing buckets are retried only before migration completes")
let afterClear = AccountDataStore(chainID: 7780, address: mixedA, defaults: legacy)
afterClear.migrateLegacy(isPrimary: true)
check(afterClear.object(.pendingRecovery) == nil, "cleared recovery stays cleared after reopening")
check(afterClear.object(.contacts) == nil, "deleted contacts stay deleted after reopening")
check(afterClear.object(.tokenHoldings) == nil, "completed migration does not refill removed holdings after reopening")

let secondary = AccountDataStore(chainID: 7780, address: B, defaults: legacy)
legacy.set(oldData, forKey: "activity.\(B)")
legacy.set([A], forKey: "linkedWallets.\(B)")
secondary.migrateLegacy(isPrimary: false)
check(secondary.load(.activity, as: Payload.self) == old, "secondary may inherit its own address-only activity")
check(secondary.object(.linkedWallets) as? [String] == [A], "secondary may inherit its own linked wallets")
check(secondary.object(.tokenChoices) == nil, "secondary does not inherit global token choices")
check(secondary.object(.contacts) == nil, "secondary does not inherit global contacts")
check(secondary.object(.pendingRecovery) == nil, "secondary does not inherit global pending recovery")

let nonLegacyChain = AccountDataStore(chainID: 7781, address: A, defaults: legacy)
legacy.set(oldData, forKey: "tokenChoices.7781")
nonLegacyChain.migrateLegacy(isPrimary: true)
for bucket in legacyBuckets {
    check(nonLegacyChain.object(bucket) == nil, "\(bucket.rawValue) address-only data belongs only to chain 7780")
}
check(nonLegacyChain.load(.tokenChoices, as: Payload.self) == old, "global token choices remain chain scoped")
check(nonLegacyChain.object(.linkedWallets) as? [String] == ["changed"], "legacy linked wallets can migrate on another chain")
check(nonLegacyChain.object(.pendingRecovery) == nil, "network switching never adopts another chain's global recovery")
check(nonLegacyChain.load(.contacts, as: [WalletContact].self) == [contactB], "legacy primary contacts remain independent of the recovery claim")

let recovery: [String: String] = ["lost": A, "to": B]
let retainedClaim: [String: Any] = ["chainID": UInt64(7780), "address": mixedA]
let interruptedRecovery = MemoryDefaults()
interruptedRecovery.set(recovery, forKey: "pendingRecovery")
interruptedRecovery.set(retainedClaim, forKey: "pendingRecoveryMigrationClaim")
let matchingClaim = AccountDataStore(chainID: 7780, address: A, defaults: interruptedRecovery)
matchingClaim.migrateLegacy(isPrimary: true)
check(matchingClaim.object(.pendingRecovery) as? [String: String] == recovery, "interrupted matching claim resumes the recovery copy")
check((interruptedRecovery.object(forKey: "pendingRecoveryMigrationClaim") as? [String: Any])?["address"] as? String == mixedA, "matching recovery claim is never rewritten")
check(interruptedRecovery.object(forKey: "pendingRecovery") as? [String: String] == recovery, "claim resumption preserves the legacy recovery dictionary")
matchingClaim.remove(.pendingRecovery)
let reopenedClaim = AccountDataStore(chainID: 7780, address: A, defaults: interruptedRecovery)
reopenedClaim.migrateLegacy(isPrimary: true)
check(reopenedClaim.object(.pendingRecovery) == nil, "cleared matching claim stays cleared after reopening")

let otherOwnerDefaults = MemoryDefaults()
otherOwnerDefaults.set(recovery, forKey: "pendingRecovery")
otherOwnerDefaults.set(retainedClaim, forKey: "pendingRecoveryMigrationClaim")
let otherOwnerPrimary = AccountDataStore(chainID: 7780, address: B, defaults: otherOwnerDefaults)
otherOwnerPrimary.migrateLegacy(isPrimary: true)
check(otherOwnerPrimary.object(.pendingRecovery) == nil, "primary migration cannot adopt recovery claimed by another owner")
check((otherOwnerDefaults.object(forKey: "pendingRecoveryMigrationClaim") as? [String: Any])?["address"] as? String == mixedA, "another owner cannot overwrite the retained recovery claim")

let unclaimedRecovery = MemoryDefaults()
unclaimedRecovery.set(recovery, forKey: "pendingRecovery")
let secondaryFirst = AccountDataStore(chainID: 7781, address: B, defaults: unclaimedRecovery)
secondaryFirst.migrateLegacy(isPrimary: false)
check(unclaimedRecovery.object(forKey: "pendingRecoveryMigrationClaim") == nil, "secondary migration leaves global recovery unclaimed")
check(secondaryFirst.object(.pendingRecovery) == nil, "secondary cannot adopt unclaimed global recovery")
let primaryLater = AccountDataStore(chainID: 7780, address: A, defaults: unclaimedRecovery)
primaryLater.migrateLegacy(isPrimary: true)
let laterClaim = unclaimedRecovery.object(forKey: "pendingRecoveryMigrationClaim") as? [String: Any]
check(laterClaim?["chainID"] as? UInt64 == 7780 && laterClaim?["address"] as? String == A, "primary can claim recovery after an earlier secondary load")
check(primaryLater.object(.pendingRecovery) as? [String: String] == recovery, "primary adopts recovery after an earlier secondary load")

let caseDefaults = MemoryDefaults()
caseDefaults.set(scopedData, forKey: "activity.7780.\(mixedA)")
let canonicalOnly = AccountDataStore(chainID: 7780, address: A, defaults: caseDefaults)
canonicalOnly.migrateLegacy(isPrimary: false)
check(canonicalOnly.load(.activity, as: Payload.self) == scoped, "migration finds legacy casing after the account address was canonicalized")

let isolatedDefaults = MemoryDefaults()
isolatedDefaults.set(scopedData, forKey: "activity.7781.\(mixedA)")
isolatedDefaults.set(scopedData, forKey: "tokenHoldings.7780.\(A)ff")
let isolated = AccountDataStore(chainID: 7780, address: A, defaults: isolatedDefaults)
isolated.migrateLegacy(isPrimary: false)
check(isolated.object(.activity) == nil, "migration never takes a different chain's data")
check(isolated.object(.tokenHoldings) == nil, "migration matches the complete address")

let ownerlessDefaults = MemoryDefaults()
ownerlessDefaults.set(try JSONEncoder().encode([contactA]), forKey: "contacts")
ownerlessDefaults.set(scopedData, forKey: "tokenChoices.7780")
ownerlessDefaults.set(recovery, forKey: "pendingRecovery")
let ownerless = AccountDataStore(chainID: 7780, address: "", defaults: ownerlessDefaults)
let writesBeforeOwnerless = ownerlessDefaults.writtenKeys.count
ownerless.migrateLegacy(isPrimary: true)
let ownerlessImportedNothing = buckets.allSatisfy { ownerless.object($0) == nil }
    && ownerlessDefaults.writtenKeys.count == writesBeforeOwnerless
    && ownerlessDefaults.object(forKey: "accountDataMigrated.7780.") == nil
    && ownerlessDefaults.object(forKey: "pendingRecoveryMigrationClaim") == nil
let availableOwner = AccountDataStore(chainID: 7780, address: A, defaults: ownerlessDefaults)
availableOwner.migrateLegacy(isPrimary: true)
let availableClaim = ownerlessDefaults.object(forKey: "pendingRecoveryMigrationClaim") as? [String: Any]
let validOwnerAdopted = availableOwner.load(.contacts, as: [WalletContact].self) == [contactA]
    && availableOwner.load(.tokenChoices, as: Payload.self) == scoped
    && availableOwner.object(.pendingRecovery) as? [String: String] == recovery
    && availableClaim?["chainID"] as? UInt64 == 7780 && availableClaim?["address"] as? String == A
check(ownerlessImportedNothing && validOwnerAdopted, "empty owner cannot import or claim legacy data, and a valid owner can adopt afterward")

if failures > 0 {
    print("account-data: \(failures)/\(assertions) checks failed")
    exit(1)
}
print("account-data OK (\(assertions) checks)")
