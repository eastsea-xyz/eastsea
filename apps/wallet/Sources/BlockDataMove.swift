import Foundation

/// A durable fresh-start choice. No block database is read, copied or verified.
/// Cleanup is authorized only by a response and a later certified block from
/// the new node, and is confined to the old block-data namespaces.
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
    struct Progress: Equatable {
        var height: UInt64 = 0
        var target: UInt64 = 0
    }
    struct Record: Codable {
        var version = 2
        let source: String
        let target: String
        let sourceID: Identity
        let targetID: Identity
        let directories: [String: Identity]
        var committed = true
        var answeredHeight: UInt64?
        var ready = false
        var cleanupDone = false

        init(source: String, target: String, sourceID: Identity, targetID: Identity,
             directories: [String: Identity]) {
            self.source = source; self.target = target
            self.sourceID = sourceID; self.targetID = targetID
            self.directories = directories
        }

        private enum CodingKeys: String, CodingKey {
            case version, source, target, sourceID, targetID, directories, committed, answeredHeight, ready, cleanupDone
        }
        init(from decoder: Decoder) throws {
            let c = try decoder.container(keyedBy: CodingKeys.self)
            version = try c.decodeIfPresent(Int.self, forKey: .version) ?? 1
            source = try c.decode(String.self, forKey: .source)
            target = try c.decode(String.self, forKey: .target)
            sourceID = try c.decode(Identity.self, forKey: .sourceID)
            targetID = try c.decode(Identity.self, forKey: .targetID)
            // Legacy copy journals still select the authoritative location.
            // They contain no pinned namespace identities, so their copied
            // cargo is never replayed or deleted by the fresh-start path.
            directories = version == 2 ? try c.decode([String: Identity].self, forKey: .directories) : [:]
            committed = try c.decode(Bool.self, forKey: .committed)
            answeredHeight = try c.decodeIfPresent(UInt64.self, forKey: .answeredHeight)
            ready = try c.decodeIfPresent(Bool.self, forKey: .ready) ?? false
            cleanupDone = try c.decode(Bool.self, forKey: .cleanupDone)
        }
    }
    enum Failure: Error, Equatable { case unavailable, occupied, invalidRecord, persistence, pending }

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

    private static func load(_ internalRoot: URL) throws -> Record? {
        let url = internalRoot.appendingPathComponent(recordName)
        guard FileManager.default.fileExists(atPath: url.path) else { return nil }
        guard identity(url, directory: false) != nil else { throw Failure.invalidRecord }
        let r = try JSONDecoder().decode(Record.self, from: Data(contentsOf: url))
        guard (1...2).contains(r.version), r.source.hasPrefix("/"), r.target.hasPrefix("/"),
              BlockDataLocation.disjoint(URL(fileURLWithPath: r.source), URL(fileURLWithPath: r.target)),
              r.directories.keys.allSatisfy({ BlockDataLocation.movedDirs.contains($0) }) else { throw Failure.invalidRecord }
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

    /// Called under the internal writer lock, after the node stops. Only
    /// namespace identities are recorded; the destination starts empty.
    static func prepare(source: URL, target: URL, sourceID: Identity, internalRoot: URL,
                        preservingInternalKeys: Bool) throws {
        guard identity(source) == sourceID, identity(internalRoot) != nil,
              BlockDataLocation.disjoint(source, target) else { throw Failure.unavailable }
        if let previous = try load(internalRoot) {
            guard previous.version != 2 || previous.cleanupDone else { throw Failure.pending }
            guard (previous.committed ? previous.target : previous.source) == source.path else { throw Failure.invalidRecord }
        }
        guard BlockDataLocation.destinationAvailable(target, preservingInternalKeys: preservingInternalKeys) else { throw Failure.occupied }
        let directories = Dictionary(uniqueKeysWithValues: BlockDataLocation.movedDirs.compactMap { name in
            identity(source.appendingPathComponent(name)).map { (name, $0) }
        })
        if !FileManager.default.fileExists(atPath: target.path) {
            try FileManager.default.createDirectory(at: target, withIntermediateDirectories: false)
        }
        guard let targetID = identity(target), syncDirectory(target), syncDirectory(target.deletingLastPathComponent()),
              identity(source) == sourceID else { throw Failure.unavailable }
        let record = Record(source: source.path, target: target.path, sourceID: sourceID, targetID: targetID,
                            directories: directories)
        try save(record, in: internalRoot)
    }

    static func pending(in internalRoot: URL) throws -> Bool {
        guard let r = try load(internalRoot) else { return false }
        return r.version == 2 && !r.cleanupDone
    }

    /// A snapshot height alone is not a followed block. Require a later
    /// height with its certificate served by the attested local process.
    /// Persist both milestones so app/node restarts can resume the move.
    @discardableResult
    static func observe(confirmedTarget: URL, internalRoot: URL, answered: Bool,
                        height: UInt64?, certifiedHeight: UInt64?) throws -> Bool {
        guard var r = try load(internalRoot), r.version == 2, r.committed,
              r.target == confirmedTarget.path, identity(confirmedTarget) == r.targetID,
              answered, let height else { return false }
        if r.ready { return true }
        if let first = r.answeredHeight {
            guard height > first, certifiedHeight == height else { return false }
            r.ready = true
        } else {
            r.answeredHeight = height
        }
        try save(r, in: internalRoot)
        return r.ready
    }

    /// Durable choices outlive asynchronous preferences and installer kills.
    static func authoritativeRoot(in internalRoot: URL) throws -> URL? {
        guard let r = try load(internalRoot) else { return nil }
        return URL(fileURLWithPath: r.committed ? r.target : r.source, isDirectory: true)
    }

    /// A different directory at the same path is never a fresh-store fallback.
    static func selectionAvailable(_ selected: URL, internalRoot: URL) -> Bool {
        do {
            guard let r = try load(internalRoot) else { return true }
            let path = r.committed ? r.target : r.source
            let id = r.committed ? r.targetID : r.sourceID
            return selected.path == path && identity(selected) == id
        } catch { return false }
    }

    /// Delete only inside a pinned block-data directory. Key/identity names
    /// and symlinks survive, including case aliases and nested key files.
    private static func removeBlockData(in dir: URL, expected: Identity) -> Bool {
        guard identity(dir) == expected, let names = try? FileManager.default.contentsOfDirectory(atPath: dir.path) else { return false }
        for name in names {
            if BlockDataLocation.keepsInternal(name) { continue }
            guard identity(dir) == expected else { return false }
            let item = dir.appendingPathComponent(name)
            if let id = identity(item) {
                guard removeBlockData(in: item, expected: id) else { return false }
                // Nonempty directories may contain keys or preserved links.
                if rmdir(item.path) != 0 && errno != ENOTEMPTY { return false }
            } else if identity(item, directory: false) != nil {
                guard unlink(item.path) == 0 || errno == ENOENT else { return false }
            }
        }
        return identity(dir) == expected && syncDirectory(dir)
    }

    /// Only now does the old block data go. Unavailable/replaced disks
    /// retain their journal for retry; no old database is hashed or read.
    @discardableResult
    static func cleanup(confirmedTarget: URL, internalRoot: URL) -> Bool {
        guard var r = try? load(internalRoot), r.version == 2, r.committed, r.ready,
              r.target == confirmedTarget.path, identity(confirmedTarget) == r.targetID else { return false }
        if r.cleanupDone { return true }
        let source = URL(fileURLWithPath: r.source)
        guard identity(source) == r.sourceID, BlockDataLocation.disjoint(source, confirmedTarget) else { return false }
        for (name, id) in r.directories {
            let dir = source.appendingPathComponent(name)
            var st = stat()
            if lstat(dir.path, &st) != 0 {
                guard errno == ENOENT else { return false }
                continue
            }
            guard identity(source) == r.sourceID, removeBlockData(in: dir, expected: id) else { return false }
            if rmdir(dir.path) != 0 && errno != ENOTEMPTY { return false }
        }
        guard syncDirectory(source), identity(confirmedTarget) == r.targetID else { return false }
        r.cleanupDone = true
        do { try save(r, in: internalRoot); return true } catch { return false }
    }
}
