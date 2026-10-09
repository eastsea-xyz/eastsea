import Foundation

/// Non-secret wallet records, isolated by both chain and account address.
struct AccountDataStore {
    enum Bucket: String, CaseIterable {
        case balanceHistory, activity, tokenHoldings, tokenChoices
        case contacts, linkedWallets, incomingNotice, pendingRecovery
    }

    let chainID: UInt64
    let address: String
    private let originalAddress: String
    private let defaults: UserDefaults

    init(chainID: UInt64, address: String, defaults: UserDefaults = .standard) {
        self.chainID = chainID
        self.address = address.lowercased()
        self.originalAddress = address
        self.defaults = defaults
    }

    func key(_ bucket: Bucket) -> String { "\(bucket.rawValue).\(chainID).\(address)" }

    func load<T: Decodable>(_ bucket: Bucket, as type: T.Type) -> T? {
        guard let data = object(bucket) as? Data else { return nil }
        return try? JSONDecoder().decode(type, from: data)
    }

    func save<T: Encodable>(_ value: T, to bucket: Bucket) throws {
        let data = try JSONEncoder().encode(value)
        set(data, for: bucket)
    }

    func object(_ bucket: Bucket) -> Any? { defaults.object(forKey: key(bucket)) }
    func set(_ value: Any?, for bucket: Bucket) { defaults.set(value, forKey: key(bucket)) }
    func remove(_ bucket: Bucket) { defaults.removeObject(forKey: key(bucket)) }

    /// Copy only missing destinations so interrupted migrations can safely rerun.
    /// Original entries stay available to earlier wallet versions and backups.
    func migrateLegacy(isPrimary: Bool) {
        guard !address.isEmpty else { return }
        let migrationKey = "accountDataMigrated.\(chainID).\(address)"
        guard defaults.object(forKey: migrationKey) as? Bool != true else { return }
        let existingKeys = defaults.dictionaryRepresentation().keys.sorted()
        for bucket in Bucket.allCases {
            copyIfMissing(bucket, from: addressKeys(prefix: "\(bucket.rawValue).\(chainID).", existing: existingKeys))
        }
        // The address-only history format predates chain switching and belongs to 7780.
        if chainID == 7780 {
            for bucket in [Bucket.balanceHistory, .activity, .tokenHoldings] {
                copyIfMissing(bucket, from: addressKeys(prefix: "\(bucket.rawValue).", existing: existingKeys))
            }
        }
        copyIfMissing(.linkedWallets, from: addressKeys(prefix: "linkedWallets.", existing: existingKeys))
        if isPrimary {
            copyIfMissing(.tokenChoices, from: ["tokenChoices.\(chainID)"])
            copyIfMissing(.contacts, from: ["contacts"])
            migratePendingRecovery()
        }
        // Write this last: interruption can resume copies, but a later intentional
        // clear must not resurrect data from the preserved legacy entries.
        defaults.set(true, forKey: migrationKey)
    }

    private func migratePendingRecovery() {
        guard defaults.object(forKey: "pendingRecovery") != nil else { return }
        let claimKey = "pendingRecoveryMigrationClaim"
        if let existing = defaults.object(forKey: claimKey) {
            guard let claim = existing as? [String: Any],
                  claim["chainID"] as? UInt64 == chainID,
                  (claim["address"] as? String)?.lowercased() == address else { return }
        } else {
            // The legacy dictionary has no chain metadata. Its first primary
            // load binds it to the selected legacy network before copying it.
            let claim: [String: Any] = ["chainID": chainID, "address": address]
            defaults.set(claim, forKey: claimKey)
        }
        copyIfMissing(.pendingRecovery, from: ["pendingRecovery"])
    }

    private func addressKeys(prefix: String, existing keys: [String]) -> [String] {
        // An account index may already contain a lowercase address while its old
        // preference keys retain checksum casing. Match the complete suffix.
        [prefix + originalAddress] + keys.filter {
            $0.hasPrefix(prefix) && $0.dropFirst(prefix.count).lowercased() == address
        }
    }

    private func copyIfMissing(_ bucket: Bucket, from candidates: [String]) {
        guard object(bucket) == nil else { return }
        for source in candidates {
            if let value = defaults.object(forKey: source) {
                set(value, for: bucket)
                return
            }
        }
    }
}

struct WalletContact: Identifiable, Codable, Equatable {
    let id: UUID
    var name: String
    var address: String

    init(id: UUID = UUID(), name: String, address: String) {
        self.id = id
        self.name = name
        self.address = address
    }
}
