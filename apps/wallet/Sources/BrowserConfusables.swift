import Foundation

/// UTS #39 revision 28, Unicode 15.1.0, section 4 skeleton:
/// NFD → remove Default_Ignorable_Code_Point → official prototypes → NFD.
/// Both complete tables are bundled, with their source digests and license.
/// This versioned algorithm does not claim Unicode 18 bidiSkeleton coverage.
/// Skeletons are comparison keys, never display names, origins or stored data.
struct BrowserConfusables {
    static let unicodeVersion = "15.1.0"

    struct Warning: Equatable {
        let protectedName: String
        let protectedURL: URL
        let protectedHost: String
        let candidateHost: String
    }

    enum Failure: Error { case invalidData, missingData }
    private let mappings: [UInt32: String]
    private let ignorables: [ClosedRange<UInt32>]

    init(confusables: String, defaultIgnorables: String) throws {
        var prototypes: [UInt32: String] = [:]
        for line in confusables.components(separatedBy: .newlines) {
            let content = line.components(separatedBy: "#")[0].trimmingCharacters(in: .whitespaces)
            if content.isEmpty { continue }
            let fields = content.components(separatedBy: ";").map { $0.trimmingCharacters(in: .whitespaces) }
            guard fields.count == 3, let source = UInt32(fields[0], radix: 16), UnicodeScalar(source) != nil,
                  prototypes[source] == nil else { throw Failure.invalidData }
            let values = fields[1].split(separator: " ").compactMap { UInt32($0, radix: 16).flatMap(UnicodeScalar.init) }
            guard !values.isEmpty, values.count == fields[1].split(separator: " ").count else { throw Failure.invalidData }
            prototypes[source] = String(String.UnicodeScalarView(values))
        }
        var ranges: [ClosedRange<UInt32>] = []
        for line in defaultIgnorables.components(separatedBy: .newlines) {
            let content = line.components(separatedBy: "#")[0].trimmingCharacters(in: .whitespaces)
            if content.isEmpty { continue }
            let fields = content.components(separatedBy: "..")
            guard (1...2).contains(fields.count), let first = UInt32(fields[0], radix: 16),
                  let last = UInt32(fields.last!, radix: 16), first <= last, last <= 0x10FFFF else { throw Failure.invalidData }
            ranges.append(first...last)
        }
        guard prototypes.count == 6311, ranges.count == 27 else { throw Failure.invalidData }
        mappings = prototypes
        ignorables = ranges
    }

    init(bundle: Bundle = .main) throws {
        guard let mappingURL = bundle.url(forResource: "BrowserConfusables-15.1", withExtension: "txt"),
              let ignorableURL = bundle.url(forResource: "BrowserDefaultIgnorables-15.1", withExtension: "txt") else { throw Failure.missingData }
        try self.init(confusables: String(contentsOf: mappingURL, encoding: .utf8),
                      defaultIgnorables: String(contentsOf: ignorableURL, encoding: .utf8))
    }

    func skeleton(_ text: String) -> String {
        var mapped = ""
        for scalar in text.decomposedStringWithCanonicalMapping.unicodeScalars {
            if ignorables.contains(where: { $0.contains(scalar.value) }) { continue }
            mapped += mappings[scalar.value] ?? String(scalar)
        }
        return mapped.decomposedStringWithCanonicalMapping
    }

    func warning(for url: URL, bookmarks: [BrowserBookmark], apps: [BrowserToolboxApp]) -> Warning? {
        guard let candidate = BrowserCanonicalOrigin.host(for: url),
              !candidate.hasPrefix("[") else { return nil }
        let unicodeCandidate = BrowserPunycode.unicodeHost(candidate)
        let candidateSkeleton = skeleton(unicodeCandidate)
        let known = bookmarks.map { ($0.title, $0.url) } + apps.map { ($0.title, $0.url) }
        for (title, protectedURL) in known {
            guard let protectedHost = BrowserCanonicalOrigin.host(for: protectedURL),
                  !protectedHost.hasPrefix("[") else { continue }
            // A legitimate exact host or its subdomain must never be warned
            // merely because one of its ordinary letters has a prototype.
            if candidate == protectedHost || candidate.hasSuffix("." + protectedHost) { continue }
            let protected = BrowserPunycode.unicodeHost(protectedHost)
            let protectedSkeleton = skeleton(protected)
            let base = protected.hasPrefix("www.") ? String(protected.dropFirst(4)) : protected
            let baseSkeleton = skeleton(base)
            if candidateSkeleton == protectedSkeleton || candidateSkeleton.hasSuffix("." + protectedSkeleton) ||
                candidateSkeleton == baseSkeleton || candidateSkeleton.hasSuffix("." + baseSkeleton) {
                return Warning(protectedName: title.isEmpty ? protectedHost : title, protectedURL: protectedURL,
                               protectedHost: protectedHost, candidateHost: candidate)
            }
        }
        return nil
    }

    private static let bundled = try? BrowserConfusables()

    static func warning(for url: URL, bookmarks: [BrowserBookmark], apps: [BrowserToolboxApp]) -> Warning? {
        bundled?.warning(for: url, bookmarks: bookmarks, apps: apps)
    }
}
