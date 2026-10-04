import Foundation

/// The 2026-10 rename (Aether → EastSea) moved this app's on-disk home and
/// its bundle id (`com.pipln.aether` → `com.pipln.eastsea`). One-time,
/// first-launch migration: bring the old data forward; **never delete the
/// old copy** except by a successful same-volume move of the (large) chain
/// data. Every step is best effort — a fresh install simply has no old
/// paths, and a failed copy only costs the node a re-sync from the network.
enum DataMigration {
    private static let oldAppID = "com.pipln.aether"
    private static let doneKey = "renameMigrationDone"

    /// Idempotent: the first caller runs the migration, later calls return.
    static func ensure() {
        let defaults = UserDefaults.standard
        guard !defaults.bool(forKey: doneKey) else { return }
        defaults.set(true, forKey: doneKey)
        guard let support = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first else { return }

        // Chain data can be gigabytes: move (a rename inside the same
        // volume). If that fails, copy the tree and leave the old one alone.
        moveOrCopyTree(support.appendingPathComponent("Aether/node", isDirectory: true),
                       support.appendingPathComponent("EastSea/node", isDirectory: true))
        // Small state files: copy without overwriting, keep the old files.
        copyIfMissing(support.appendingPathComponent("Aether/update-state.json"),
                      support.appendingPathComponent("EastSea/update-state.json"))
        // The wallet key handle is precious and tiny: copy, never move.
        for name in ["enclave-key.dat", "simulator-software-key.dat"] {
            copyIfMissing(support.appendingPathComponent("AetherWallet/\(name)"),
                          support.appendingPathComponent("EastSeaWallet/\(name)"))
        }
        copyOldPreferences(into: defaults)
    }

    /// Internal (not `private`) so the pure-Swift test can exercise the
    /// move/copy rules on temp directories.
    static func moveOrCopyTree(_ old: URL, _ new: URL) {
        let fm = FileManager.default
        guard fm.fileExists(atPath: old.path), !fm.fileExists(atPath: new.path) else { return }
        do {
            try fm.createDirectory(at: new.deletingLastPathComponent(), withIntermediateDirectories: true)
            do { try fm.moveItem(at: old, to: new) }
            catch { try fm.copyItem(at: old, to: new) }   // cross-volume or partial: copy, keep old
        } catch { /* nothing to forward to; the node re-syncs on its own */ }
    }

    static func copyIfMissing(_ old: URL, _ new: URL) {
        let fm = FileManager.default
        guard fm.fileExists(atPath: old.path), !fm.fileExists(atPath: new.path) else { return }
        do {
            try fm.createDirectory(at: new.deletingLastPathComponent(), withIntermediateDirectories: true)
            try fm.copyItem(at: old, to: new)
        } catch { /* keep using defaults; the file is rewritten on first change */ }
    }

    /// The bundle-id change also moves the UserDefaults domain. Copy the old
    /// domain's keys once (the node on/off switch, terms acceptance, mode),
    /// leaving anything the new install has already written in place.
    private static func copyOldPreferences(into defaults: UserDefaults) {
        guard let keys = CFPreferencesCopyKeyList(oldAppID as CFString,
                                                  kCFPreferencesCurrentUser,
                                                  kCFPreferencesAnyHost) as? [String] else { return }
        for key in keys where defaults.object(forKey: key) == nil {
            guard let value = CFPreferencesCopyValue(key as CFString, oldAppID as CFString,
                                                     kCFPreferencesCurrentUser, kCFPreferencesAnyHost) else { continue }
            defaults.set(value, forKey: key)
        }
    }
}
