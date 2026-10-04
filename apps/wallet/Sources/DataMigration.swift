import Foundation

/// The 2026-10 rename (Aether → EastSea) moved this app's on-disk home and
/// its bundle id (`com.pipln.aether` → `com.pipln.eastsea`). One-time,
/// first-launch migration: bring the old data forward, **verified** — the
/// done flag is set only after every precious item (the wallet key handles,
/// the validator identity and threshold share, the vote journal, the node
/// database) exists at its new home and matches the old copy (audit 5,
/// A5-5). A failed or deferred run leaves the old data exactly where it was
/// and retries on the next launch; nothing is ever deleted except by a
/// successful same-volume move of the (large) chain data (A5-7).
enum DataMigration {
    enum Outcome: Equatable {
        /// No old install found (fresh install): nothing to move.
        case noOldData
        /// Everything moved and verified.
        case done
        /// The old app is still running (it holds `run.lock`): untouched,
        /// retried on the next launch.
        case deferred(String)
        /// Something failed or did not verify: the old data is intact, the
        /// done flag is NOT set, the next launch retries.
        case failed(String)
    }

    private static let oldAppID = "com.pipln.aether"
    private static let doneKey = "renameMigrationDone"
    /// The old node's live lock. Never copied (a copied lock file is not a
    /// lock); it moves with a same-volume move, like every other file.
    private static let lockName = "run.lock"
    /// Left in an old tree that had to be copied (not moved): the old app
    /// cannot be changed to know about the rename, so the marker is what
    /// tells a human this tree is no longer the live one.
    private static let markerName = "MIGRATED-TO-EASTSEA"
    /// Files at or under this size are compared by SHA-256, not size alone
    /// (keys, journals, configs). Larger files (the chain database) are
    /// compared by the tree manifest: same paths, same sizes.
    private static let hashLimit = 4 << 20

    private static var fm: FileManager { FileManager.default }

    /// Where the app keeps its data (the sandboxed Application Support).
    static var supportURL: URL {
        fm.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
    }

    /// Idempotent: the first caller runs the migration, later calls return.
    @discardableResult
    static func ensure() -> Outcome {
        migrate(support: supportURL, defaults: UserDefaults.standard)
    }

    /// The migration proper, on injectable roots so the pure-Swift test can
    /// run it against throwaway directories and its own defaults domain.
    static func migrate(support: URL, defaults: UserDefaults) -> Outcome {
        guard !defaults.bool(forKey: doneKey) else { return .done }
        let oldNode = support.appending(path: "Aether/node")
        let newNode = support.appending(path: "EastSea/node")
        let oldHandles = ["enclave-key.dat", "simulator-software-key.dat"]
            .map { support.appending(path: "AetherWallet/\($0)") }
        let oldState = support.appending(path: "Aether/update-state.json")
        let hasOld = fm.fileExists(atPath: oldNode.path)
            || oldHandles.contains { fm.fileExists(atPath: $0.path) }
            || fm.fileExists(atPath: oldState.path)
        if !hasOld {
            copyOldPreferences(into: defaults)
            defaults.set(true, forKey: doneKey)
            return .noOldData
        }
        if let why = destinationProblem(support) { return .failed(why) }

        // The old node must not be running while its data moves: hold its
        // own run.lock exclusively first (red team #12's lock, reused).
        var lockFD: Int32?
        if fm.fileExists(atPath: oldNode.path) {
            switch tryHoldLock(oldNode.appending(path: lockName)) {
            case .held(let fd):
                lockFD = fd
            case .busy:
                return .deferred("Quit the old Aether app first — it is running (it holds the old data's run.lock). "
                    + "Nothing was moved; the move happens on the next launch.")
            case .broken(let why):
                return .failed("cannot lock the old data directory (\(why)); nothing was changed — "
                    + "the next launch retries")
            }
        }
        defer { if let fd = lockFD { close(fd) } }

        var problems: [String] = []
        if fm.fileExists(atPath: oldNode.path) {
            // A tree already at its new home is the resume path of an
            // interrupted run (or a cross-volume fallback): verified copy,
            // old tree kept and marked. A missing one moves outright.
            let movedIn = fm.fileExists(atPath: newNode.path)
                ? syncTreeVerified(oldNode, newNode) && markOldTreeMigrated(oldNode)
                : moveTreeVerified(oldNode, newNode)
            if !movedIn { problems.append("the node data (identity, share, journal, database) did not move or did not verify") }
        }
        let smallFiles = oldHandles.map {
            (old: $0, new: support.appending(path: "EastSeaWallet/\($0.lastPathComponent)"))
        } + [(old: oldState, new: support.appending(path: "EastSea/update-state.json"))]
        for (old, new) in smallFiles where fm.fileExists(atPath: old.path) {
            if !copyVerified(old, new) { problems.append("\(old.lastPathComponent) did not copy or did not verify") }
        }
        guard problems.isEmpty else {
            return .failed("migration incomplete: \(problems.joined(separator: "; ")). "
                + "Nothing was deleted; the next launch retries.")
        }
        copyOldPreferences(into: defaults)
        defaults.set(true, forKey: doneKey)
        return .done
    }

    /// Audit 5, A5-7: while an unmigrated old identity exists, making a fresh
    /// one would strand it (a second wallet address; a validator that can
    /// never vote). `nil` = allowed. `loadOrCreate` refuses on a message.
    static func mayCreateFreshWalletKey(support: URL? = nil, defaults: UserDefaults = .standard) -> String? {
        let s = support ?? supportURL
        if defaults.bool(forKey: doneKey) { return nil }
        for name in ["enclave-key.dat", "simulator-software-key.dat"] {
            let old = s.appending(path: "AetherWallet/\(name)")
            let new = s.appending(path: "EastSeaWallet/\(name)")
            if fm.fileExists(atPath: old.path) && !fm.fileExists(atPath: new.path) {
                return "the old Aether wallet key is still waiting to move into place. Launch the app again to "
                    + "finish the data move (if it says to quit the old Aether app, quit it first) — a new key now "
                    + "would give this Mac a second wallet address and strand the first."
            }
        }
        return nil
    }

    /// Audit 5, A5-7: the node must not start a fresh data directory while
    /// the old one (identity, threshold share, chain) waits unmigrated.
    static func mayStartNode(support: URL? = nil, defaults: UserDefaults = .standard) -> String? {
        let s = support ?? supportURL
        if defaults.bool(forKey: doneKey) { return nil }
        if fm.fileExists(atPath: s.appending(path: "Aether/node").path)
            && !fm.fileExists(atPath: s.appending(path: "EastSea/node").path) {
            return "the old Aether node data (validator identity, threshold share, chain) has not moved into place "
                + "yet. Launch the app once more to finish the data move (quit the old Aether app if it asks) — "
                + "starting fresh now would strand this Mac's validator identity."
        }
        return nil
    }

    // MARK: verification

    /// True when both files exist with the same size and — for files within
    /// the hash limit — the same SHA-256. The comparison behind every "done".
    static func fileMatches(_ old: URL, _ new: URL) -> Bool {
        guard let a = size(of: old), let b = size(of: new), a == b else { return false }
        guard a <= hashLimit else { return true }
        guard let da = try? Data(contentsOf: old), let db = try? Data(contentsOf: new) else { return false }
        return sha256Hex(da) == sha256Hex(db)
    }

    /// SHA-256 as lowercase hex (FIPS 180-4). Pure Swift: this file must
    /// compile alone for the pure-Swift test, which allows no crypto
    /// framework — the "abc" and empty vectors pin it in that test.
    static func sha256Hex(_ data: Data) -> String {
        var h: [UInt32] = [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
                           0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19]
        let k: [UInt32] = [
            0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
            0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
            0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
            0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
            0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
            0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
            0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
            0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2]
        var padded = [UInt8](data)
        let bitLength = UInt64(data.count) &* 8
        padded.append(0x80)
        while padded.count % 64 != 56 { padded.append(0) }
        for shift in stride(from: 56, through: 0, by: -8) { padded.append(UInt8((bitLength >> UInt64(shift)) & 0xff)) }
        for chunkStart in stride(from: 0, to: padded.count, by: 64) {
            var w = [UInt32](repeating: 0, count: 64)
            for i in 0..<16 {
                let j = chunkStart + i * 4
                w[i] = UInt32(padded[j]) << 24 | UInt32(padded[j + 1]) << 16 | UInt32(padded[j + 2]) << 8 | UInt32(padded[j + 3])
            }
            for i in 16..<64 {
                let s0 = rotr(w[i - 15], 7) ^ rotr(w[i - 15], 18) ^ (w[i - 15] >> 3)
                let s1 = rotr(w[i - 2], 17) ^ rotr(w[i - 2], 19) ^ (w[i - 2] >> 10)
                w[i] = w[i - 16] &+ s0 &+ w[i - 7] &+ s1
            }
            var (a, b, c, d, e, f, g, hh) = (h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7])
            for i in 0..<64 {
                let s1 = rotr(e, 6) ^ rotr(e, 11) ^ rotr(e, 25)
                let ch = (e & f) ^ (~e & g)
                let t1 = hh &+ s1 &+ ch &+ k[i] &+ w[i]
                let s0 = rotr(a, 2) ^ rotr(a, 13) ^ rotr(a, 22)
                let maj = (a & b) ^ (a & c) ^ (b & c)
                let t2 = s0 &+ maj
                hh = g; g = f; f = e; e = d &+ t1; d = c; c = b; b = a; a = t1 &+ t2
            }
            h[0] = h[0] &+ a; h[1] = h[1] &+ b; h[2] = h[2] &+ c; h[3] = h[3] &+ d
            h[4] = h[4] &+ e; h[5] = h[5] &+ f; h[6] = h[6] &+ g; h[7] = h[7] &+ hh
        }
        return h.map { String(format: "%08x", $0) }.joined()
    }

    private static func rotr(_ x: UInt32, _ n: UInt32) -> UInt32 { (x >> n) | (x << (32 - n)) }

    private static func size(of url: URL) -> Int64? {
        (try? fm.attributesOfItem(atPath: url.path))?[.size] as? Int64
    }

    /// Same-volume move, verified: inventory first, move, inventory again.
    /// A move that cannot happen (cross-volume) falls back to a verified
    /// copy that leaves the old tree in place, marked migrated.
    private static func moveTreeVerified(_ old: URL, _ new: URL) -> Bool {
        guard let before = manifest(of: old) else { return false }
        do {
            try fm.createDirectory(at: new.deletingLastPathComponent(), withIntermediateDirectories: true)
            try fm.moveItem(at: old, to: new)
        } catch {
            return syncTreeVerified(old, new) && markOldTreeMigrated(old)
        }
        guard let after = manifest(of: new) else { return false }
        return before == after
    }

    /// Verified copy of a tree, file by file (the cross-volume fallback and
    /// the resume path of an interrupted earlier run). Already-correct files
    /// are kept; a file that does not match is replaced and re-verified;
    /// `run.lock` is never copied. True only when the whole manifest matches.
    static func syncTreeVerified(_ old: URL, _ new: URL) -> Bool {
        guard let entries = manifest(of: old) else { return false }
        for (rel, bytes) in entries where rel != lockName {
            let o = old.appending(path: rel), n = new.appending(path: rel)
            if let have = size(of: n), have == bytes, fileMatches(o, n) { continue }
            do {
                try fm.createDirectory(at: n.deletingLastPathComponent(), withIntermediateDirectories: true)
                if fm.fileExists(atPath: n.path) { try fm.removeItem(at: n) }   // a partial earlier attempt
                try fm.copyItem(at: o, to: n)
            } catch { return false }
            if !fileMatches(o, n) { return false }
        }
        guard let copied = manifest(of: new) else { return false }
        return copied.filter { $0.key != lockName } == entries.filter { $0.key != lockName }
    }

    /// Copy one small precious file unless a verified copy is already there.
    private static func copyVerified(_ old: URL, _ new: URL) -> Bool {
        if fileMatches(old, new) { return true }
        do {
            try fm.createDirectory(at: new.deletingLastPathComponent(), withIntermediateDirectories: true)
            if fm.fileExists(atPath: new.path) { try fm.removeItem(at: new) }
            try fm.copyItem(at: old, to: new)
        } catch { return false }
        return fileMatches(old, new)
    }

    /// Every file under `root`, as `relative path → size` (the tree's
    /// fingerprint). Built by a name-by-name walk: the enumerator's absolute
    /// paths resolve macOS's `/var` → `/private/var` symlink, which would
    /// otherwise mangle the keys.
    private static func manifest(of root: URL) -> [String: Int64]? {
        var out: [String: Int64] = [:]
        guard walk(root, prefix: "", into: &out) else { return nil }
        return out
    }

    /// `contentsOfDirectory` is unordered; the manifest does not care (it is
    /// a dictionary). Symlinks are leaves — copied as links, never followed.
    private static func walk(_ dir: URL, prefix: String, into out: inout [String: Int64]) -> Bool {
        guard let names = try? fm.contentsOfDirectory(atPath: dir.path) else { return false }
        for name in names {
            let url = dir.appending(path: name)
            let rel = prefix.isEmpty ? name : "\(prefix)/\(name)"
            let isLink = (try? fm.destinationOfSymbolicLink(atPath: url.path)) != nil
            var isDir: ObjCBool = false
            _ = fm.fileExists(atPath: url.path, isDirectory: &isDir)
            if !isLink && isDir.boolValue {
                guard walk(url, prefix: rel, into: &out) else { return false }
            } else if let bytes = size(of: url) {
                out[rel] = bytes
            } else {
                return false
            }
        }
        return true
    }

    private static func markOldTreeMigrated(_ old: URL) -> Bool {
        let when = ISO8601DateFormatter().string(from: Date())
        guard (try? "migrated to EastSea on \(when)".write(to: old.appending(path: markerName),
                                                           atomically: true, encoding: .utf8)) != nil else { return false }
        return true
    }

    // MARK: the safety refusals

    /// Refuse a symlinked old or new home (the move must not write through a
    /// link somebody pointed somewhere else) and any old/new overlap.
    private static func destinationProblem(_ support: URL) -> String? {
        for part in ["Aether", "Aether/node", "AetherWallet", "EastSea", "EastSea/node", "EastSeaWallet"] {
            let p = support.appending(path: part)
            if (try? fm.destinationOfSymbolicLink(atPath: p.path)) != nil {
                return "\(part) is a symbolic link — refusing to migrate through it. Remove the link and relaunch."
            }
        }
        let oldR = support.appending(path: "Aether/node").standardizedFileURL.path
        let newR = support.appending(path: "EastSea/node").standardizedFileURL.path
        if oldR == newR { return "the old and new data paths resolve to the same place (\(oldR))" }
        if newR.hasPrefix(oldR + "/") || oldR.hasPrefix(newR + "/") { return "one data path sits inside the other" }
        return nil
    }

    private enum Lock { case held(Int32); case busy; case broken(String) }

    /// An exclusive non-blocking flock on the old node's run.lock — the same
    /// lock the node itself holds for its whole run (supervisor.rs).
    private static func tryHoldLock(_ path: URL) -> Lock {
        let fd = open(path.path, O_RDWR | O_CREAT, 0o600)
        guard fd >= 0 else { return .broken(String(cString: strerror(errno))) }
        if flock(fd, LOCK_EX | LOCK_NB) == 0 { return .held(fd) }
        close(fd)
        return .busy
    }

    // MARK: preferences

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
