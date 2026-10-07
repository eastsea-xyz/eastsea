import Foundation
import CryptoKit

/// The Aether 0.6.7 bridge's decisions, free of side effects (Tests/bridge-plan).
///
/// Why a bridge exists (release-070 review, B2): Sparkle in Aether 0.6.6 only
/// installs an archive whose app is `Aether.app` / bundle id
/// `com.pipln.aether`, so it can never update a user to `EastSea.app`
/// (`com.pipln.eastsea`). Aether 0.6.7 is still `Aether.app`, reaches every
/// 0.6.6 through the old feed, and its one job is to put a verified EastSea in
/// place and step aside. It moves no user data: EastSea's own first-launch
/// migration does that, verified, once the old node has stopped.
enum BridgePlan {
    static let eastSeaBundleID = "com.pipln.eastsea"
    static let eastSeaAppName = "EastSea.app"
    /// Pipln's Developer ID team: the only signer an installed EastSea may have.
    static let teamID = "45WU468FZE"
    static let minimumEastSea = "0.7.0"
    /// EastSea's own Sparkle feed (0.7.0+). Never the legacy appcast.xml,
    /// which lists this bridge.
    static let feedURL = URL(string: "https://github.com/eastsea-xyz/eastsea/releases/latest/download/eastsea-appcast.xml")!
    static let releasesPage = URL(string: "https://github.com/eastsea-xyz/eastsea/releases/latest")!
    /// Downloads come only from this repository's release assets.
    static let downloadHost = "github.com"
    static let downloadPathPrefix = "/eastsea-xyz/eastsea/releases/download/"

    /// The code requirement EastSea must meet before it is installed or
    /// launched: Apple-anchored Developer ID (the intermediate and leaf
    /// Developer ID extensions), Pipln's team, EastSea's bundle id.
    /// Notarization is checked separately (Gatekeeper's assessment).
    static var requirement: String {
        "anchor apple generic and identifier \"\(eastSeaBundleID)\""
            + " and certificate 1[field.1.2.840.113635.100.6.2.6] exists"
            + " and certificate leaf[field.1.2.840.113635.100.6.1.13] exists"
            + " and certificate leaf[subject.OU] = \"\(teamID)\""
    }

    // MARK: the feed

    struct Item: Equatable {
        var build = ""
        var version = ""
        var url: URL?
        var edSignature = ""
        var length: Int64 = 0
        var channel: String?
        var minimumSystemVersion: String?
    }

    /// The items of a Sparkle appcast (RSS with the sparkle namespace).
    static func parseAppcast(_ data: Data) -> [Item] {
        let reader = AppcastReader()
        let parser = XMLParser(data: data)
        parser.shouldProcessNamespaces = false
        parser.delegate = reader
        guard parser.parse() else { return [] }
        return reader.items
    }

    /// The EastSea to install: the newest item (by build number) that is on
    /// the default channel, at least `minimumEastSea`, runs on this macOS,
    /// is signed, has a length, and downloads from this repository's
    /// releases over https.
    static func choose(_ items: [Item], systemVersion: String) -> Item? {
        items.filter { item in
            guard let url = item.url, isAllowedDownload(url) else { return false }
            guard item.channel == nil, !item.edSignature.isEmpty, item.length > 0 else { return false }
            guard compareVersions(item.version, minimumEastSea) != .orderedAscending else { return false }
            if let min = item.minimumSystemVersion, compareVersions(systemVersion, min) == .orderedAscending { return false }
            return Int(item.build) != nil
        }
        .max { (Int($0.build) ?? 0) < (Int($1.build) ?? 0) }
    }

    static func isAllowedDownload(_ url: URL) -> Bool {
        url.scheme == "https" && url.host == downloadHost
            && url.path.hasPrefix(downloadPathPrefix) && url.pathExtension == "dmg"
            && !url.path.contains("/../")
    }

    /// Dotted numeric comparison; missing parts are 0.
    static func compareVersions(_ a: String, _ b: String) -> ComparisonResult {
        let x = a.split(separator: ".").map { Int($0) ?? 0 }
        let y = b.split(separator: ".").map { Int($0) ?? 0 }
        for i in 0..<max(x.count, y.count) {
            let l = i < x.count ? x[i] : 0, r = i < y.count ? y[i] : 0
            if l != r { return l < r ? .orderedAscending : .orderedDescending }
        }
        return .orderedSame
    }

    // MARK: verification

    /// Sparkle's EdDSA (Ed25519) signature over the whole archive, checked
    /// with the public key every Aether and EastSea carries (`SUPublicEDKey`).
    static func verifyEdDSA(file: URL, signatureBase64: String, publicKeyBase64: String) -> Bool {
        guard let keyBytes = Data(base64Encoded: publicKeyBase64),
              let key = try? Curve25519.Signing.PublicKey(rawRepresentation: keyBytes),
              let signature = Data(base64Encoded: signatureBase64),
              let data = try? Data(contentsOf: file, options: .alwaysMapped) else { return false }
        return key.isValidSignature(signature, for: data)
    }

    /// `spctl --assess --type execute --verbose=2` accepted the app as a
    /// notarized Developer ID app.
    static func gatekeeperAccepts(output: String, exitCode: Int32) -> Bool {
        exitCode == 0 && output.contains("accepted") && output.contains("source=Notarized Developer ID")
    }

    // MARK: the old node

    /// The old node processes of `uid` in `ps -axww -o pid=,uid=,args=`
    /// output: the `aether` helper of an Aether.app bundle (the app's child,
    /// or a CLI run through the ~/.local/bin link into it), never EastSea's.
    static func oldNodePIDs(psOutput: String, uid: uid_t) -> [pid_t] {
        psOutput.split(separator: "\n").compactMap { line in
            let parts = line.split(separator: " ", maxSplits: 2, omittingEmptySubsequences: true)
            guard parts.count == 3, let pid = pid_t(parts[0]), let owner = uid_t(parts[1]), owner == uid else { return nil }
            let exe = parts[2].split(separator: " ").first.map(String.init) ?? ""
            return exe.hasSuffix("/Aether.app/Contents/Helpers/aether")
                || exe.hasSuffix("/Aether.app/Contents/Helpers/aether.prev") ? pid : nil
        }
    }

    // MARK: installing

    /// /Applications when this user may write there (admins), else
    /// ~/Applications — both places EastSea runs from (InstallLocation).
    static func installDirectory(applicationsWritable: Bool, home: URL) -> URL {
        applicationsWritable ? URL(fileURLWithPath: "/Applications", isDirectory: true)
            : home.appendingPathComponent("Applications", isDirectory: true)
    }

    enum Existing: Equatable { case none, keep, replace }

    /// An EastSea already at the destination is kept when it is genuine and
    /// at least as new as the download; otherwise it is replaced (the old
    /// copy goes to the Trash, never deleted).
    static func existingDecision(installedVersion: String?, installedValid: Bool, candidateVersion: String) -> Existing {
        guard let installedVersion else { return .none }
        if installedValid && compareVersions(installedVersion, candidateVersion) != .orderedAscending { return .keep }
        return .replace
    }

    /// An EastSea already installed somewhere is enough (no download) when it
    /// is genuine and at least the minimum.
    static func installedIsEnough(version: String?, valid: Bool) -> Bool {
        guard let version, valid else { return false }
        return compareVersions(version, minimumEastSea) != .orderedAscending
    }
}

/// Collects `<item>`s: `sparkle:version`/`shortVersionString`/`channel`/
/// `minimumSystemVersion` as elements or as enclosure attributes.
private final class AppcastReader: NSObject, XMLParserDelegate {
    var items: [BridgePlan.Item] = []
    private var current: BridgePlan.Item?
    private var text = ""

    func parser(_ parser: XMLParser, didStartElement name: String, namespaceURI: String?,
                qualifiedName: String?, attributes: [String: String] = [:]) {
        text = ""
        if name == "item" { current = BridgePlan.Item() }
        guard name == "enclosure", current != nil else { return }
        current?.url = attributes["url"].flatMap(URL.init(string:))
        current?.edSignature = attributes["sparkle:edSignature"] ?? ""
        current?.length = Int64(attributes["length"] ?? "") ?? 0
        if let v = attributes["sparkle:version"] { current?.build = v }
        if let v = attributes["sparkle:shortVersionString"] { current?.version = v }
    }

    func parser(_ parser: XMLParser, foundCharacters string: String) { text += string }

    func parser(_ parser: XMLParser, didEndElement name: String, namespaceURI: String?, qualifiedName: String?) {
        let value = text.trimmingCharacters(in: .whitespacesAndNewlines)
        switch name {
        case "item":
            if let current { items.append(current) }
            current = nil
        case "sparkle:version": current?.build = value
        case "sparkle:shortVersionString": current?.version = value
        case "sparkle:channel": current?.channel = value.isEmpty ? nil : value
        case "sparkle:minimumSystemVersion": current?.minimumSystemVersion = value
        default: break
        }
        text = ""
    }
}
