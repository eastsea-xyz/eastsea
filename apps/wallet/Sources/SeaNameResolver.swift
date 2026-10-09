import Foundation
import CryptoKit

/// Deployment pins are supplied per chain, never inferred from a name.
struct SeaRegistrySources: Codable, Sendable {
    struct Pin: Codable, Sendable {
        let address: String
        let codeSHA256: String
        enum CodingKeys: String, CodingKey { case address; case codeSHA256 = "code_sha256" }
        var valid: Bool {
            SeaNameResolver.validHex(address, bytes: 20) && !SeaNameResolver.isZero(address)
                && codeSHA256.count == 64 && SeaNameResolver.validHex("0x" + codeSHA256, bytes: 32)
        }
    }
    let names: Pin
    let apps: Pin
    private struct File: Decodable { let chains: [String: SeaRegistrySources] }

    static func parse(_ data: Data, chainID: UInt64) -> SeaRegistrySources? {
        guard let pins = (try? JSONDecoder().decode(File.self, from: data))?.chains[String(chainID)],
              pins.names.valid, pins.apps.valid else { return nil }
        return pins
    }

    static func bundled(chainID: UInt64) -> SeaRegistrySources? {
        guard let url = Bundle.main.url(forResource: "name-sources", withExtension: "json"),
              let data = try? Data(contentsOf: url) else { return nil }
        return parse(data, chainID: chainID)
    }
}

struct SeaAppRecord: Equatable, Sendable {
    let appID: String
    let sequence: UInt32
    let manifestHash: String
    let bundleHash: String
}

/// The P2P lane supplies verified bytes here. This lane returns no content.
protocol ContentSource: Sendable {
    func page(for app: SeaAppRecord, at link: SeaURL.NameLink) async throws -> ContentPage?
}

struct ContentPage: Sendable {
    let bytes: Data
    let mimeType: String
}

struct PendingContentSource: ContentSource {
    func page(for app: SeaAppRecord, at link: SeaURL.NameLink) async throws -> ContentPage? { nil }
}

/// Read-only name -> app lookup, injected so it can run without a node or UI.
enum SeaNameResolver {
    enum Failure: Error, Equatable { case registryUnavailable, wrongCode, badAnswer, unregistered, expired, noApp, unlistedApp, unstable }
    struct NameRecord: Equatable, Sendable {
        let link: SeaURL.NameLink
        let owner: String
        let address: String
    }
    struct Resolution: Equatable, Sendable {
        let link: SeaURL.NameLink
        let owner: String
        let address: String
        let app: SeaAppRecord
    }
    typealias Read = (_ to: String, _ data: String) async throws -> String

    /// Search can resolve a registered name without requiring an app binding.
    static func lookup(_ link: SeaURL.NameLink, now: UInt64, sources: SeaRegistrySources?, chainID: UInt64 = 1,
                       code: (_ to: String) async throws -> String, read: Read) async throws -> NameRecord {
        guard !link.isLegacy || chainID == 7780 else { throw SeaURL.ParseError.legacyNameUnsupported }
        let raw = "sea://" + link.registryName + link.path + (link.query.map { "?" + $0 } ?? "")
        guard case .name(let canonical) = try SeaURL.parse(raw, chainID: chainID), canonical == link else {
            throw SeaURL.ParseError.invalidName
        }
        guard let pins = sources, pins.names.valid else { throw Failure.registryUnavailable }
        let rawCode = try await code(pins.names.address)
        guard let bytes = decodeBytes(rawCode), !bytes.isEmpty,
              SHA256.hash(data: bytes).map({ String(format: "%02x", $0) }).joined() == pins.names.codeSHA256 else {
            throw Failure.wrongCode
        }
        let registryName = chainID == 7780 ? String(link.name.dropLast(4)) : link.name
        let nodeWords = try words(await read(pins.names.address, "0x864ce5e1" + word(32) + stringArgument(registryName)))
        guard nodeWords.count == 1, !isZero(nodeWords[0]) else { throw Failure.badAnswer }
        let node = nodeWords[0]
        let owner = try address(await read(pins.names.address, "0x7dd56411" + node))
        let expires = try number(await read(pins.names.address, "0x9dfcc616" + node))
        guard !isZero(owner) else { throw Failure.unregistered }
        // Match existing root resolution's grace window and child expiry.
        let grace: UInt64 = link.name.split(separator: ".").count == 2 ? 30 * 24 * 60 * 60 : 0
        let (liveUntil, overflow) = expires.addingReportingOverflow(grace)
        guard expires > 0, !overflow, now < liveUntil else { throw Failure.expired }
        let target = try address(await read(pins.names.address, "0xb25be181" + node))
        return NameRecord(link: link, owner: owner, address: target)
    }

    /// Direct registry app links need no name or name-registry deployment.
    static func resolveApp(appID: String, sources: SeaRegistrySources?,
                           code: (_ to: String) async throws -> String, read: Read) async throws -> SeaAppRecord {
        guard validHex(appID, bytes: 32), !isZero(appID) else { throw Failure.noApp }
        guard let pins = sources, pins.apps.valid else { throw Failure.registryUnavailable }
        let rawCode = try await code(pins.apps.address)
        guard let bytes = decodeBytes(rawCode), !bytes.isEmpty,
              SHA256.hash(data: bytes).map({ String(format: "%02x", $0) }).joined() == pins.apps.codeSHA256 else {
            throw Failure.wrongCode
        }
        let release = try words(await read(pins.apps.address, "0x470d9498" + appID.dropFirst(2)))
        guard release.count == 4 else { throw Failure.badAnswer }
        guard release[3] == word(0) || release[3] == word(1) else { throw Failure.badAnswer }
        guard release[3] == word(1) else { throw Failure.unlistedApp }
        let seq = try number("0x" + release[0])
        guard seq > 0, seq <= UInt64(UInt32.max), !isZero(release[1]), !isZero(release[2]) else { throw Failure.badAnswer }
        return SeaAppRecord(appID: appID, sequence: UInt32(seq),
                            manifestHash: "0x" + release[1], bundleHash: "0x" + release[2])
    }

    static func resolve(_ link: SeaURL.NameLink, now: UInt64, sources: SeaRegistrySources?, chainID: UInt64 = 1,
                        code: (_ to: String) async throws -> String, read: Read) async throws -> Resolution {
        guard !link.isLegacy || chainID == 7780 else { throw SeaURL.ParseError.legacyNameUnsupported }
        guard let pins = sources, pins.names.valid, pins.apps.valid else { throw Failure.registryUnavailable }
        for pin in [pins.names, pins.apps] {
            let raw = try await code(pin.address)
            guard let bytes = decodeBytes(raw), !bytes.isEmpty,
                  SHA256.hash(data: bytes).map({ String(format: "%02x", $0) }).joined() == pin.codeSHA256 else {
                throw Failure.wrongCode
            }
        }
        // 7780's old registry hashes a bare label; new registries accept the
        // canonical hostname. The .aeth alias never reaches a new registry.
        let registryName = chainID == 7780 ? String(link.name.dropLast(4)) : link.name
        let nodeWords = try words(await read(pins.names.address, "0x864ce5e1" + word(32) + stringArgument(registryName)))
        guard nodeWords.count == 1, !isZero(nodeWords[0]) else { throw Failure.badAnswer }
        let node = nodeWords[0]
        let owner = try address(await read(pins.names.address, "0x7dd56411" + node))
        let expires = try number(await read(pins.names.address, "0x9dfcc616" + node))
        guard !isZero(owner) else { throw Failure.unregistered }
        // Root views retain the existing 30-day grace window. Child records
        // expire at the parent's actual expiry, without that window.
        let grace: UInt64 = link.name.split(separator: ".").count == 2 ? 30 * 24 * 60 * 60 : 0
        let (liveUntil, overflow) = expires.addingReportingOverflow(grace)
        guard expires > 0, !overflow, now < liveUntil else { throw Failure.expired }
        let target = try address(await read(pins.names.address, "0xb25be181" + node))
        let appID = try string(await read(pins.names.address, "0xffdbd0e3" + node + word(64) + stringArgument("app")))
        guard validHex(appID, bytes: 32), !isZero(appID) else { throw Failure.noApp }
        let release = try words(await read(pins.apps.address, "0x470d9498" + appID.dropFirst(2)))
        guard release.count == 4 else { throw Failure.badAnswer }
        guard release[3] == word(0) || release[3] == word(1) else { throw Failure.badAnswer }
        guard release[3] == word(1) else { throw Failure.unlistedApp }
        let seq = try number("0x" + release[0])
        guard seq > 0, seq <= UInt64(UInt32.max), !isZero(release[1]), !isZero(release[2]) else { throw Failure.badAnswer }
        return Resolution(link: link, owner: owner, address: target,
                          app: SeaAppRecord(appID: appID, sequence: UInt32(seq),
                                            manifestHash: "0x" + release[1], bundleHash: "0x" + release[2]))
    }

    static func validHex(_ value: String, bytes: Int) -> Bool {
        value.hasPrefix("0x") && value.utf8.count == bytes * 2 + 2
            && value.dropFirst(2).utf8.allSatisfy { (48...57).contains($0) || (97...102).contains($0) }
    }
    static func isZero(_ value: String) -> Bool {
        (value.hasPrefix("0x") ? value.dropFirst(2) : value[...]).allSatisfy { $0 == "0" }
    }
    private static func decodeBytes(_ value: String) -> Data? {
        guard value.hasPrefix("0x"), value.count % 2 == 0 else { return nil }
        let h = Array(value.dropFirst(2))
        var data = Data()
        for i in stride(from: 0, to: h.count, by: 2) {
            guard let byte = UInt8(String(h[i...i + 1]), radix: 16) else { return nil }
            data.append(byte)
        }
        return data
    }
    private static func word(_ value: UInt64) -> String {
        let h = String(value, radix: 16)
        return String(repeating: "0", count: 64 - h.count) + h
    }
    private static func stringArgument(_ value: String) -> String {
        let h = value.utf8.map { String(format: "%02x", $0) }.joined()
        return word(UInt64(value.utf8.count)) + h + String(repeating: "0", count: (64 - h.count % 64) % 64)
    }
    private static func words(_ raw: String) throws -> [String] {
        guard raw.hasPrefix("0x") else { throw Failure.badAnswer }
        let h = Array(raw.dropFirst(2))
        guard !h.isEmpty, h.count % 64 == 0,
              h.allSatisfy({ $0.isASCII && $0.isHexDigit }) else { throw Failure.badAnswer }
        return stride(from: 0, to: h.count, by: 64).map { String(h[$0..<$0 + 64]).lowercased() }
    }
    private static func address(_ raw: String) throws -> String {
        let w = try words(raw)
        guard w.count == 1, w[0].prefix(24).allSatisfy({ $0 == "0" }) else { throw Failure.badAnswer }
        return "0x" + w[0].suffix(40)
    }
    private static func number(_ raw: String) throws -> UInt64 {
        let w = try words(raw)
        guard w.count == 1, w[0].prefix(48).allSatisfy({ $0 == "0" }),
              let n = UInt64(w[0].suffix(16), radix: 16) else { throw Failure.badAnswer }
        return n
    }
    private static func string(_ raw: String) throws -> String {
        let w = try words(raw)
        guard w.count >= 2, w[0] == word(32) else { throw Failure.badAnswer }
        let n = try number("0x" + w[1])
        guard n <= 128, w.count == 2 + (Int(n) + 31) / 32,
              let bytes = decodeBytes("0x" + w.dropFirst(2).joined().prefix(Int(n) * 2)),
              let value = String(data: bytes, encoding: .utf8) else { throw Failure.badAnswer }
        return value
    }
}

enum SeaNameText {
    static func message(_ error: Error) -> String {
        if let error = error as? SeaURL.ParseError {
            switch error {
            case .externalTLD: return String(localized: "Web addresses (.com, etc.) aren't EastSea names. Open them with https://.")
            case .reservedName: return String(localized: "This name is reserved for a wallet action.")
            case .legacyNameUnsupported: return String(localized: "Legacy .aeth names are only available on testnet 7780.")
            default: return String(localized: "That is not a valid .sea name.")
            }
        }
        switch error as? SeaNameResolver.Failure {
        case .registryUnavailable: return String(localized: "Name registry is not available on this network.")
        case .unregistered: return String(localized: "This name is not registered.")
        case .expired: return String(localized: "This name has expired.")
        case .noApp: return String(localized: "No app is linked to this name.")
        case .unlistedApp: return String(localized: "This app is no longer listed.")
        default: return String(localized: "The name registry could not be read.")
        }
    }
}
