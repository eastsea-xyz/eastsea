import Foundation

/// Keys stay on this Mac and out of backups (docs/design/36-apple-platform.md
/// §6.2, §11 N2/N3). The real hazard for a validator key is a second running
/// copy: a Time Machine restore or Migration Assistant onto a new Mac, an
/// external disk plugged into another Mac, an iCloud-synced folder. So:
/// - N2: keys live only in the node's data folder on the internal disk (the
///   node's `--data`; the block data goes to `--chain-data`). The block-data
///   mover never takes a key, and a chosen block-data folder that holds one
///   is refused.
/// - N3: key files are excluded from Time Machine, and nothing of the node's
///   goes under an iCloud-synced Desktop, Documents or iCloud Drive.
/// Pure decisions here (Tests/key-safety); `apply` does the file-system part.
enum KeySafety {
    /// The node's key files, relative to its data folder.
    static let keyFiles: Set<String> = [
        "validator.key", "validator.pub.json", "node-account.key", "threshold.json",
        "devicecheck-token", "key-binding.json", "follow/wallet-node.key",
    ]
    /// Prefixes of signing journals that are as precious as keys.
    static let keyPrefixes = ["aether-consensus", "dkg-agreement-", "vote-epoch-"]

    static func isKey(_ relative: String) -> Bool {
        keyFiles.contains(relative) || relative == "wallet-node.key" || keyPrefixes.contains { relative.hasPrefix($0) }
    }

    /// The key files among a folder's top-level names.
    static func keysFound(in names: [String]) -> [String] {
        names.filter { isKey($0) }.sorted()
    }

    static func keysRefusal(found: [String], ko: Bool) -> String? {
        guard !found.isEmpty else { return nil }
        return ko ? "이 폴더에 노드 키(\(found.joined(separator: ", ")))가 있어요. 키는 이 Mac에만 두어야 해요. 키가 없는 빈 폴더를 골라 주세요."
            : "This folder holds node keys (\(found.joined(separator: ", "))). Keys must stay on this Mac. Pick an empty folder without keys."
    }

    /// The key folder must be on the internal disk.
    static func keyDirAllowed(path: String, isInternal: Bool) -> Bool {
        isInternal && !(path as NSString).standardizingPath.hasPrefix("/Volumes/")
    }

    /// Which present files to exclude from Time Machine: exactly the keys.
    static func backupExclusions(present: [String]) -> [String] {
        present.filter { name in keyFiles.contains(name) || keyPrefixes.contains { name.hasPrefix($0) } }.sorted()
    }

    /// A folder synced by iCloud (Desktop & Documents, or iCloud Drive) must
    /// not hold the node's data: a synced copy is a second copy.
    static func iCloudRefusal(path: String, home: String, ubiquitous: Bool, ko: Bool) -> String? {
        let p = (path as NSString).standardizingPath
        let inDrive = p.hasPrefix(home + "/Library/Mobile Documents/")
        let inSyncable = ["Desktop", "Documents"].contains { p == home + "/" + $0 || p.hasPrefix(home + "/" + $0 + "/") }
        guard inDrive || (inSyncable && ubiquitous) else { return nil }
        return ko ? "iCloud로 동기화되는 폴더에는 둘 수 없어요. 다른 Mac에 사본이 생기기 때문이에요. 다른 위치를 골라 주세요."
            : "A folder iCloud syncs cannot hold it: another Mac would get a copy. Pick another place."
    }

    #if os(macOS)
    /// Exclude the key files in `dataDir` from Time Machine (sticky, follows
    /// the file): `NSURLIsExcludedFromBackupKey`, with `tmutil addexclusion`
    /// as the fallback when the resource value cannot be set. Best effort,
    /// idempotent; returns what it excluded.
    @discardableResult
    static func excludeKeysFromBackup(in dataDir: URL) -> [String] {
        let fm = FileManager.default
        let present = keyFiles.filter { fm.fileExists(atPath: dataDir.appendingPathComponent($0).path) }
            + ((try? fm.contentsOfDirectory(atPath: dataDir.path)) ?? []).filter { n in keyPrefixes.contains { n.hasPrefix($0) } }
        var done: [String] = []
        for rel in backupExclusions(present: present) {
            var url = dataDir.appendingPathComponent(rel)
            if (try? url.resourceValues(forKeys: [.isExcludedFromBackupKey]))?.isExcludedFromBackup == true { done.append(rel); continue }
            var values = URLResourceValues()
            values.isExcludedFromBackup = true
            if (try? url.setResourceValues(values)) != nil {
                done.append(rel)
            } else {
                let p = Process()
                p.executableURL = URL(fileURLWithPath: "/usr/bin/tmutil")
                p.arguments = ["addexclusion", url.path]
                if (try? p.run()) != nil { p.waitUntilExit(); if p.terminationStatus == 0 { done.append(rel) } }
            }
        }
        return done
    }

    /// Whether `url` is in iCloud (Desktop & Documents sync or iCloud Drive).
    static func isUbiquitous(_ url: URL) -> Bool {
        var probe = url
        while !FileManager.default.fileExists(atPath: probe.path), probe.pathComponents.count > 1 { probe.deleteLastPathComponent() }
        return (try? probe.resourceValues(forKeys: [.isUbiquitousItemKey]))?.isUbiquitousItem == true
    }
    #endif
}
