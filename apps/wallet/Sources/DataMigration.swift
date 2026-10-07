import Foundation

/// The 2026-10 rename (Aether → EastSea) moved this app's on-disk home and
/// its bundle id (`com.pipln.aether` → `com.pipln.eastsea`). One-time,
/// first-launch migration: bring the old data forward, **verified** — the
/// done flag is set only after every precious item (the wallet key handles,
/// the validator identity and threshold share, the vote journal, the node
/// database) exists at its new home and checks out (audit 5, A5-5).
///
/// How the node tree checks out depends on the path it takes, chosen
/// explicitly by comparing volume identifiers (release-070 review, M2):
/// - same volume: one atomic `rename(2)` of the whole tree, checked by a
///   name → size manifest before and after (a rename moves no bytes, so there
///   is nothing to hash). Nothing is ever deleted: the old name simply stops
///   existing at the instant the new one appears.
/// - different volumes (or a resume over an existing new tree): a copy that
///   is SHA-256-checked file by file, however large (audit 6, A6-6). The old
///   tree is then left in place but emptied of everything its old binary
///   could sign with (audit 6, A6-5) — moved into a quarantine directory
///   inside it, never deleted.
/// A failed or deferred run leaves the old data where it was and retries on
/// the next launch. A done flag never outvotes the disk (release-070 review,
/// B4): while old data still lacks its new counterpart, the migration runs
/// again and the identity gates stay shut. The slow (hashing) path runs off
/// the main thread with visible progress (M1); every run is serialized (L2).
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
        /// The verified copy is running in the background (M1): the gates
        /// stay shut until it finishes, and the app shows its progress.
        case running(String)
        /// Everything that could move did; what is left (the wallet key
        /// handle, complete file protection) cannot be read while the Mac is
        /// locked (poc-m3, 2026-10-07). Not settled: retried on unlock.
        case waitingForUnlock(String)
    }

    static func unlockSentence(locale: Locale = .current, bundle: Bundle = .main) -> String {
        String(localized: "Unlock this Mac to finish moving your wallet. Your node data has already moved; the wallet key file can only be read while the Mac is unlocked. Nothing was deleted.", bundle: bundle, locale: locale)
    }

    static var unlockSentence: String { unlockSentence() }

    static let oldAppID = "com.pipln.aether"
    static let doneKey = "renameMigrationDone"
    /// The node tree's own completion state (pre-audit 7, H2): set the moment
    /// the old tree's signing material is quarantined, before the cosmetic
    /// marker or the preference copy. A wallet-preference step that fails
    /// after it only delays the done flag — the verified new node may start,
    /// because nothing signing-related is left to retry.
    private static let nodeDoneKey = "renameNodeMigrationDone"
    /// The old node's live lock. Never copied (a copied lock file is not a
    /// lock); it moves with a same-volume move, like every other file.
    private static let lockName = "run.lock"
    /// Left in an old tree that had to be copied (not moved): the old app
    /// cannot be changed to know about the rename, so the marker is what
    /// tells a human this tree is no longer the live one.
    private static let markerName = "MIGRATED-TO-EASTSEA"
    /// What an old binary can sign with (audit 6, A6-5): the validator key,
    /// its public file, the account key, the threshold share — and the
    /// journals its consensus would resume from.
    private static let signingMaterial = ["validator.key", "validator.pub.json", "node-account.key", "threshold.json"]
    private static let signingMaterialPrefixes = ["aether-consensus", "dkg-agreement-", "vote-epoch-"]
    /// Signing material below the root (release-070 review, L1): the
    /// follower's own endpoint key. Left behind, the old app would publish a
    /// second endpoint under this Mac's wallet node id.
    private static let nestedSigningMaterial = ["follow/wallet-node.key"]
    /// The node's sibling identity guard (candidate.rs `registered_identity`):
    /// `<data dir>.identity`, beside the tree, surviving its deletion.
    private static let identityGuardSuffix = ".identity"
    /// A conflicting identity found in the new tree (one minted there while a
    /// stale done flag hid the old one, B4) is set aside under this prefix —
    /// recoverable by hand, never deleted.
    private static let replacedPrefix = "eastsea-replaced-"
    /// Quarantined signing material lives in the old tree, under this
    /// prefix: recoverable by hand, and invisible to the node — the
    /// identity marks (candidate.rs) and the network sweep (supervisor.rs)
    /// never look inside a subdirectory for a key.
    private static let quarantinePrefix = "eastsea-quarantine-"

    static var fm: FileManager { FileManager.default }

    /// Where the app keeps its data (the sandboxed Application Support).
    static var supportURL: URL {
        fm.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
    }

    /// Idempotent and serialized (L2): the first caller runs the migration,
    /// concurrent callers wait for it (off the main thread) or get its
    /// `.running` state (on the main thread, which must never block on the
    /// hashing path — M1). Once a run in this process settled, later calls
    /// return at once.
    @discardableResult
    static func ensure() -> Outcome {
        #if WALLET_SCREENS
        // The screens renderer (scripts/wallet-screens.sh) never touches the real data.
        return .noOldData
        #endif
        Thread.isMainThread ? Runner.shared.ensureFromMain() : Runner.shared.runNow()
    }

    /// The migration proper, on injectable roots so the pure-Swift test can
    /// run it against throwaway directories and its own defaults domain.
    /// `oldPreferencesDomain` is the old app's defaults domain (a test passes
    /// one that does not exist, so no real preferences are copied into its
    /// throwaway suite); `forceCopy` takes the cross-volume path on one
    /// volume; `meter` receives the bytes the verified-copy path works through.
    static func migrate(support: URL, defaults: UserDefaults, oldPreferencesDomain: String = oldAppID,
                        forceCopy: Bool = false, meter: ProgressMeter? = nil,
                        oldPreferences: [String: Any]? = nil, locale: Locale = .current, bundle: Bundle = .main) -> Outcome {
        let copyPreferences = {
            if let oldPreferences { copyPreferencesDict(oldPreferences, into: defaults) }
            else { copyOldPreferences(from: oldPreferencesDomain, into: defaults) }
        }
        // B4: the flag is a cache of what the disk says, never a substitute.
        // A flag left over from an earlier build (or data moved back by hand)
        // with old data still lacking its new counterpart means: run again.
        let stale = unmigratedOldData(support: support)
        if defaults.bool(forKey: doneKey) {
            if stale.isEmpty { return .done }
            defaults.removeObject(forKey: doneKey)
            if stale.contains(oldNodeItem) { defaults.removeObject(forKey: nodeDoneKey) }
        }
        let oldNode = support.appending(path: "Aether/node")
        let newNode = support.appending(path: "EastSea/node")
        let oldHandles = walletHandleNames.map { support.appending(path: "AetherWallet/\($0)") }
        let oldState = support.appending(path: "Aether/update-state.json")
        let oldGuard = support.appending(path: "Aether/node\(identityGuardSuffix)")
        let hasOld = fm.fileExists(atPath: oldNode.path)
            || oldHandles.contains { fm.fileExists(atPath: $0.path) }
            || fm.fileExists(atPath: oldState.path)
            || fm.fileExists(atPath: oldGuard.path)
        if !hasOld {
            copyPreferences()
            defaults.set(true, forKey: doneKey)
            return .noOldData
        }
        if let why = destinationProblem(support, locale: locale, bundle: bundle) { return .failed(why) }

        // The old node must not be running while its data moves: hold its
        // own run.lock exclusively first (red team #12's lock, reused).
        var lockFD: Int32?
        if fm.fileExists(atPath: oldNode.path) {
            switch tryHoldLock(oldNode.appending(path: lockName)) {
            case .held(let fd):
                lockFD = fd
            case .busy:
                return .deferred(String(localized: "Quit the old Aether app first — it is running. Nothing was moved; the move happens on the next launch.", bundle: bundle, locale: locale))
            case .broken(let why):
                NSLog("cannot lock the old data directory (%@); nothing was changed; the next launch retries", why)
                return .failed(String(localized: "The old data folder could not be opened. Nothing was changed; the next launch retries.", bundle: bundle, locale: locale))
            }
        }
        defer { if let fd = lockFD { close(fd) } }

        var problems: [String] = []
        /// Problems that are only "unreadable while locked" (EPERM/EACCES).
        var unreadable: [String] = []
        var oldTreeRemains = false
        if fm.fileExists(atPath: oldNode.path) {
            if fm.fileExists(atPath: newNode.path) {
                // A tree already at its new home is the resume path of an
                // interrupted run (or a cross-volume fallback): a verified
                // copy — quarantine happens only at the commit point below,
                // after every other fallible copy has landed (H2).
                oldTreeRemains = true
                setAsideConflictingIdentity(old: oldNode, new: newNode, support: support)
                if !syncTreeVerified(oldNode, newNode, meter: meter) {
                    problems.append("the node data (identity, share, journal, database) did not copy or did not verify")
                }
            } else {
                // A missing new tree moves outright on one volume; across
                // volumes it takes the verified copy, with the same deferred
                // old-tree shutdown (M2: decided by volume identifiers).
                switch moveTreeVerified(oldNode, newNode, forceCopy: forceCopy, meter: meter) {
                case .moved: break
                case .copied: oldTreeRemains = true
                case .failed:
                    oldTreeRemains = fm.fileExists(atPath: oldNode.path)
                    problems.append("the node data (identity, share, journal, database) did not move or did not verify")
                }
            }
        }
        // Every fallible non-node copy finishes and verifies BEFORE anything
        // is quarantined (pre-audit 7, H2): while these can still fail, the
        // old tree keeps everything its binary signs with, so a disk fault
        // here strands nobody — the old app still works and the next launch
        // retries.
        // The wallet key handles must arrive exactly; the update record and
        // the identity guard only need to exist (an existing one is the new
        // app's own, newer state — never overwritten).
        let smallFiles = oldHandles.map {
            (old: $0, new: support.appending(path: "EastSeaWallet/\($0.lastPathComponent)"), exact: true)
        } + [(old: oldState, new: support.appending(path: "EastSea/update-state.json"), exact: false),
             (old: oldGuard, new: support.appending(path: "EastSea/node\(identityGuardSuffix)"), exact: false)]
        for (old, new, exact) in smallFiles where fm.fileExists(atPath: old.path) {
            if !exact && fm.fileExists(atPath: new.path) { continue }
            switch copyVerified(old, new) {
            case .ok: break
            case .unreadable(let code):
                unreadable.append("\(old.lastPathComponent) (errno \(code))")
                problems.append("\(old.lastPathComponent) cannot be read while the Mac is locked")
            case .failed:
                problems.append("\(old.lastPathComponent) did not copy or did not verify")
            }
        }
        let nodeProblem = problems.contains { $0.hasPrefix("the node data") }
        if oldTreeRemains && problems.isEmpty {
            // The commit point: the new tree is verified and every fallible
            // copy has landed, so the old tree now stops being a signer.
            // Each step from here is idempotent and retried on the next
            // launch, and the durable node completion flag goes down the
            // moment the quarantine finishes — a failure after it (the
            // marker write, the preference copy, the done flag) leaves a
            // startable new node and exactly one usable signer.
            if quarantineOldSigningMaterial(oldNode) {
                defaults.set(true, forKey: nodeDoneKey)
                if !markOldTreeMigrated(oldNode) { problems.append("the old tree could not be marked migrated") }
                // The node's half is done: its preferences may arrive now.
                copyPreferences()
            } else {
                problems.append("the old tree's signing material did not move into quarantine "
                    + "(nothing was stranded: the old app still works); the next launch retries")
            }
        }
        // poc-m3: the preferences (the node switch, the accepted terms) used
        // to wait for the wallet handle too, so a locked screen left the node
        // off with no word. They follow the node's half: a moved tree (same
        // volume) or the copy path's commit point above. Non-overwriting and
        // idempotent, and every gate still reads the disk.
        if !nodeProblem && !oldTreeRemains { copyPreferences() }
        guard problems.isEmpty else {
            if problems.count == unreadable.count {
                return .waitingForUnlock(unlockSentence(locale: locale, bundle: bundle))
            }
            NSLog("migration incomplete: %@. Nothing was deleted; the next launch retries.", problems.joined(separator: "; "))
            return .failed(String(localized: "The data move could not be completed. Nothing was deleted; the next launch retries.", bundle: bundle, locale: locale))
        }
        copyPreferences()
        defaults.set(true, forKey: doneKey)
        return .done
    }

    // MARK: what is still waiting (B4)

    private static let walletHandleNames = ["enclave-key.dat", "simulator-software-key.dat"]
    /// `unmigratedOldData` names, for the callers that branch on them.
    static let oldNodeItem = "Aether/node"
    static let oldIdentityGuardItem = "Aether/node.identity"

    /// Old data that still lacks its new counterpart, read from the disk
    /// alone — never from a flag (release-070 review, B4). Empty once the
    /// migration really happened (or there never was old data):
    /// - `Aether/node` while it still holds an identity (a finished migration
    ///   never leaves one at the old root: a same-volume move takes the whole
    ///   tree, a copy quarantines it), or while it exists with no new tree
    ///   and no migrated marker. An identity-less old tree beside an existing
    ///   new one is the old app re-run as a follower, not data to move.
    /// - an old wallet key handle with no handle at the new home.
    /// - the old identity guard `Aether/node.identity` while the new home has
    ///   neither its own guard nor an identity: a node started there now
    ///   would mint a new validator identity.
    static func unmigratedOldData(support: URL) -> [String] {
        var out: [String] = []
        if oldNodeUnmigrated(support) { out.append(oldNodeItem) }
        for name in walletHandleNames {
            let old = support.appending(path: "AetherWallet/\(name)")
            let new = support.appending(path: "EastSeaWallet/\(name)")
            if fm.fileExists(atPath: old.path) && !fm.fileExists(atPath: new.path) { out.append("AetherWallet/\(name)") }
        }
        let oldGuard = support.appending(path: "Aether/node\(identityGuardSuffix)")
        let newGuard = support.appending(path: "EastSea/node\(identityGuardSuffix)")
        if fm.fileExists(atPath: oldGuard.path) && !fm.fileExists(atPath: newGuard.path)
            && !holdsIdentity(support.appending(path: "EastSea/node")) {
            out.append(oldIdentityGuardItem)
        }
        return out
    }

    private static func oldNodeUnmigrated(_ support: URL) -> Bool {
        let old = support.appending(path: "Aether/node")
        guard fm.fileExists(atPath: old.path) else { return false }
        if holdsIdentity(old) { return true }
        return !fm.fileExists(atPath: support.appending(path: "EastSea/node").path)
            && !fm.fileExists(atPath: old.appending(path: markerName).path)
    }

    /// Whether a node root holds identity material at the names the node
    /// itself reads (the validator key and its public file, the account key,
    /// the threshold share, the vote journals).
    private static func holdsIdentity(_ root: URL) -> Bool {
        let names = (try? fm.contentsOfDirectory(atPath: root.path)) ?? []
        return names.contains(where: isRootSigningMaterial)
    }

    private static func isRootSigningMaterial(_ name: String) -> Bool {
        signingMaterial.contains(name) || signingMaterialPrefixes.contains { name.hasPrefix($0) }
    }

    // MARK: the gates

    /// Audit 5, A5-7: while an unmigrated old identity exists, making a fresh
    /// one would strand it (a second wallet address; a validator that can
    /// never vote). `nil` = allowed. `loadOrCreate` refuses on a message.
    /// No done flag can open this gate (B4): only the handle's arrival can.
    static func mayCreateFreshWalletKey(support: URL? = nil, defaults: UserDefaults = .standard,
                                        runner: Runner? = nil, locale: Locale = .current, bundle: Bundle = .main) -> String? {
        if (runner ?? (support == nil ? Runner.shared : nil))?.isRunning == true { return movingSentence(locale: locale, bundle: bundle) }
        let s = support ?? supportURL
        for name in walletHandleNames {
            let old = s.appending(path: "AetherWallet/\(name)")
            let new = s.appending(path: "EastSeaWallet/\(name)")
            if fm.fileExists(atPath: old.path) && !fm.fileExists(atPath: new.path) {
                return String(localized: "Your old Aether wallet key has not moved over yet. Open the app once more to finish the move (if it asks you to quit the old Aether app, quit it first). A new key now would give this Mac a second wallet address and leave the first one behind.", bundle: bundle, locale: locale)
            }
        }
        return nil
    }

    /// Audit 5, A5-7 and audit 6, A6-5/A6-6: the node must not start while
    /// the old one (identity, threshold share, chain) waits unmigrated —
    /// not even onto a partially-copied new tree, and not while a stale done
    /// flag says otherwise (B4). The node's own completion state (pre-audit
    /// 7, H2) is `nodeMigrationComplete`: the verified new tree may start
    /// once the old tree has stopped being a signer, even if a later
    /// wallet-preference step still retries. Everything that can mint a
    /// validator identity in the new home (the node, `candidate-info`, the
    /// unattended daemon's marker) asks this first.
    static func mayStartNode(support: URL? = nil, defaults: UserDefaults = .standard,
                             runner: Runner? = nil, locale: Locale = .current, bundle: Bundle = .main) -> String? {
        if (runner ?? (support == nil ? Runner.shared : nil))?.isRunning == true { return movingSentence(locale: locale, bundle: bundle) }
        let s = support ?? supportURL
        if unmigratedOldData(support: s).contains(oldIdentityGuardItem) {
            return String(localized: "This Mac's node identity from the old Aether app has not moved over yet. Open the app once more to finish the move. Starting now would make a second node.", bundle: bundle, locale: locale)
        }
        if nodeMigrationComplete(support: s, defaults: defaults) { return nil }
        return String(localized: "Your old Aether node data has not finished moving over. Open the app once more to finish the move (quit the old Aether app if it asks). Starting now could run the same node twice.", bundle: bundle, locale: locale)
    }

    static func movingSentence(locale: Locale = .current, bundle: Bundle = .main) -> String {
        String(localized: "EastSea is moving your data over from Aether. This takes a moment; the wallet and the node start as soon as it is done.", bundle: bundle, locale: locale)
    }

    static var movingSentence: String { movingSentence() }

    // MARK: verification

    /// True when both files exist with the same size and the same streamed
    /// SHA-256 — any size, no cutoff: a same-size different-content file is
    /// caught however big it is (audit 6, A6-6). The comparison behind
    /// every "done".
    static func fileMatches(_ old: URL, _ new: URL, meter: ProgressMeter? = nil) -> Bool {
        guard let a = size(of: old), let b = size(of: new), a == b else { return false }
        guard let ha = streamSHA256(old, meter: meter), let hb = streamSHA256(new, meter: meter) else { return false }
        return ha == hb
    }

    /// SHA-256 of a file of any size, read in chunks (audit 6, A6-6): no
    /// size limit and never the whole file in memory — the chain database
    /// is gigabytes. `nil` on any read error: a digest of "whatever we
    /// managed to read" would be a false match waiting to happen.
    private static func streamSHA256(_ url: URL, meter: ProgressMeter? = nil) -> String? {
        var sha = SHA256()
        do {
            let fh = try FileHandle(forReadingFrom: url)
            defer { try? fh.close() }
            while let chunk = try fh.read(upToCount: 1 << 20) {   // nil = end of file
                sha.update(chunk)
                meter?.add(Int64(chunk.count))
            }
        } catch { return nil }
        return sha.finalHex()
    }

    /// SHA-256 as lowercase hex (FIPS 180-4). Pure Swift: this file must
    /// compile alone for the pure-Swift test, which allows no crypto
    /// framework — the "abc" and empty vectors pin it in that test.
    static func sha256Hex(_ data: Data) -> String {
        var sha = SHA256()
        sha.update(data)
        return sha.finalHex()
    }

    /// Incremental SHA-256 (FIPS 180-4). Blocks are consumed as slices of
    /// one buffer with a single `removeSubrange` per update, so streaming a
    /// file stays linear instead of quadratic.
    private struct SHA256 {
        private static let k: [UInt32] = [
            0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
            0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
            0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
            0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
            0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
            0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
            0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
            0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2]

        private var h: [UInt32] = [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
                                   0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19]
        private var buffer = Data()
        private var bytes = UInt64(0)

        mutating func update(_ data: Data) {
            buffer.append(data)
            bytes &+= UInt64(data.count)
            var start = buffer.startIndex
            while buffer.endIndex - start >= 64 {
                compress(buffer[start..<start + 64])
                start += 64
            }
            buffer.removeSubrange(buffer.startIndex..<start)
        }

        mutating func finalHex() -> String {
            var tail = buffer
            let bitLength = bytes &* 8
            tail.append(0x80)
            while tail.count % 64 != 56 { tail.append(0) }
            for shift in stride(from: 56, through: 0, by: -8) { tail.append(UInt8((bitLength >> UInt64(shift)) & 0xff)) }
            var start = tail.startIndex
            while tail.endIndex - start >= 64 {
                compress(tail[start..<start + 64])
                start += 64
            }
            return h.map { String(format: "%08x", $0) }.joined()
        }

        private mutating func compress(_ block: Data) {
            let base = block.startIndex
            var w = [UInt32](repeating: 0, count: 64)
            for i in 0..<16 {
                let j = base + i * 4
                w[i] = UInt32(block[j]) << 24 | UInt32(block[j + 1]) << 16 | UInt32(block[j + 2]) << 8 | UInt32(block[j + 3])
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
                let t1 = hh &+ s1 &+ ch &+ Self.k[i] &+ w[i]
                let s0 = rotr(a, 2) ^ rotr(a, 13) ^ rotr(a, 22)
                let maj = (a & b) ^ (a & c) ^ (b & c)
                let t2 = s0 &+ maj
                hh = g; g = f; f = e; e = d &+ t1; d = c; c = b; b = a; a = t1 &+ t2
            }
            h[0] = h[0] &+ a; h[1] = h[1] &+ b; h[2] = h[2] &+ c; h[3] = h[3] &+ d
            h[4] = h[4] &+ e; h[5] = h[5] &+ f; h[6] = h[6] &+ g; h[7] = h[7] &+ hh
        }

        private func rotr(_ x: UInt32, _ n: UInt32) -> UInt32 { (x >> n) | (x << (32 - n)) }
    }

    private static func size(of url: URL) -> Int64? {
        (try? fm.attributesOfItem(atPath: url.path))?[.size] as? Int64
    }

    /// The node tree's move, verified (M2). The path is chosen up front by
    /// comparing volume identifiers, not discovered by a failing move:
    /// `FileManager.moveItem` does NOT fail across volumes — it copies and
    /// then deletes the source, checked by nothing. So:
    /// - same volume: `rename(2)`, atomic, inventoried before and after;
    /// - different (or unknown) volumes, or `forceCopy`: the SHA-256-verified
    ///   copy, leaving the old tree in place. Its shutdown (quarantine,
    ///   marker) is the commit point in `migrate`, not this function's
    ///   business (pre-audit 7, H2: nothing is quarantined before every
    ///   fallible copy has landed).
    /// A rename that fails anyway (EXDEV from a mount the identifiers did not
    /// show, a destination that appeared meanwhile) moved nothing — rename is
    /// all or nothing — and takes the verified copy too.
    private enum TreeMove { case moved; case copied; case failed }

    private static func moveTreeVerified(_ old: URL, _ new: URL, forceCopy: Bool, meter: ProgressMeter?) -> TreeMove {
        guard let before = manifest(of: old) else { return .failed }
        do {
            try fm.createDirectory(at: new.deletingLastPathComponent(), withIntermediateDirectories: true)
        } catch { return .failed }
        let oneVolume = !forceCopy && sameVolume(old, new.deletingLastPathComponent()) == true
        guard oneVolume, rename(old.path, new.path) == 0 else {
            meter?.expect(copyWork(before))
            return syncTreeVerified(old, new, meter: meter) ? .copied : .failed
        }
        guard let after = manifest(of: new) else { return .failed }
        return before == after ? .moved : .failed
    }

    /// Whether two existing paths live on one volume, by the volume
    /// identifiers Foundation reports. `nil` when either cannot be read —
    /// callers treat that as "different" and take the verified copy.
    static func sameVolume(_ a: URL, _ b: URL) -> Bool? {
        func id(_ u: URL) -> NSObject? {
            (try? u.resourceValues(forKeys: [.volumeIdentifierKey]))?.volumeIdentifier as? NSObject
        }
        guard let x = id(a), let y = id(b) else { return nil }
        return x.isEqual(y)
    }

    /// The bytes a verified copy of `entries` works through: each file is
    /// copied once and hashed on both sides.
    private static func copyWork(_ entries: [String: Int64]) -> Int64 {
        entries.filter { $0.key != lockName && !$0.key.hasPrefix(quarantinePrefix) }.values.reduce(0, +) * 3
    }

    /// Verified copy of a tree, file by file (the cross-volume path and the
    /// resume path of an interrupted earlier run). Already-correct files
    /// are kept; a file that does not match is replaced and re-verified by
    /// SHA-256; `run.lock` is never copied. True only when every file the old
    /// tree vouches for is at the new home with the content it had.
    static func syncTreeVerified(_ old: URL, _ new: URL, meter: ProgressMeter? = nil) -> Bool {
        guard let entries = manifest(of: old) else { return false }
        if let meter, meter.total == 0 { meter.expect(copyWork(entries)) }
        // The old tree's live lock is never copied, and its quarantine
        // directories are recovery copies for a human — not cargo for the
        // new home (they can appear mid-resume, after the copy already ran).
        func syncable(_ rel: String) -> Bool { rel != lockName && !rel.hasPrefix(quarantinePrefix) }
        for (rel, bytes) in entries where syncable(rel) {
            let o = old.appending(path: rel), n = new.appending(path: rel)
            if let have = size(of: n), have == bytes, fileMatches(o, n, meter: meter) {
                meter?.add(bytes)   // no copy needed
                continue
            }
            do {
                try fm.createDirectory(at: n.deletingLastPathComponent(), withIntermediateDirectories: true)
                if fm.fileExists(atPath: n.path) { try fm.removeItem(at: n) }   // a partial earlier attempt
                try fm.copyItem(at: o, to: n)
            } catch { return false }
            meter?.add(bytes)
            if !fileMatches(o, n, meter: meter) { return false }
        }
        guard let copied = manifest(of: new) else { return false }
        // Contents were hash-checked file by file above; here every old file
        // must be present with its size. The new side may hold files the old
        // side no longer lists — a quarantine resumed after the copy removes
        // entries from the old manifest that the new tree rightly keeps.
        return entries.filter { syncable($0.key) }
            .allSatisfy { copied[$0.key] == $0.value }
    }

    /// Copy one small precious file unless a verified copy is already there.
    /// The copy lands under a temporary name and is renamed into place only
    /// once it verifies: a reader (the key loader, running while a slow
    /// migration works in the background) sees no file or the whole file,
    /// never half of one. A different file already in place is set aside
    /// beside it, never deleted.
    enum CopyResult: Equatable { case ok; case unreadable(Int32); case failed }

    /// A verified copy; a source this process may not read right now
    /// (EPERM under complete file protection while locked, EACCES) is told
    /// apart from a real failure, so the caller can wait for the unlock.
    private static func copyVerified(_ old: URL, _ new: URL) -> CopyResult {
        let fd = open(old.path, O_RDONLY)
        if fd < 0 {
            let e = errno
            return (e == EPERM || e == EACCES) ? .unreadable(e) : .failed
        }
        close(fd)
        return copyVerifiedReadable(old, new) ? .ok : .failed
    }

    private static func copyVerifiedReadable(_ old: URL, _ new: URL) -> Bool {
        if fileMatches(old, new) { return true }
        let dir = new.deletingLastPathComponent()
        let temp = dir.appending(path: ".\(new.lastPathComponent).migrating-\(UUID().uuidString)")
        do {
            try fm.createDirectory(at: dir, withIntermediateDirectories: true)
            try fm.copyItem(at: old, to: temp)
        } catch {
            try? fm.removeItem(at: temp)
            return false
        }
        guard fileMatches(old, temp) else {
            try? fm.removeItem(at: temp)
            return false
        }
        if fm.fileExists(atPath: new.path) {
            let aside = dir.appending(path: "\(new.lastPathComponent).\(replacedPrefix)\(Int(Date().timeIntervalSince1970 * 1000))")
            guard (try? fm.moveItem(at: new, to: aside)) != nil else {
                try? fm.removeItem(at: temp)
                return false
            }
        }
        guard rename(temp.path, new.path) == 0 else {
            try? fm.removeItem(at: temp)
            return false
        }
        return fileMatches(old, new)
    }

    /// B4, the resume path after a stale done flag: the new tree may already
    /// hold a DIFFERENT validator identity, minted there while the flag hid
    /// the old one. The old identity is the one the chain knows (registered,
    /// staked), so it wins; the new tree's identity material — and the new
    /// identity guard that pins it — is set aside inside the new tree, where
    /// the node never looks for keys, and kept for a human to recover. A new
    /// tree whose key matches (our own interrupted copy) is left alone.
    private static func setAsideConflictingIdentity(old: URL, new: URL, support: URL) {
        let oldKey = old.appending(path: "validator.key"), newKey = new.appending(path: "validator.key")
        guard fm.fileExists(atPath: oldKey.path), fm.fileExists(atPath: newKey.path),
              !fileMatches(oldKey, newKey) else { return }
        let stamp = Int(Date().timeIntervalSince1970 * 1000)
        let aside = new.appending(path: "\(replacedPrefix)\(stamp)")
        guard (try? fm.createDirectory(at: aside, withIntermediateDirectories: true)) != nil else { return }
        let names = ((try? fm.contentsOfDirectory(atPath: new.path)) ?? []).filter(isRootSigningMaterial)
        for name in names { try? fm.moveItem(at: new.appending(path: name), to: aside.appending(path: name)) }
        let newGuard = support.appending(path: "EastSea/node\(identityGuardSuffix)")
        if fm.fileExists(atPath: newGuard.path) {
            try? fm.moveItem(at: newGuard, to: support.appending(path: "EastSea/node\(identityGuardSuffix).\(replacedPrefix)\(stamp)"))
        }
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

    /// Audit 6, A6-5: a copied (not moved) old tree must stop being a usable
    /// signer. The old binary cannot be taught the rename, and its
    /// one-instance lock is per data directory (supervisor.rs `run.lock`),
    /// so the only barrier the old app respects is the material itself: the
    /// validator key, the threshold share and the journals move into a
    /// quarantine directory inside the old tree — recoverable by hand, and
    /// invisible to the node's key lookup, which never reads inside a
    /// subdirectory (candidate.rs / supervisor.rs). `network.json` and the
    /// old `run.lock` stay, so the old binary still refuses to mint a fresh
    /// identity there (its `registered_identity` guard).
    ///
    /// Each item is one rename: interrupted anywhere, the tree has the item
    /// either still in place (the next run moves it) or already quarantined
    /// — never half-moved. The old binary signs with validator.key AND
    /// threshold.json together, so they move last and anything already moved
    /// is rolled back on a failure (pre-audit 7, H2): an unfinished
    /// quarantine always leaves the old tree a usable signer, never a
    /// stranded non-signer. This runs only after the new tree and every
    /// fallible copy are verified, and `mayStartNode` keeps the new node off
    /// until the quarantine finishes, so at no point can both trees serve
    /// the identity.
    ///
    /// An OS-wide lock keyed by the validator public key (audit 5, A5-5's
    /// option) was considered and rejected: the old binary predates the
    /// rename and would never take that lock, so it excludes nothing that
    /// matters — a lock only the new app honors is a false sense of safety,
    /// while removing the material excludes every binary at once.
    private static func quarantineOldSigningMaterial(_ old: URL) -> Bool {
        guard let names = try? fm.contentsOfDirectory(atPath: old.path) else { return false }
        var targets = names.filter(isRootSigningMaterial)
            + nestedSigningMaterial.filter { fm.fileExists(atPath: old.appending(path: $0).path) }
        guard !targets.isEmpty else { return true }   // an interrupted quarantine, already finished
        // The pair the old binary needs to sign moves last: any failure before
        // that leaves both in place (and a live failure rolls the rest back).
        targets.sort { a, _ in a == "validator.key" }
        targets.sort { a, _ in a == "threshold.json" }
        let q = old.appending(path: "\(quarantinePrefix)\(Int(Date().timeIntervalSince1970 * 1000))")
        do {
            try fm.createDirectory(at: q, withIntermediateDirectories: true)
            var moved: [String] = []
            for name in targets {
                do {
                    try fm.createDirectory(at: q.appending(path: name).deletingLastPathComponent(),
                                           withIntermediateDirectories: true)
                    try fm.moveItem(at: old.appending(path: name), to: q.appending(path: name))
                    moved.append(name)
                } catch {
                    // Put back what already moved (best effort, reverse
                    // order): whenever quarantine did not finish, the old
                    // tree stays a usable signer.
                    for back in moved.reversed() {
                        try? fm.moveItem(at: q.appending(path: back), to: old.appending(path: back))
                    }
                    return false
                }
            }
        } catch { return false }
        return true
    }

    /// The durable node-specific completion state (pre-audit 7, H2): the new
    /// tree is verified AND the old tree has stopped being a signer. The
    /// flag (`nodeDoneKey`) is written right after the quarantine; the
    /// tree-derived fallback covers a crash between the last rename and that
    /// write on the next launch, once the idempotent commit re-runs — an old
    /// root that is clean of signing material and marked migrated can only
    /// exist because a verified copy finished and its quarantine ran.
    static func nodeMigrationComplete(support: URL, defaults: UserDefaults) -> Bool {
        // B4: an old root that still holds an identity is unmigrated whatever
        // the flags say.
        if oldNodeUnmigrated(support) { return false }
        if defaults.bool(forKey: doneKey) || defaults.bool(forKey: nodeDoneKey) { return true }
        let oldNode = support.appending(path: "Aether/node")
        guard fm.fileExists(atPath: oldNode.path) else { return true }   // moved away entirely
        guard fm.fileExists(atPath: oldNode.appending(path: markerName).path) else { return false }
        return !holdsIdentity(oldNode)
            && !nestedSigningMaterial.contains { fm.fileExists(atPath: oldNode.appending(path: $0).path) }
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
    private static func destinationProblem(_ support: URL, locale: Locale, bundle: Bundle) -> String? {
        for part in ["Aether", "Aether/node", "AetherWallet", "EastSea", "EastSea/node", "EastSeaWallet"] {
            let p = support.appending(path: part)
            if (try? fm.destinationOfSymbolicLink(atPath: p.path)) != nil {
                NSLog("%@ is a symbolic link; refusing to migrate through it", part)
                return String(localized: "A data folder points to another location. Remove the link and reopen the app.", bundle: bundle, locale: locale)
            }
        }
        let oldR = support.appending(path: "Aether/node").standardizedFileURL.path
        let newR = support.appending(path: "EastSea/node").standardizedFileURL.path
        if oldR == newR {
            NSLog("the old and new data paths resolve to the same place (%@)", oldR)
            return String(localized: "The old and new data folders are in the same place. Choose a separate folder and reopen the app.", bundle: bundle, locale: locale)
        }
        if newR.hasPrefix(oldR + "/") || oldR.hasPrefix(newR + "/") {
            NSLog("one data path sits inside the other: %@; %@", oldR, newR)
            return String(localized: "One data folder is inside the other. Choose a separate folder and reopen the app.", bundle: bundle, locale: locale)
        }
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
    /// The same, from a dictionary (tests: never touch cfprefsd).
    private static func copyPreferencesDict(_ old: [String: Any], into defaults: UserDefaults) {
        for (key, value) in old where defaults.object(forKey: key) == nil { defaults.set(value, forKey: key) }
    }

    private static func copyOldPreferences(from oldAppID: String, into defaults: UserDefaults) {
        guard let keys = CFPreferencesCopyKeyList(oldAppID as CFString,
                                                  kCFPreferencesCurrentUser,
                                                  kCFPreferencesAnyHost) as? [String] else { return }
        for key in keys where defaults.object(forKey: key) == nil {
            guard let value = CFPreferencesCopyValue(key as CFString, oldAppID as CFString,
                                                     kCFPreferencesCurrentUser, kCFPreferencesAnyHost) else { continue }
            defaults.set(value, forKey: key)
        }
    }

    // MARK: running it (M1, L2)

    /// Counts the bytes the verified-copy path works through and reports a
    /// fraction, at most every 64 MiB (and at the end), from whatever thread
    /// does the work.
    final class ProgressMeter {
        private let lock = NSLock()
        private(set) var total: Int64 = 0
        private var done: Int64 = 0
        private var reported: Int64 = 0
        private let report: (Double) -> Void

        init(report: @escaping (Double) -> Void) { self.report = report }

        func expect(_ bytes: Int64) {
            lock.lock(); total = max(total, bytes); lock.unlock()
        }

        func add(_ bytes: Int64) {
            lock.lock()
            done += bytes
            let due = done - reported >= 64 << 20
            if due { reported = done }
            let fraction = total > 0 ? min(1, Double(done) / Double(total)) : 0
            lock.unlock()
            if due { report(fraction) }
        }
    }

    /// One migration at a time per support root, and the main thread never
    /// waits on the slow path (M1):
    /// - `runNow` (any thread but main) runs `migrate` under a lock, so two
    ///   concurrent callers no longer race for the old run.lock and the same
    ///   files (L2) — the second waits and then sees the first one's result;
    /// - `ensureFromMain` runs the fast cases inline (nothing to do, a
    ///   same-volume rename: milliseconds) and starts the verified copy — a
    ///   cross-volume move or the resume over an existing new tree, which
    ///   hashes every byte — on a background queue, answering `.running`
    ///   meanwhile. `onProgress` and `onFinish` tell the app (any thread).
    /// Once a run reached `.done`/`.noOldData`, the process never runs it
    /// again.
    final class Runner {
        static let shared = Runner(support: DataMigration.supportURL, defaults: .standard)

        let support: URL
        let defaults: UserDefaults
        let oldPreferencesDomain: String
        let forceCopy: Bool
        let locale: Locale
        let bundle: Bundle
        /// The verified copy's progress, 0…1.
        var onProgress: ((Double) -> Void)?
        /// A background run started (the app shows its progress).
        var onStart: (() -> Void)?
        /// Every background run's outcome.
        var onFinish: ((Outcome) -> Void)?

        private let runLock = NSLock()
        private let stateLock = NSLock()
        /// The last inline outcome reported (ensure() runs on every data-dir
        /// read: a deferral must be reported once, not on every call).
        private var lastReported: Outcome?
        private var running = false
        private var settled = false

        init(support: URL, defaults: UserDefaults, oldPreferencesDomain: String = DataMigration.oldAppID,
             forceCopy: Bool = false, locale: Locale = .current, bundle: Bundle = .main) {
            self.support = support
            self.defaults = defaults
            self.oldPreferencesDomain = oldPreferencesDomain
            self.forceCopy = forceCopy
            self.locale = locale
            self.bundle = bundle
        }

        /// A migration is working right now (the gates stay shut).
        var isRunning: Bool { stateLock.lock(); defer { stateLock.unlock() }; return running }
        private var isSettled: Bool { stateLock.lock(); defer { stateLock.unlock() }; return settled }

        /// Run (or wait for the run in progress), serialized.
        func runNow() -> Outcome {
            runLock.lock()
            defer { runLock.unlock() }
            return runHoldingLock()
        }

        private func runHoldingLock() -> Outcome {
            if isSettled { return .done }
            setRunning(true)
            defer { setRunning(false) }
            let meter = ProgressMeter { [weak self] in self?.onProgress?($0) }
            let outcome = DataMigration.migrate(support: support, defaults: defaults,
                                                oldPreferencesDomain: oldPreferencesDomain,
                                                forceCopy: forceCopy, meter: meter, locale: locale, bundle: bundle)
            if outcome == .done || outcome == .noOldData {
                stateLock.lock(); settled = true; stateLock.unlock()
            }
            return outcome
        }

        /// From the main thread: never blocks on a slow run.
        func ensureFromMain() -> Outcome {
            if isSettled { return .done }
            if !expectsLongRun() {
                // Fast path; but if a background run holds the lock, do not wait.
                guard runLock.try() else { return .running(DataMigration.movingSentence(locale: locale, bundle: bundle)) }
                let outcome = runHoldingLock()
                runLock.unlock()
                // Every inline outcome reaches the app (the founder's MacBook
                // and poc-m3, 2026-10-07): a finish starts the node, a
                // deferral or a lock is shown — once per change.
                stateLock.lock()
                let changed = lastReported != outcome
                lastReported = outcome
                stateLock.unlock()
                if changed { onFinish?(outcome) }
                return outcome
            }
            start()
            return .running(DataMigration.movingSentence(locale: locale, bundle: bundle))
        }

        /// Start the background run unless one is already going.
        func start() {
            #if WALLET_SCREENS
            return
            #endif
            stateLock.lock()
            let busy = running || settled
            let alreadySettled = settled
            if !busy { running = true }   // claimed now: no second start, gates shut at once
            stateLock.unlock()
            if alreadySettled {
                // A caller waiting on this retry (the old app's "quit and
                // move to Trash") learns the move is already done.
                DispatchQueue.global(qos: .userInitiated).async { [self] in onFinish?(.done) }
                return
            }
            guard !busy else { return }
            onStart?()
            DispatchQueue.global(qos: .userInitiated).async { [self] in
                runLock.lock()
                let outcome = runHoldingLock()   // keeps the claim; clears it when done
                runLock.unlock()
                onFinish?(outcome)
            }
        }

        private func setRunning(_ on: Bool) { stateLock.lock(); running = on; stateLock.unlock() }

        /// The hashing path is ahead: an old node tree that must be copied
        /// (another volume) or resumed over an existing new tree.
        func expectsLongRun() -> Bool {
            if defaults.bool(forKey: DataMigration.doneKey) && DataMigration.unmigratedOldData(support: support).isEmpty {
                return false
            }
            let old = support.appending(path: "Aether/node")
            guard DataMigration.fm.fileExists(atPath: old.path) else { return false }
            let eastSea = support.appending(path: "EastSea")
            if DataMigration.fm.fileExists(atPath: eastSea.appending(path: "node").path) { return true }
            let target = DataMigration.fm.fileExists(atPath: eastSea.path) ? eastSea : support
            return forceCopy || DataMigration.sameVolume(old, target) != true
        }
    }
}
