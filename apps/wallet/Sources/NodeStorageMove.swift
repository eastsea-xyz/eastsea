#if os(macOS)
import AppKit
import Foundation

/// 블록 데이터 위치: moving the node's chain data to a disk the person picks,
/// safely — stop the node, persist its new place, then start empty and sync.
/// Old data goes only after a response and a followed certified block there.
/// Keys never move (`BlockDataLocation`). An unplugged disk is a stop reason
/// ("디스크가 연결되지 않음"), never a silent re-sync onto the internal disk.
extension NodeController {
    /// UserDefaults: the old chain root to clear once the node runs from the new one.
    static let cleanupKey = "nodeChainDataCleanup"

    /// Claim the storage source only after proving a current writer lease.
    /// The marker is already suspended; the caller owns the returned fd.
    func acquireStorageMoveOwnership() async -> Int32? {
        guard storageMovePreparing, !updateInProgress, !Task.isCancelled else { return nil }
        let ownPID = process?.processIdentifier
        let daemonPID = unattended?.runningNodePID
        let wasAttached = attached
        if let ownPID, let daemonPID, ownPID != daemonPID { return nil }
        if let rootPID = ownPID ?? daemonPID {
            guard let expected = Self.helperBinaryURL,
                  let sample = await LocalRPC.callVerified(rootPID: rootPID, port: Self.port, expected: expected,
                                                          method: "aether_status", params: []),
                  NodeReleaseIdentity.hasWriterLease(status: sample.value),
                  !Task.isCancelled, !updateInProgress, storageMovePreparing,
                  process?.processIdentifier == ownPID, unattended?.runningNodePID == daemonPID,
                  attached == wasAttached,
                  NodeReleaseIdentity.matches(binding: sample.binding, port: Self.port, expected: expected) else { return nil }
            stop(keepSwitch: true)
            if let daemonPID { unattended?.stopDaemonNode(expectedPID: daemonPID) }
            guard let fd = await BlockDataMove.holdRunLock(in: Self.dataDir, timeout: Self.storageMoveLockTimeout) else { return nil }
            guard !Task.isCancelled, !updateInProgress, storageMovePreparing else { close(fd); return nil }
            return fd
        }
        // No claimed parent is not proof of absence. Never wait for an
        // unknown parent to disappear and leave an unleased child writing.
        guard !wasAttached, await LocalRPC.endpointIsAbsent(port: Self.port),
              !Task.isCancelled, !updateInProgress, storageMovePreparing,
              process == nil, unattended?.runningNodePID == nil, !attached,
              let fd = await BlockDataMove.holdRunLock(in: Self.dataDir, timeout: 0) else { return nil }
        guard !Task.isCancelled, !updateInProgress, storageMovePreparing,
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

    /// Validate both picked destinations and a return to the default.
    /// Neither validation nor preparation reads the old database.
    func problem(with picked: URL) -> String? {
        problemMovingBlockData(to: BlockDataLocation.chainDir(picked: picked))
    }

    func problemMovingBlockData(to dest: URL?) -> String? {
        let target = BlockDataLocation.resolvedRoot(dest ?? Self.dataDir)
        guard BlockDataLocation.disjoint(target, chainRoot) else {
            return BlockDataLocation.sentence(.inUse)
        }
        guard BlockDataLocation.destinationAvailable(target, preservingInternalKeys: dest == nil) else {
            return String(localized: "This folder already holds block data. Pick an empty folder; existing data will be kept.")
        }
        var parent = target
        while !FileManager.default.fileExists(atPath: parent.path), parent.pathComponents.count > 1 { parent.deleteLastPathComponent() }
        guard let v = Self.volume(of: parent) else {
            return String(localized: "This place cannot be read. Pick another folder.")
        }
        if let why = KeySafety.iCloudRefusal(path: target.path, home: FileManager.default.homeDirectoryForCurrentUser.path,
                                             ubiquitous: KeySafety.isUbiquitous(parent)) { return why }
        if dest != nil {
            let names = ((try? FileManager.default.contentsOfDirectory(atPath: parent.path)) ?? [])
                + ((try? FileManager.default.contentsOfDirectory(atPath: target.path)) ?? [])
            if let why = KeySafety.keysRefusal(found: KeySafety.keysFound(in: names.map { $0.lowercased() })) { return why }
        }
        let problem = BlockDataLocation.validateFresh(v)
        storageMoveOffersDiskUtility = problem?.fixInDiskUtility == true
        if let problem { return BlockDataLocation.sentence(problem, returningToDefault: dest == nil) }
        if !Self.canWrite(in: parent) { return BlockDataLocation.sentence(.readOnly) }
        return nil
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
        confirmBlockDataMove(to: BlockDataLocation.chainDir(picked: url))
    }

    /// The confirmation explains the fresh sync, including archive rebuilding.
    func confirmBlockDataMove(to dest: URL?) {
        if let why = problemMovingBlockData(to: dest) { storageMoveError = why; return }
        let alert = NSAlert()
        alert.messageText = String(localized: "Start fresh at the new place?")
        alert.informativeText = BlockDataLocation.confirmation(archive: archive)
        alert.addButton(withTitle: String(localized: "Start Fresh"))
        alert.addButton(withTitle: String(localized: "Cancel"))
        if let window = NSApp.keyWindow {
            alert.beginSheetModal(for: window) { response in
                guard response == .alertFirstButtonReturn else { return }
                Task { @MainActor in self.moveBlockData(to: dest) }
            }
        } else if alert.runModal() == .alertFirstButtonReturn {
            moveBlockData(to: dest)
        }
    }

    /// Start fresh at dest (nil: the default), holding the same writer lease.
    func moveBlockData(to dest: URL?) {
        guard !storageMovePreparing, storageMoveSync == nil, !updateInProgress else { return }
        let source = BlockDataLocation.resolvedRoot(chainRoot)
        let target = BlockDataLocation.resolvedRoot(dest ?? Self.dataDir)
        guard let sourceID = BlockDataMove.identity(source) else {
            storageMoveError = String(localized: "The source disk is unavailable. Reconnect it before moving the block data.")
            return
        }
        if let why = problemMovingBlockData(to: dest) { storageMoveError = why; return }
        storageMoveError = nil
        storageMoveOffersDiskUtility = false
        storageMovePreparing = true
        logEvent("storage", "starting fresh from \(source.path) at \(target.path)")
        if let daemon = unattended, !daemon.suspendRespawn() {
            storageMoveError = String(localized: "The node could not pause automatic restarts. The block data is still where it was.")
            storageMovePreparing = false
            daemon.resumeRespawn()
            return
        }
        let dataDir = Self.dataDir
        Task { @MainActor in
            await self.storageMoveCleanup?.value
            guard let moveFD = await self.acquireStorageMoveOwnership() else {
                self.storageMoveError = String(localized: "The running node could not be stopped safely. The move was cancelled; the block data is still where it was.")
                self.storageMovePreparing = false
                self.unattended?.resumeRespawn()
                self.applyPower()
                return
            }
            Task.detached {
                do {
                    try BlockDataMove.prepare(source: source, target: target, sourceID: sourceID,
                                              internalRoot: dataDir, preservingInternalKeys: dest == nil)
                    await MainActor.run {
                        self.chainDataPath = dest?.path ?? ""
                        self.storageMoveSync = BlockDataMove.Progress()
                        self.logEvent("storage", "saved fresh block-data location \(target.path); syncing from the network")
                    }
                } catch {
                    await MainActor.run {
                        self.storageMoveError = (error as? BlockDataMove.Failure) == .pending
                            ? String(localized: "Wait until the node has answered and followed a block at the new location before changing it again.")
                            : String(localized: "The new location could not be saved. The block data is still where it was.")
                        self.logEvent("storage", "fresh-start preparation failed; staying on \(source.path)")
                    }
                }
                await MainActor.run {
                    // Journal and preferences are settled before replacement
                    // writers can acquire the internal lock.
                    close(moveFD)
                    self.storageMovePreparing = false
                    self.unattended?.resumeRespawn()
                    self.applyPower()
                }
            }
        }
    }

    /// Answering alone cannot authorize deleting the old block data.
    /// Called for every attested status, including an attached daemon node.
    func finishBlockDataMove(answered: Bool, height: UInt64?, certifiedHeight: UInt64?, networkHeight: UInt64?) {
        guard storageMoveSync != nil, !storageMovePreparing else { return }
        Self.storageMoveDefaults.removeObject(forKey: Self.cleanupKey)
        let target = BlockDataLocation.resolvedRoot(chainRoot), internalRoot = Self.dataDir
        let progress = BlockDataMove.Progress(height: height ?? 0, target: max(height ?? 0, networkHeight ?? 0))
        storageMoveSync = progress
        let previous = storageMoveCleanup
        storageMoveCleanup = Task.detached {
            await previous?.value
            do {
                let ready = try BlockDataMove.observe(confirmedTarget: target, internalRoot: internalRoot,
                                                     answered: answered, height: height, certifiedHeight: certifiedHeight)
                let cleaned = ready && BlockDataMove.cleanup(confirmedTarget: target, internalRoot: internalRoot)
                await MainActor.run {
                    guard BlockDataLocation.resolvedRoot(self.chainRoot).path == target.path, !self.storageMovePreparing else { return }
                    if cleaned && progress.target > 0 && progress.height >= progress.target { self.storageMoveSync = nil }
                }
            } catch {
                // Keep both locations and retry the small journal on the next
                // response; never restart or read the old database for this.
                await MainActor.run { self.logEvent("storage", "fresh-start confirmation deferred: \(error.localizedDescription)") }
            }
        }
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
