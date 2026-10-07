import Foundation

/// The copy's durable ownership record. A path preference is committed only
/// after publication; a killed copy therefore leaves the source authoritative.
/// Verified publication can resume, and cleanup never enumerates fresh data.
enum BlockDataMove {
    static let recordName = "block-data-move.json"
    struct Identity: Codable, Equatable {
        let device: UInt64
        let inode: UInt64
        let volumeUUID: String?
        static func == (a: Identity, b: Identity) -> Bool {
            guard a.inode == b.inode else { return false }
            if let x = a.volumeUUID, let y = b.volumeUUID { return x == y }
            return a.volumeUUID == b.volumeUUID && a.device == b.device
        }
    }
    struct File: Codable {
        let identity: Identity
        let hash: String
    }
    struct Record: Codable {
        let source: String
        let target: String
        let staging: String
        let stagingID: Identity
        let sourceID: Identity
        let targetID: Identity
        let directories: [String]
        let files: [String: File]
        var committed = false
        var cleanupDone = false
    }
    enum Failure: Error { case unavailable, occupied, invalidRecord, copy, persistence }

    static func identity(_ url: URL, directory: Bool = true) -> Identity? {
        var st = stat()
        guard lstat(url.path, &st) == 0,
              (st.st_mode & S_IFMT) == (directory ? S_IFDIR : S_IFREG) else { return nil }
        if directory, let volume = BlockDataLocation.volumeName(ofPath: url.path) {
            var fs = statfs()
            guard statfs(url.path, &fs) == 0 else { return nil }
            let mount = withUnsafeBytes(of: fs.f_mntonname) { String(decoding: $0.prefix { $0 != 0 }, as: UTF8.self) }
            guard mount == "/Volumes/\(volume)" else { return nil } // no internal /Volumes fallback
        }
        let uuid = (try? url.resourceValues(forKeys: [.volumeUUIDStringKey]))?.volumeUUIDString
        return Identity(device: UInt64(UInt32(bitPattern: st.st_dev)), inode: UInt64(st.st_ino), volumeUUID: uuid)
    }

    private static func scan(_ dir: URL, prefix: String, files: inout [String: File]) throws {
        guard identity(dir) != nil else { throw Failure.unavailable }
        for name in try FileManager.default.contentsOfDirectory(atPath: dir.path) {
            if BlockDataLocation.keepInternal.contains(name) || name == "run.lock" { continue }
            let item = dir.appendingPathComponent(name), rel = "\(prefix)/\(name)"
            if identity(item) != nil {
                try scan(item, prefix: rel, files: &files)
            } else {
                guard let id = identity(item, directory: false), let hash = DataMigration.streamSHA256(item) else { throw Failure.copy }
                files[rel] = File(identity: id, hash: hash)
            }
        }
    }

    private static func load(_ internalRoot: URL) throws -> Record? {
        let url = internalRoot.appendingPathComponent(recordName)
        guard FileManager.default.fileExists(atPath: url.path) else { return nil }
        guard identity(url, directory: false) != nil else { throw Failure.invalidRecord }
        let r = try JSONDecoder().decode(Record.self, from: Data(contentsOf: url))
        guard BlockDataLocation.disjoint(URL(fileURLWithPath: r.source), URL(fileURLWithPath: r.target)),
              r.staging.hasPrefix(".eastsea-storage-move-"), !r.staging.contains("/"),
              r.directories.allSatisfy({ BlockDataLocation.movedDirs.contains($0) }),
              r.files.keys.allSatisfy({ rel in
                  let parts = rel.split(separator: "/", omittingEmptySubsequences: false)
                  return parts.count > 1 && r.directories.contains(String(parts[0]))
                      && parts.allSatisfy { !$0.isEmpty && $0 != "." && $0 != ".." && !BlockDataLocation.keepInternal.contains(String($0)) }
              }) else { throw Failure.invalidRecord }
        return r
    }

    @discardableResult private static func syncDirectory(_ dir: URL) -> Bool {
        let fd = open(dir.path, O_RDONLY | O_NOFOLLOW)
        guard fd >= 0 else { return false }
        defer { close(fd) }
        return fsync(fd) == 0
    }

    private static func save(_ record: Record, in dir: URL) throws {
        let url = dir.appendingPathComponent(recordName)
        let temp = dir.appendingPathComponent(".block-data-move-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: temp) }
        try JSONEncoder().encode(record).write(to: temp, options: .withoutOverwriting)
        let handle = try FileHandle(forWritingTo: temp)
        try handle.synchronize(); try handle.close()
        guard rename(temp.path, url.path) == 0, syncDirectory(dir) else { throw Failure.persistence }
    }

    /// Returns after every intended file is published, durably. An existing
    /// record is replayed only on the same disks and unchanged source bytes.
    static func copy(source: URL, target: URL, sourceID: Identity, internalRoot: URL,
                     preservingInternalKeys: Bool, meter: DataMigration.ProgressMeter) throws -> UInt64 {
        let fm = FileManager.default
        guard identity(source) == sourceID, identity(internalRoot) != nil,
              BlockDataLocation.disjoint(source, target) else { throw Failure.unavailable }
        var record = try load(internalRoot)
        if record?.cleanupDone == true { record = nil }
        if var previous = record, !previous.committed, previous.source == source.path,
           previous.sourceID == sourceID {
            let unchanged = previous.files.allSatisfy { rel, proof in
                let item = source.appendingPathComponent(rel)
                return identity(item, directory: false) == proof.identity && DataMigration.streamSHA256(item) == proof.hash
            }
            if previous.target != target.path || !unchanged {
                // Source writers may advance after a killed, uncommitted
                // move. Abort durably, retaining all copied cargo, so a fresh
                // move to an empty folder remains possible.
                previous.cleanupDone = true
                try save(previous, in: internalRoot)
                record = nil
            }
        }
        if let r = record {
            guard r.source == source.path, r.target == target.path, r.sourceID == sourceID,
                  identity(target) == r.targetID else { throw Failure.invalidRecord }
        } else {
            guard BlockDataLocation.destinationAvailable(target, preservingInternalKeys: preservingInternalKeys) else { throw Failure.occupied }
            let names = try fm.contentsOfDirectory(atPath: source.path)
            let dirs = BlockDataLocation.movedDirs.filter { names.contains($0) }
            var files: [String: File] = [:]
            for d in dirs { try scan(source.appendingPathComponent(d), prefix: d, files: &files) }
            guard identity(source) == sourceID else { throw Failure.unavailable }
            if !fm.fileExists(atPath: target.path) { try fm.createDirectory(at: target, withIntermediateDirectories: false) }
            guard let targetID = identity(target) else { throw Failure.unavailable }
            let stageName = ".eastsea-storage-move-\(UUID().uuidString)"
            let stage = target.appendingPathComponent(stageName)
            try fm.createDirectory(at: stage, withIntermediateDirectories: false)
            guard let stagingID = identity(stage) else { throw Failure.unavailable }
            // Until a verified record exists, only this private stage may go.
            do {
                for d in dirs {
                    let staged = stage.appendingPathComponent(d)
                    try fm.createDirectory(at: staged, withIntermediateDirectories: false)
                    guard DataMigration.syncTreeVerified(source.appendingPathComponent(d), staged, meter: meter) else { throw Failure.copy }
                    var directories = [staged]
                    guard let entries = fm.enumerator(at: staged, includingPropertiesForKeys: nil) else { throw Failure.persistence }
                    for case let file as URL in entries {
                        if identity(file) != nil { directories.append(file); continue }
                        guard identity(file, directory: false) != nil else { continue }
                        let h = try FileHandle(forWritingTo: file)
                        try h.synchronize(); try h.close()
                    }
                    for dir in directories.reversed() { guard syncDirectory(dir) else { throw Failure.persistence } }
                }
                guard syncDirectory(stage), syncDirectory(target), syncDirectory(target.deletingLastPathComponent()) else { throw Failure.persistence }
                guard identity(source) == sourceID, identity(target) == targetID else { throw Failure.unavailable }
                let r = Record(source: source.path, target: target.path, staging: stageName,
                               stagingID: stagingID, sourceID: sourceID, targetID: targetID, directories: dirs, files: files)
                try save(r, in: internalRoot)
                record = r
            } catch {
                // If persistence succeeded before a later fsync failure,
                // retain its cargo for replay. Never recursively clear target.
                if (try? load(internalRoot)) == nil, identity(target) == targetID, identity(stage) == stagingID { try? fm.removeItem(at: stage) }
                throw error
            }
        }
        guard var r = record, identity(source) == r.sourceID, identity(target) == r.targetID else { throw Failure.unavailable }
        let stage = target.appendingPathComponent(r.staging)
        if fm.fileExists(atPath: stage.path), identity(stage) != r.stagingID { throw Failure.invalidRecord }
        // Verify all replay inputs before any publication. Additional source
        // data is retained; changed snapshot files cannot authorize a switch.
        for (rel, proof) in r.files {
            let original = source.appendingPathComponent(rel)
            guard identity(original, directory: false) == proof.identity, DataMigration.streamSHA256(original) == proof.hash else { throw Failure.copy }
            let staged = stage.appendingPathComponent(rel), final = target.appendingPathComponent(rel)
            let cargo = fm.fileExists(atPath: staged.path) ? staged : final
            guard identity(cargo, directory: false) != nil, DataMigration.streamSHA256(cargo) == proof.hash else { throw Failure.copy }
        }
        // A resumed destination must contain only our verified cargo and,
        // at the default home, the preexisting internal endpoint key.
        for d in r.directories where fm.fileExists(atPath: target.appendingPathComponent(d).path) {
            var existing: [String: File] = [:]
            try scan(target.appendingPathComponent(d), prefix: d, files: &existing)
            guard existing.allSatisfy({ r.files[$0.key]?.hash == $0.value.hash }) else { throw Failure.occupied }
        }
        for d in r.directories {
            let staged = stage.appendingPathComponent(d), final = target.appendingPathComponent(d)
            if !fm.fileExists(atPath: final.path) {
                try fm.moveItem(at: staged, to: final)
            } else {
                guard identity(final) != nil else { throw Failure.occupied }
                if fm.fileExists(atPath: staged.path) {
                    for name in try fm.contentsOfDirectory(atPath: staged.path) {
                        let entry = final.appendingPathComponent(name)
                        guard !fm.fileExists(atPath: entry.path) else { throw Failure.occupied }
                        try fm.moveItem(at: staged.appendingPathComponent(name), to: entry)
                        guard syncDirectory(staged) else { throw Failure.persistence }
                    }
                }
            }
            guard syncDirectory(final), syncDirectory(target) else { throw Failure.persistence }
        }
        guard identity(source) == r.sourceID, identity(target) == r.targetID else { throw Failure.unavailable }
        // R07 will exclude keys during copying itself; this preserves the
        // current endpoint-key behavior until its separate regression/fix.
        for d in r.directories {
            for key in BlockDataLocation.keepInternal { try? fm.removeItem(at: target.appendingPathComponent(d).appendingPathComponent(key)) }
        }
        // Never recursively delete replay paths: unexpected leftovers survive.
        if identity(stage) == r.stagingID {
            if let dirs = fm.enumerator(at: stage, includingPropertiesForKeys: nil) {
                let emptyDirs = dirs.compactMap { $0 as? URL }.filter { identity($0) != nil }
                for dir in emptyDirs.reversed() { _ = rmdir(dir.path) }
            }
            _ = rmdir(stage.path)
            guard syncDirectory(target) else { throw Failure.persistence }
        }
        r.committed = true
        try save(r, in: internalRoot)
        return r.files.keys.reduce(0) { total, rel in
            total + UInt64(((try? fm.attributesOfItem(atPath: target.appendingPathComponent(rel).path))?[.size] as? NSNumber)?.uint64Value ?? 0)
        }
    }

    static func canResume(source: URL, target: URL, internalRoot: URL) -> Bool {
        guard let r = try? load(internalRoot) else { return false }
        return !r.cleanupDone && r.source == source.path && r.target == target.path
    }

    /// Durable choices outlive asynchronous preferences and installer kills.
    static func authoritativeRoot(in internalRoot: URL) throws -> URL? {
        guard let r = try load(internalRoot) else { return nil }
        return URL(fileURLWithPath: r.committed ? r.target : r.source, isDirectory: true)
    }

    /// Recheck the selected disk at startup and on every mount/start gate.
    /// A different directory at the same path is never a fresh-store fallback.
    static func selectionAvailable(_ selected: URL, internalRoot: URL) -> Bool {
        do {
            guard let r = try load(internalRoot) else { return true }
            let expectedPath = r.committed ? r.target : r.source
            let expectedID = r.committed ? r.targetID : r.sourceID
            return selected.path == expectedPath && identity(selected) == expectedID
        } catch { return false }
    }

    /// An unavailable disk retains the record. A replaced or changed source
    /// file stays put. Only exact verified files are unlinked, never subtrees.
    static func cleanup(confirmedTarget: URL, internalRoot: URL) {
        guard var r = try? load(internalRoot), r.committed, !r.cleanupDone, r.target == confirmedTarget.path,
              identity(confirmedTarget) == r.targetID else { return }
        let source = URL(fileURLWithPath: r.source)
        guard identity(source) == r.sourceID, BlockDataLocation.disjoint(source, confirmedTarget) else { return }
        for (rel, proof) in r.files {
            let item = source.appendingPathComponent(rel)
            guard BlockDataLocation.resolvedRoot(item).path == item.path,
                  identity(item, directory: false) == proof.identity,
                  DataMigration.streamSHA256(item) == proof.hash,
                  DataMigration.streamSHA256(confirmedTarget.appendingPathComponent(rel)) == proof.hash else { continue }
            _ = unlink(item.path)
            _ = syncDirectory(item.deletingLastPathComponent())
        }
        // rmdir refuses anything nonempty: new entries and keys survive.
        for d in r.directories { _ = rmdir(source.appendingPathComponent(d).path) }
        _ = syncDirectory(source)
        if source.lastPathComponent == BlockDataLocation.folderName { _ = rmdir(source.path) }
        r.cleanupDone = true
        try? save(r, in: internalRoot)
    }
}
