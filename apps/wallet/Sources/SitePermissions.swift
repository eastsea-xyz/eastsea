import Foundation

/// One external site's standing with the Explore tab's browser (docs/design/
/// 09-wallet.md "인앱 브라우저"): it may see this wallet's address until the
/// user revokes that in Settings ▸ Security. Kept on this device only.
struct SitePermission: Codable, Equatable, Identifiable {
    var id: String { origin }
    let origin: String
    let address: String
    let grantedAt: Date
    var displayOrigin: String? = nil
}

/// The store behind it. Everything except `save`/`load` is pure, so the
/// permission rules (grant, revoke, address-changed invalidation) test
/// without a domain object.
struct SitePermissionStore: Equatable {
    private(set) var sites: [SitePermission] = []

    /// The address this site may see: nil while disconnected, and nil once
    /// the wallet holds a different account than the one the permission was
    /// granted for (the extension's rule — a permission follows its address,
    /// it is never silently re-pointed at a new one).
    func connectedAddress(origin: String, current: String) -> String? {
        guard let s = sites.first(where: { $0.origin == origin }) else { return nil }
        return s.address.lowercased() == current.lowercased() ? s.address : nil
    }

    mutating func grant(origin: String, address: String, at date: Date = Date(), displayOrigin: String? = nil) {
        revoke(origin: origin)
        sites.insert(SitePermission(origin: origin, address: address, grantedAt: date, displayOrigin: displayOrigin), at: 0)
    }

    mutating func revoke(origin: String) {
        sites.removeAll { $0.origin == origin }
    }

    /// A wallet that switched accounts is connected to nothing (the grants
    /// above still name the old address; the user connects sites again).
    mutating func revokeAll() {
        sites.removeAll()
    }

    static let key = "explore.sites"

    /// Persisted in UserDefaults (an address a user chose to share, not a
    /// secret — the same line the app's other settings walk).
    func save(defaults: UserDefaults = .standard) {
        defaults.set(try? JSONEncoder().encode(sites), forKey: Self.key)
    }

    @discardableResult
    mutating func load(defaults: UserDefaults = .standard) -> Bool {
        guard let data = defaults.data(forKey: Self.key),
              let loaded = try? JSONDecoder().decode([SitePermission].self, from: data) else { return false }
        sites = loaded
        return true
    }
}
