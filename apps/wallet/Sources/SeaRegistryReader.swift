import Foundation

/// The node answers eth_call at its latest finalized snapshot (block tags
/// are not supported yet). Check the snapshot around the whole lookup and
/// retry a bounded number of times instead of combining different blocks.
enum SeaRegistryReader {
    typealias RPC = (_ method: String, _ params: [Any]) async throws -> Any

    // Foundation transport works in both wallet targets. LocalRPC belongs to
    // the macOS node lifecycle and is unavailable in the iOS target.
    enum RPCFailure: Error { case unavailable, rpc(Int) }

    /// Query bytes must never follow a redirect from the local node to a site.
    private final class LocalReadDelegate: NSObject, URLSessionTaskDelegate, @unchecked Sendable {
        func urlSession(_ session: URLSession, task: URLSessionTask,
                        willPerformHTTPRedirection response: HTTPURLResponse, newRequest request: URLRequest,
                        completionHandler: @escaping (URLRequest?) -> Void) { completionHandler(nil) }
    }

    static func call(port: UInt16, method: String, params: [Any]) async throws -> Any {
        guard let url = URL(string: "http://127.0.0.1:\(port)/"),
              let body = try? JSONSerialization.data(withJSONObject:
                ["jsonrpc": "2.0", "id": 1, "method": method, "params": params]) else { throw RPCFailure.unavailable }
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
        let session = URLSession(configuration: configuration, delegate: LocalReadDelegate(), delegateQueue: nil)
        defer { session.invalidateAndCancel() }
        let (data, response) = try await session.data(for: request)
        guard
              let http = response as? HTTPURLResponse, http.statusCode == 200,
              http.url?.scheme == "http", http.url?.host == "127.0.0.1", http.url?.port == Int(port),
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else { throw RPCFailure.unavailable }
        if let error = object["error"] as? [String: Any] {
            throw RPCFailure.rpc(error["code"] as? Int ?? -32603)
        }
        guard let result = object["result"] else { throw RPCFailure.unavailable }
        return result
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

    static func lookup(_ link: SeaURL.NameLink, chainID: UInt64, port: UInt16,
                       sources: SeaRegistrySources?, readRPC: RPC? = nil) async throws -> SeaNameResolver.NameRecord {
        guard sources != nil else { throw SeaNameResolver.Failure.registryUnavailable }
        let rpc: RPC = readRPC ?? { method, params in
            try await call(port: port, method: method, params: params)
        }
        for _ in 0..<3 {
            try Task.checkCancellation()
            let before = try await Snapshot.read(rpc: rpc)
            try Task.checkCancellation()
            guard before.chain == chainID else { throw SeaNameResolver.Failure.unstable }
            do {
                let result = try await SeaNameResolver.lookup(link, now: before.timestamp, sources: sources, chainID: chainID,
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
                try Task.checkCancellation()
                let after = try await Snapshot.read(rpc: rpc)
                try Task.checkCancellation()
                if before == after { return result }
            } catch {
                try Task.checkCancellation()
                let after = try await Snapshot.read(rpc: rpc)
                try Task.checkCancellation()
                if before == after { throw error }
            }
        }
        throw SeaNameResolver.Failure.unstable
    }

    static func resolveApp(appID: String, chainID: UInt64, port: UInt16,
                           sources: SeaRegistrySources?, readRPC: RPC? = nil) async throws -> SeaAppRecord {
        guard sources != nil else { throw SeaNameResolver.Failure.registryUnavailable }
        let rpc: RPC = readRPC ?? { method, params in
            try await call(port: port, method: method, params: params)
        }
        for _ in 0..<3 {
            try Task.checkCancellation()
            let before = try await Snapshot.read(rpc: rpc)
            try Task.checkCancellation()
            guard before.chain == chainID else { throw SeaNameResolver.Failure.unstable }
            do {
                let result = try await SeaNameResolver.resolveApp(appID: appID, sources: sources,
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
                try Task.checkCancellation()
                let after = try await Snapshot.read(rpc: rpc)
                try Task.checkCancellation()
                if before == after { return result }
            } catch {
                try Task.checkCancellation()
                let after = try await Snapshot.read(rpc: rpc)
                try Task.checkCancellation()
                if before == after { throw error }
            }
        }
        throw SeaNameResolver.Failure.unstable
    }

    static func resolve(_ link: SeaURL.NameLink, chainID: UInt64, port: UInt16,
                        sources: SeaRegistrySources?, readRPC: RPC? = nil) async throws -> SeaNameResolver.Resolution {
        guard sources != nil else { throw SeaNameResolver.Failure.registryUnavailable }
        let rpc: RPC = readRPC ?? { method, params in
            try await call(port: port, method: method, params: params)
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
