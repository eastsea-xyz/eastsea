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


    /// Own the internal node lock until the caller closes the returned fd.
    /// An installer or replacement node must never inherit this descriptor.
    static func holdRunLock(in dir: URL, timeout: TimeInterval = 60) async -> Int32? {
        guard timeout.isFinite, timeout >= 0, let rootID = identity(dir) else { return nil }
        let lockURL = dir.appendingPathComponent("run.lock")
        // A running installation already has this inode. flock needs no
        // writable descriptor or new allocation on a full source volume.
        var fd = open(lockURL.path, O_RDONLY | O_NOFOLLOW | O_CLOEXEC)
        if fd < 0, errno == ENOENT { fd = open(lockURL.path, O_RDWR | O_CREAT | O_NOFOLLOW | O_CLOEXEC, 0o600) }
        guard fd >= 0 else { return nil }
        let flags = fcntl(fd, F_GETFD)
        var descriptor = stat()
        guard flags >= 0, fcntl(fd, F_SETFD, flags | FD_CLOEXEC) == 0 else { close(fd); return nil }
        let confirmedFlags = fcntl(fd, F_GETFD)
        guard confirmedFlags >= 0, (confirmedFlags & FD_CLOEXEC) != 0,
              fstat(fd, &descriptor) == 0, (descriptor.st_mode & S_IFMT) == S_IFREG,
              let lockID = identity(lockURL, directory: false),
              lockID.device == UInt64(UInt32(bitPattern: descriptor.st_dev)),
              lockID.inode == UInt64(descriptor.st_ino) else { close(fd); return nil }
        let deadline = ProcessInfo.processInfo.systemUptime + timeout
        while !Task.isCancelled {
            guard identity(dir) == rootID, identity(lockURL, directory: false) == lockID else { break }
            if flock(fd, LOCK_EX | LOCK_NB) == 0 {
                guard identity(dir) == rootID, identity(lockURL, directory: false) == lockID else { break }
                return fd
            }
            guard errno == EWOULDBLOCK || errno == EAGAIN || errno == EINTR else { break }
            let remaining = deadline - ProcessInfo.processInfo.systemUptime
            guard remaining > 0 else { break }
            do { try await Task.sleep(nanoseconds: UInt64(min(0.05, remaining) * 1_000_000_000)) }
            catch { break }
        }
        close(fd)
        return nil
    }

    static func identity(_ url: URL, directory: Bool = true) -> Identity? {
        var st = stat()
        guard lstat(url.path, &st) == 0,
              (st.st_mode & S_IFMT) == (directory ? S_IFDIR : S_IFREG) else { return nil }
        if directory, BlockDataLocation.volumeName(ofPath: url.path) != nil {
            var fs = statfs()
            guard statfs(url.path, &fs) == 0 else { return nil }
            let mount = withUnsafeBytes(of: fs.f_mntonname) { String(decoding: $0.prefix { $0 != 0 }, as: UTF8.self) }
            // Accept genuine nested mounts too, never a missing disk path
            // that falls back onto the system volume.
            guard mount.hasPrefix("/Volumes/"), url.path == mount || url.path.hasPrefix(mount + "/") else { return nil }
        }
        let uuid = (try? url.resourceValues(forKeys: [.volumeUUIDStringKey]))?.volumeUUIDString
        return Identity(device: UInt64(UInt32(bitPattern: st.st_dev)), inode: UInt64(st.st_ino), volumeUUID: uuid)
    }

    private static func scan(_ dir: URL, prefix: String, files: inout [String: File], hashing: Bool = true) throws {
        guard identity(dir) != nil else { throw Failure.unavailable }
        for name in try FileManager.default.contentsOfDirectory(atPath: dir.path) {
            if BlockDataLocation.keepsInternal(name) || name.lowercased() == "run.lock" { continue }
            let item = dir.appendingPathComponent(name), rel = "\(prefix)/\(name)"
            if identity(item) != nil {
                try scan(item, prefix: rel, files: &files, hashing: hashing)
            } else {
                guard let id = identity(item, directory: false) else { throw Failure.copy }
                guard let hash = hashing ? DataMigration.streamSHA256(item) : "" else { throw Failure.copy }
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
                      && parts.allSatisfy { !$0.isEmpty && $0 != "." && $0 != ".." && !BlockDataLocation.keepsInternal(String($0))
                          && $0.lowercased() != "run.lock" }
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
        while true {
            do { try persist(record, in: dir); return }
            catch {
                guard isNoSpace(error), try preserveLogsAndFreeSpace(in: dir, target: URL(fileURLWithPath: record.target)) else { throw error }
            }
        }
    }

    private static func isNoSpace(_ error: Error) -> Bool {
        let e = error as NSError
        if e.domain == NSPOSIXErrorDomain && e.code == Int(ENOSPC) { return true }
        if e.domain == NSCocoaErrorDomain && e.code == CocoaError.fileWriteOutOfSpace.rawValue { return true }
        return (e.userInfo[NSUnderlyingErrorKey] as? Error).map(isNoSpace) ?? false
    }

    /// The ownership journal must remain durable even when the internal
    /// disk filled. Preserve diagnostics on the destination before releasing
    /// their old allocation; block data and keys are never used as spare space.
    private static func preserveLogsAndFreeSpace(in dir: URL, target: URL) throws -> Bool {
        for name in ["node-status.log", "node.log"] {
            let original = dir.appendingPathComponent(name)
            guard let originalID = identity(original, directory: false),
                  let bytes = (try? FileManager.default.attributesOfItem(atPath: original.path))?[.size] as? NSNumber,
                  bytes.int64Value > 0 else { continue }
            let backup = target.appendingPathComponent(".\(name).before-storage-move-\(UUID().uuidString)")
            guard DataMigration.streamCopyVerified(original, backup) != nil, syncDirectory(target),
                  identity(original, directory: false) == originalID else { throw Failure.persistence }
            let fd = open(original.path, O_WRONLY | O_NOFOLLOW | O_CLOEXEC)
            guard fd >= 0 else { throw Failure.persistence }
            var descriptor = stat()
            let matches = fstat(fd, &descriptor) == 0
                && originalID.device == UInt64(UInt32(bitPattern: descriptor.st_dev))
                && originalID.inode == UInt64(descriptor.st_ino)
            let released = matches && ftruncate(fd, 0) == 0 && DataMigration.syncFile(fd)
            close(fd)
            guard released else { throw Failure.persistence }
            return true
        }
        return false
    }

    private static func persist(_ record: Record, in dir: URL) throws {
        let url = dir.appendingPathComponent(recordName)
        let temp = dir.appendingPathComponent(".block-data-move-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: temp) }
        try JSONEncoder().encode(record).write(to: temp, options: .withoutOverwriting)
        let handle = try FileHandle(forWritingTo: temp)
        defer { try? handle.close() }
        guard DataMigration.syncFile(handle.fileDescriptor) else { throw NSError(domain: NSPOSIXErrorDomain, code: Int(errno)) }
        try handle.close()
        guard rename(temp.path, url.path) == 0 else { throw NSError(domain: NSPOSIXErrorDomain, code: Int(errno)) }
        guard syncDirectory(dir) else { throw NSError(domain: NSPOSIXErrorDomain, code: Int(errno)) }
    }

    /// Returns after every intended file is published, durably. An existing
    /// record is replayed only on the same disks and unchanged source bytes.
    static func copy(source: URL, target: URL, sourceID: Identity, internalRoot: URL,
                     preservingInternalKeys: Bool, meter: DataMigration.ProgressMeter) throws -> UInt64 {
        let fm = FileManager.default
        guard identity(source) == sourceID, identity(internalRoot) != nil,
              BlockDataLocation.disjoint(source, target) else { throw Failure.unavailable }
        var record = try load(internalRoot)
        var freshlyVerified = false
        if record?.cleanupDone == true { record = nil }
        if var previous = record, !previous.committed, previous.source == source.path,
           previous.sourceID == sourceID {
            if previous.target != target.path {
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
            var dirs: [String] = []
            for name in BlockDataLocation.movedDirs {
                let dir = source.appendingPathComponent(name)
                var entry = stat()
                if lstat(dir.path, &entry) != 0 {
                    // A missing optional namespace is fine; an inaccessible
                    // one is not evidence that this is an empty source.
                    guard errno == ENOENT else { throw Failure.unavailable }
                    continue
                }
                // Native lookup recognizes Follow/follow on APFS. lstat
                // distinguishes a broken or live symlink from absence, and
                // identity requires a real accessible directory before copy.
                guard fm.fileExists(atPath: dir.path), identity(dir) != nil else { throw Failure.unavailable }
                dirs.append(name)
            }
            var files: [String: File] = [:]
            // Inventory identities only. The source digest is collected
            // during its one copy read, while the caller owns the writer lock.
            for d in dirs { try scan(source.appendingPathComponent(d), prefix: d, files: &files, hashing: false) }
            if meter.total == 0 {
                let bytes = files.keys.reduce(Int64(0)) { total, rel in
                    total + (((try? fm.attributesOfItem(atPath: source.appendingPathComponent(rel).path))?[.size] as? NSNumber)?.int64Value ?? 0)
                }
                meter.expect(bytes * 2)
            }
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
                    guard DataMigration.syncTreeVerified(source.appendingPathComponent(d), staged, meter: meter,
                                                        excluding: BlockDataLocation.keepInternal.union(["run.lock"]), verified: { rel, hash in
                        let path = "\(d)/\(rel)"
                        guard let proof = files[path], identity(source.appendingPathComponent(path), directory: false) == proof.identity else { return false }
                        files[path] = File(identity: proof.identity, hash: hash)
                        return true
                    }) else { throw Failure.copy }
                    var directories = [staged]
                    guard let entries = fm.enumerator(at: staged, includingPropertiesForKeys: nil) else { throw Failure.persistence }
                    for case let file as URL in entries {
                        if identity(file) != nil { directories.append(file); continue }
                        // Each regular temp was already flushed before its
                        // verified rename. Only directory entries remain.
                    }
                    for dir in directories.reversed() { guard syncDirectory(dir) else { throw Failure.persistence } }
                }
                guard syncDirectory(stage), syncDirectory(target), syncDirectory(target.deletingLastPathComponent()) else { throw Failure.persistence }
                guard identity(source) == sourceID, identity(target) == targetID else { throw Failure.unavailable }
                guard files.values.allSatisfy({ !$0.hash.isEmpty }) else { throw Failure.copy }
                let r = Record(source: source.path, target: target.path, staging: stageName,
                               stagingID: stagingID, sourceID: sourceID, targetID: targetID, directories: dirs, files: files)
                try save(r, in: internalRoot)
                record = r
                freshlyVerified = true
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
        if !freshlyVerified, meter.total == 0 {
            let bytes = r.files.keys.reduce(Int64(0)) { total, rel in
                total + (((try? fm.attributesOfItem(atPath: source.appendingPathComponent(rel).path))?[.size] as? NSNumber)?.int64Value ?? 0)
            }
            meter.expect(bytes * 2)
        }
        for (rel, proof) in r.files where !freshlyVerified {
            let original = source.appendingPathComponent(rel)
            guard identity(original, directory: false) == proof.identity, DataMigration.streamSHA256(original, meter: meter) == proof.hash else { throw Failure.copy }
            let staged = stage.appendingPathComponent(rel), final = target.appendingPathComponent(rel)
            let cargo = fm.fileExists(atPath: staged.path) ? staged : final
            guard identity(cargo, directory: false) != nil, DataMigration.streamSHA256(cargo, meter: meter) == proof.hash else { throw Failure.copy }
            if cargo == staged, fm.fileExists(atPath: final.path) { throw Failure.occupied }
        }
        // A resumed destination must contain only our verified cargo and,
        // at the default home, the preexisting internal endpoint key.
        for d in r.directories where fm.fileExists(atPath: target.appendingPathComponent(d).path) {
            var existing: [String: File] = [:]
            try scan(target.appendingPathComponent(d), prefix: d, files: &existing, hashing: false)
            guard existing.keys.allSatisfy({ r.files[$0] != nil }) else { throw Failure.occupied }
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
