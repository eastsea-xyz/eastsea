#if os(macOS)
import AppKit
import Foundation

/// 블록 데이터 위치: moving the node's chain data to a disk the person picks,
/// safely — stop the node, copy, verify every file by SHA-256, switch, and
/// remove the old copy only after the node has answered from the new place.
/// Keys never move (`BlockDataLocation`). An unplugged disk is a stop reason
/// ("디스크가 연결되지 않음"), never a silent re-sync onto the internal disk.
extension NodeController {
    /// UserDefaults: the old chain root to clear once the node runs from the new one.
    static let cleanupKey = "nodeChainDataCleanup"

    /// Claim the storage source only after proving a current writer lease.
    /// The marker is already suspended; the caller owns the returned fd.
    func acquireStorageMoveOwnership() async -> Int32? {
        guard storageMovePercent != nil, !updateInProgress, !Task.isCancelled else { return nil }
        let ownPID = process?.processIdentifier
        let daemonPID = unattended?.runningNodePID
        let wasAttached = attached
        if let ownPID, let daemonPID, ownPID != daemonPID { return nil }
        if let rootPID = ownPID ?? daemonPID {
            guard let expected = Self.helperBinaryURL,
                  let sample = await LocalRPC.callVerified(rootPID: rootPID, port: Self.port, expected: expected,
                                                          method: "aether_status", params: []),
                  NodeReleaseIdentity.hasWriterLease(status: sample.value),
                  !Task.isCancelled, !updateInProgress, storageMovePercent != nil,
                  process?.processIdentifier == ownPID, unattended?.runningNodePID == daemonPID,
                  attached == wasAttached,
                  NodeReleaseIdentity.matches(binding: sample.binding, port: Self.port, expected: expected) else { return nil }
            stop(keepSwitch: true)
            if let daemonPID { unattended?.stopDaemonNode(expectedPID: daemonPID) }
            guard let fd = await BlockDataMove.holdRunLock(in: Self.dataDir, timeout: Self.storageMoveLockTimeout) else { return nil }
            guard !Task.isCancelled, !updateInProgress, storageMovePercent != nil else { close(fd); return nil }
            return fd
        }
        // No claimed parent is not proof of absence. Never wait for an
        // unknown parent to disappear and leave an unleased child writing.
        guard !wasAttached, await LocalRPC.endpointIsAbsent(port: Self.port),
              !Task.isCancelled, !updateInProgress, storageMovePercent != nil,
              process == nil, unattended?.runningNodePID == nil, !attached,
              let fd = await BlockDataMove.holdRunLock(in: Self.dataDir, timeout: 0) else { return nil }
        guard !Task.isCancelled, !updateInProgress, storageMovePercent != nil,
              process == nil, unattended?.runningNodePID == nil, !attached else { close(fd); return nil }
        stop(keepSwitch: true)
        return fd
    }

    /// The chain-data root in use: the chosen folder, or the node's data folder.
    var chainRoot: URL {
        chainDataPath.isEmpty ? Self.dataDir : URL(fileURLWithPath: chainDataPath, isDirectory: true)
    }

    /// Where the block data is, as the start gate sees it. A chosen folder
    /// that is not there means its disk is not connected; the node is never
    /// started at a path that would land on the internal disk.
    func blockDataStorageState() -> NodeStorageState {
        let pinned = BlockDataMove.selectionAvailable(BlockDataLocation.resolvedRoot(chainRoot), internalRoot: Self.dataDir)
        guard !chainDataPath.isEmpty else {
            return pinned ? .standard : .chosen(volume: String(localized: "Block data"), mounted: false, writable: false)
        }
        let url = URL(fileURLWithPath: chainDataPath, isDirectory: true)
        let volume = BlockDataLocation.volumeName(ofPath: chainDataPath) ?? url.deletingLastPathComponent().lastPathComponent
        var isDir: ObjCBool = false
        let mounted = pinned && FileManager.default.fileExists(atPath: url.path, isDirectory: &isDir) && isDir.boolValue
        return .chosen(volume: volume, mounted: mounted, writable: mounted && Self.canWrite(in: url))
    }

    /// A real write (macOS privacy decides at open time, not at `access`).
    /// The first one on a removable disk is what raises the system's
    /// "EastSea wants to access files on a removable volume" prompt.
    nonisolated static func canWrite(in dir: URL) -> Bool {
        let probe = dir.appendingPathComponent(".eastsea-write-check-\(UUID().uuidString)")
        guard (try? Data().write(to: probe, options: .withoutOverwriting)) != nil else { return false }
        try? FileManager.default.removeItem(at: probe)
        return true
    }

    /// The facts about the disk `url` is on.
    nonisolated static func volume(of url: URL) -> BlockDataLocation.Volume? {
        var st = statfs()
        guard statfs(url.path, &st) == 0 else { return nil }
        let format = withUnsafeBytes(of: st.f_fstypename) { raw in
            String(decoding: raw.prefix { $0 != 0 }, as: UTF8.self)
        }.lowercased()
        let keys: Set<URLResourceKey> = [.volumeNameKey, .volumeIsLocalKey, .volumeIsReadOnlyKey, .volumeIsInternalKey]
        let v = try? url.resourceValues(forKeys: keys)
        return BlockDataLocation.Volume(name: v?.volumeName ?? url.lastPathComponent,
                                        format: format,
                                        isLocal: v?.volumeIsLocal ?? ((st.f_flags & UInt32(MNT_LOCAL)) != 0),
                                        isReadOnly: v?.volumeIsReadOnly ?? ((st.f_flags & UInt32(MNT_RDONLY)) != 0),
                                        isInternal: v?.volumeIsInternal ?? false,
                                        freeBytes: UInt64(st.f_bavail) * UInt64(st.f_bsize))
    }

    /// Bytes under `url` (0 when it does not exist).
    nonisolated static func treeBytes(_ url: URL) -> UInt64 {
        guard let e = FileManager.default.enumerator(at: url, includingPropertiesForKeys: [.fileSizeKey, .isRegularFileKey]) else { return 0 }
        var total: UInt64 = 0
        for case let f as URL in e {
            let v = try? f.resourceValues(forKeys: [.fileSizeKey, .isRegularFileKey])
            if v?.isRegularFile == true { total += UInt64(v?.fileSize ?? 0) }
        }
        return total
    }

    /// The block data's size now (what a move copies).
    var blockDataBytes: UInt64 {
        BlockDataLocation.movedDirs.reduce(0) { $0 + Self.treeBytes(chainRoot.appendingPathComponent($1)) }
    }

    /// Why a picked folder cannot hold the block data, or nil.
    func problem(with picked: URL) -> String? {
        let dest = BlockDataLocation.chainDir(picked: picked)
        if !BlockDataLocation.disjoint(dest, chainRoot) {
            return BlockDataLocation.sentence(.inUse)
        }
        guard BlockDataLocation.destinationAvailable(dest, preservingInternalKeys: false) else {
            return String(localized: "This folder already holds block data. Pick an empty folder; existing data will be kept.")
        }
        guard let v = Self.volume(of: picked) else {
            return String(localized: "This place cannot be read. Pick another folder.")
        }
        // Design 36 N3: never under an iCloud-synced folder (a second copy).
        if let why = KeySafety.iCloudRefusal(path: picked.path, home: FileManager.default.homeDirectoryForCurrentUser.path,
                                             ubiquitous: KeySafety.isUbiquitous(picked)) {
            return why
        }
        // Design 36 N2: a folder that already holds node keys is refused —
        // keys stay on this Mac's internal disk only.
        let names = ((try? FileManager.default.contentsOfDirectory(atPath: picked.path)) ?? [])
            + ((try? FileManager.default.contentsOfDirectory(atPath: dest.path)) ?? [])
        if let why = KeySafety.keysRefusal(found: KeySafety.keysFound(in: names)) { return why }
        let problem = BlockDataLocation.validate(v, dataBytes: blockDataBytes)
        storageMoveOffersDiskUtility = problem?.fixInDiskUtility == true
        return problem.map { BlockDataLocation.sentence($0) }
    }

    /// Disk Utility, where an exFAT/FAT disk is erased as APFS.
    func openDiskUtility() {
        NSWorkspace.shared.open(URL(fileURLWithPath: "/System/Applications/Utilities/Disk Utility.app"))
    }

    /// NSOpenPanel → validate → move.
    func chooseBlockDataLocation() {
        let panel = NSOpenPanel()
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.canCreateDirectories = true
        panel.allowsMultipleSelection = false
        panel.directoryURL = URL(fileURLWithPath: "/Volumes", isDirectory: true)
        panel.prompt = String(localized: "Store Here")
        panel.message = String(localized: "Pick a folder for the block data. A “\(BlockDataLocation.folderName)” folder is made inside it. The keys stay on this Mac.")
        guard panel.runModal() == .OK, let url = panel.url else { return }
        if let why = problem(with: url) {
            storageMoveError = why
            return
        }
        moveBlockData(to: BlockDataLocation.chainDir(picked: url))
    }

    /// Move the block data to `dest` (nil: back to the default place).
    func moveBlockData(to dest: URL?) {
        guard storageMovePercent == nil, !updateInProgress else { return }
        let source = BlockDataLocation.resolvedRoot(chainRoot)
        let target = BlockDataLocation.resolvedRoot(dest ?? Self.dataDir)
        guard let sourceID = BlockDataMove.identity(source) else {
            storageMoveError = String(localized: "The source disk is unavailable. Reconnect it before moving the block data.")
            return
        }
        guard BlockDataLocation.disjoint(source, target) else {
            storageMoveError = String(localized: "Pick a folder outside the current block data. The two locations cannot contain each other.")
            return
        }
        guard BlockDataLocation.destinationAvailable(target, preservingInternalKeys: dest == nil)
                || BlockDataMove.canResume(source: source, target: target, internalRoot: Self.dataDir) else {
            storageMoveError = String(localized: "This folder already holds block data. Pick an empty folder; existing data will be kept.")
            return
        }
        storageMoveError = nil
        storageMoveOffersDiskUtility = false
        storageMovePercent = 0
        logEvent("storage", "moving block data from \(source.path) to \(target.path)")
        // Suspend before shutdown: the daemon may otherwise restart the
        // source while a large database copy is still working.
        if let daemon = unattended, !daemon.suspendRespawn() {
            storageMoveError = String(localized: "The node could not pause automatic restarts. The block data is still where it was.")
            storageMovePercent = nil
            daemon.resumeRespawn()
            return
        }
        let dataDir = Self.dataDir
        Task { @MainActor in
            // A released parent lock cannot prove legacy children are gone.
            // Fresh signed lease preflight precedes every source shutdown.
            guard let moveFD = await self.acquireStorageMoveOwnership() else {
                self.storageMoveError = String(localized: "The running node could not be stopped safely. The move was cancelled; the block data is still where it was.")
                self.storageMovePercent = nil
                self.unattended?.resumeRespawn()
                self.applyPower()
                return
            }
            Task.detached {
                let total = BlockDataLocation.movedDirs.reduce(UInt64(0)) { $0 + Self.treeBytes(source.appendingPathComponent($1)) }
                let meter = DataMigration.ProgressMeter { f in
                    Task { @MainActor in
                        if self.storageMovePercent != nil { self.storageMovePercent = min(99, Int(f * 100)) }
                    }
                }
                meter.expect(Int64(total) * 3)
                let copied = (try? BlockDataMove.copy(source: source, target: target, sourceID: sourceID,
                                                     internalRoot: dataDir, preservingInternalKeys: dest == nil, meter: meter)) != nil
                await MainActor.run {
                    if copied {
                        self.chainDataPath = dest?.path ?? ""
                        self.logEvent("storage", "copied and verified \(NodeStopReason.gb(total)); the node now uses \(target.path)")
                    } else {
                        // Nothing switched: the old copy is untouched and stays in use.
                        self.storageMoveError = String(localized: "The move did not complete: the copy did not verify or space ran out. The block data is still where it was.")
                        self.logEvent("storage", "move failed; staying on \(source.path)")
                    }
                    // The durable record and UI selection are settled. Release
                    // before either daemon or app tries to start the replacement.
                    close(moveFD)
                    self.storageMovePercent = nil
                    self.unattended?.resumeRespawn()
                    self.applyPower()
                }
            }
        }
    }

    /// The node answered from its new place: only now does the old copy go
    /// (its block-data folders only — never the keys, never anything else).
    func finishBlockDataMove() {
        // Legacy path-only records carry no proof and authorize no deletion.
        Self.storageMoveDefaults.removeObject(forKey: Self.cleanupKey)
        let target = BlockDataLocation.resolvedRoot(chainRoot), internalRoot = Self.dataDir
        Task.detached { BlockDataMove.cleanup(confirmedTarget: target, internalRoot: internalRoot) }
    }

    /// Turn archive off: back to a normal follower; the archive's extra
    /// history is deleted only when the person says so.
    func turnArchiveOff(deleteHistory: Bool) {
        archive = false
        guard deleteHistory else { return }
        let dir = chainRoot.appendingPathComponent("archive")
        logEvent("archive", "archive off; deleting \(dir.path) on request")
        Task.detached { try? FileManager.default.removeItem(at: dir) }
    }

    /// Disks coming and going: re-run the start gate at once (a chosen disk
    /// back → the node resumes), and stop the node before its disk is ejected
    /// so the eject succeeds and the database closes cleanly.
    func watchVolumes() {
        guard mountObservers.isEmpty else { return }
        let center = NSWorkspace.shared.notificationCenter
        mountObservers = [
            center.addObserver(forName: NSWorkspace.didMountNotification, object: nil, queue: .main) { [weak self] _ in
                MainActor.assumeIsolated { self?.applyPower() }
            },
            center.addObserver(forName: NSWorkspace.didUnmountNotification, object: nil, queue: .main) { [weak self] _ in
                MainActor.assumeIsolated { self?.applyPower() }
            },
            center.addObserver(forName: NSWorkspace.willUnmountNotification, object: nil, queue: .main) { [weak self] note in
                MainActor.assumeIsolated {
                    guard let self, !self.chainDataPath.isEmpty,
                          let path = (note.userInfo?[NSWorkspace.volumeURLUserInfoKey] as? URL)?.path,
                          self.chainDataPath.hasPrefix(path + "/") else { return }
                    self.logEvent("storage", "the block-data disk is being ejected; stopping the node")
                    self.stop(keepSwitch: true)
                    self.applyPower()
                }
            },
        ]
    }
}
#endif
