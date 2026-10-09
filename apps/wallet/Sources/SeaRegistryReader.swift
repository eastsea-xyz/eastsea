import Foundation

/// The node answers eth_call at its latest finalized snapshot (block tags
/// are not supported yet). Check the snapshot around the whole lookup and
/// retry a bounded number of times instead of combining different blocks.
enum SeaRegistryReader {
    typealias RPC = (_ method: String, _ params: [Any]) async throws -> Any

    // Foundation transport works in both wallet targets. LocalRPC belongs to
    // the macOS node lifecycle and is unavailable in the iOS target.
    private static func call(port: UInt16, method: String, params: [Any]) async -> Any? {
        guard let url = URL(string: "http://127.0.0.1:\(port)/"),
              let body = try? JSONSerialization.data(withJSONObject:
                ["jsonrpc": "2.0", "id": 1, "method": method, "params": params]) else { return nil }
        var request = URLRequest(url: url, timeoutInterval: 5)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.setValue("close", forHTTPHeaderField: "Connection")
        request.httpBody = body
        let configuration = URLSessionConfiguration.ephemeral
        configuration.urlCache = nil
        configuration.connectionProxyDictionary = [:]
        configuration.urlCredentialStorage = nil
        configuration.httpCookieStorage = nil
        configuration.httpShouldSetCookies = false
        configuration.requestCachePolicy = .reloadIgnoringLocalCacheData
        let session = URLSession(configuration: configuration)
        defer { session.invalidateAndCancel() }
        guard let (data, response) = try? await session.data(for: request),
              let http = response as? HTTPURLResponse, http.statusCode == 200,
              http.url?.scheme == "http", http.url?.host == "127.0.0.1", http.url?.port == Int(port),
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              object["error"] == nil else { return nil }
        return object["result"]
    }
    private struct Snapshot: Equatable {
        let chain: UInt64
        let height: UInt64
        let root: String
        let hash: String
        let timestamp: UInt64

        static func read(rpc: RPC) async throws -> Snapshot {
            guard let s = try await rpc("aether_status", []) as? [String: Any],
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
                        sources: SeaRegistrySources?, readRPC: RPC? = nil) async throws -> SeaNameResolver.Resolution {
        guard sources != nil else { throw SeaNameResolver.Failure.registryUnavailable }
        let rpc: RPC = readRPC ?? { method, params in
            guard let value = await call(port: port, method: method, params: params) else {
                throw SeaNameResolver.Failure.badAnswer
            }
            return value
        }
        for _ in 0..<3 {
            try Task.checkCancellation()
            let before = try await Snapshot.read(rpc: rpc)
            guard before.chain == chainID else { throw SeaNameResolver.Failure.unstable }
            do {
                let result = try await SeaNameResolver.resolve(link, now: before.timestamp, sources: sources, chainID: chainID,
                    code: { to in
                        try Task.checkCancellation()
                        guard let code = try await rpc("eth_getCode", [to, "latest"]) as? String else {
                            throw SeaNameResolver.Failure.badAnswer
                        }
                        return code
                    }, read: { to, data in
                        try Task.checkCancellation()
                        guard let value = try await rpc("eth_call", [["to": to, "data": data], "latest"]) as? String else {
                            throw SeaNameResolver.Failure.badAnswer
                        }
                        return value
                    })
                if before == (try await Snapshot.read(rpc: rpc)) { return result }
            } catch {
                if before == (try await Snapshot.read(rpc: rpc)) { throw error }
            }
        }
        throw SeaNameResolver.Failure.unstable
    }
}
