import Foundation

/// One preference for both the app's node and its unattended follower. The
/// node reads this file before serving requests, so no restart is necessary.
struct PublicReadSettings: Codable, Equatable {
    static let fileName = "public-read.json"
    static let bytesPerMiB: UInt64 = 1_048_576
    static let defaultValue = PublicReadSettings(enabled: true, dailyBytes: 256 * bytesPerMiB)

    var enabled: Bool
    var dailyBytes: UInt64

    private enum CodingKeys: String, CodingKey {
        case enabled
        case dailyBytes = "daily_bytes"
    }

    struct ReadResult: Equatable {
        let settings: PublicReadSettings
        let needsRepair: Bool
    }

    /// Only absence uses the default. A corrupt or unreadable preference
    /// pauses sharing until the owner successfully saves a valid setting.
    /// Reading never creates a directory or writes a default preference.
    static func read(in directory: URL) -> ReadResult {
        do {
            let data = try Data(contentsOf: directory.appendingPathComponent(fileName))
            // Match the node's deny_unknown_fields policy: an edited or
            // newer schema must not display "on" while the node fails closed.
            guard let fields = try JSONSerialization.jsonObject(with: data) as? [String: Any],
                  Set(fields.keys) == ["enabled", "daily_bytes"] else {
                throw DecodingError.dataCorrupted(.init(codingPath: [],
                    debugDescription: "public read settings require only enabled and daily_bytes"))
            }
            let settings = try JSONDecoder().decode(Self.self, from: data)
            return ReadResult(settings: settings, needsRepair: false)
        } catch let error as CocoaError where error.code == .fileReadNoSuchFile {
            return ReadResult(settings: defaultValue, needsRepair: false)
        } catch {
            return ReadResult(settings: PublicReadSettings(enabled: false, dailyBytes: defaultValue.dailyBytes),
                              needsRepair: true)
        }
    }

    /// Atomic replacement ensures the running node sees the complete old or
    /// new policy, never a partially written toggle or bandwidth cap.
    func write(in directory: URL) throws {
        let data = try JSONEncoder().encode(self)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        try data.write(to: directory.appendingPathComponent(Self.fileName), options: .atomic)
    }
}
