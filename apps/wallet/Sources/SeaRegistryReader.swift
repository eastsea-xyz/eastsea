import Foundation

/// The node answers eth_call at its latest finalized snapshot (block tags
/// are not supported yet). Check the snapshot around the whole lookup and
/// retry a bounded number of times instead of combining different blocks.
enum SeaRegistryReader {
    private struct Snapshot: Equatable {
        let chain: UInt64
        let height: UInt64
        let root: String
        let hash: String
        let timestamp: UInt64

        static func read(port: UInt16) async throws -> Snapshot {
            guard let s = await LocalRPC.call(port: port, method: "aether_status", params: []) as? [String: Any],
                  let chain = s["chain_id"] as? NSNumber, let height = s["height"] as? NSNumber,
                  let timestamp = s["timestamp_ms"] as? NSNumber,
                  let root = s["state_root"] as? String, let hash = s["hash"] as? String else {
                throw SeaNameResolver.Failure.badAnswer
            }
            return Snapshot(chain: chain.uint64Value, height: height.uint64Value, root: root, hash: hash,
                            timestamp: timestamp.uint64Value / 1000)
        }
    }

    static func resolve(_ link: SeaURL.NameLink, chainID: UInt64, port: UInt16,
                        sources: SeaRegistrySources?) async throws -> SeaNameResolver.Resolution {
        guard sources != nil else { throw SeaNameResolver.Failure.registryUnavailable }
        for _ in 0..<3 {
            try Task.checkCancellation()
            let before = try await Snapshot.read(port: port)
            guard before.chain == chainID else { throw SeaNameResolver.Failure.unstable }
            do {
                let result = try await SeaNameResolver.resolve(link, now: before.timestamp, sources: sources, chainID: chainID,
                    code: { to in
                        try Task.checkCancellation()
                        guard let code = await LocalRPC.call(port: port, method: "eth_getCode", params: [to, "latest"]) as? String else {
                            throw SeaNameResolver.Failure.badAnswer
                        }
                        return code
                    }, read: { to, data in
                        try Task.checkCancellation()
                        guard let value = await LocalRPC.call(port: port, method: "eth_call", params: [["to": to, "data": data], "latest"]) as? String else {
                            throw SeaNameResolver.Failure.badAnswer
                        }
                        return value
                    })
                if before == (try await Snapshot.read(port: port)) { return result }
            } catch {
                if before == (try await Snapshot.read(port: port)) { throw error }
            }
        }
        throw SeaNameResolver.Failure.unstable
    }
}
