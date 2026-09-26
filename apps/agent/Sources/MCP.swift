import Foundation

/// Minimal MCP server over stdio (JSON-RPC 2.0, one message per line): the
/// transport every agent harness speaks (Claude Code, Codex, Antigravity,
/// OpenClaw, Hermes, ...). Only JSON-RPC goes to stdout; diagnostics go to stderr.
enum MCPServer {
    static let protocolVersion = "2025-06-18"

    static let instructions = """
    Aether wallet for this agent. The agent has its own account whose key lives in this Mac's Secure Enclave \
    (it cannot be exported). Balances are verified on this Mac against the validators' threshold signature. \
    Payments are limited by the owner's Touch ID-signed policy (per payment and per 24 h); if a payment is refused, \
    tell the human the limit and ask them to change it with `aether-agent policy set` — do not retry around it. \
    Use aether_send with dry_run first when unsure. Amounts are decimal AETH strings.
    """

    static func run() {
        while let line = readLine(strippingNewline: true) {
            guard !line.trimmingCharacters(in: .whitespaces).isEmpty else { continue }
            guard let data = line.data(using: .utf8), let msg = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else {
                reply(["jsonrpc": "2.0", "id": NSNull(), "error": ["code": -32700, "message": "parse error"]])
                continue
            }
            if let response = handle(msg) { reply(response) }
        }
    }

    static func handle(_ msg: [String: Any]) -> [String: Any]? {
        let method = msg["method"] as? String ?? ""
        guard let id = msg["id"] else { return nil }  // notification (e.g. notifications/initialized)
        let params = msg["params"] as? [String: Any] ?? [:]
        func ok(_ result: [String: Any]) -> [String: Any] { ["jsonrpc": "2.0", "id": id, "result": result] }
        switch method {
        case "initialize":
            return ok(["protocolVersion": params["protocolVersion"] as? String ?? protocolVersion,
                       "capabilities": ["tools": ["listChanged": false]],
                       "serverInfo": ["name": "aether", "title": "Aether Wallet", "version": Version.string],
                       "instructions": instructions])
        case "ping":
            return ok([:])
        case "tools/list":
            return ok(["tools": Tools.all.map { t -> [String: Any] in
                ["name": t.name, "description": t.description, "inputSchema": t.schema,
                 "annotations": ["readOnlyHint": t.readOnly, "destructiveHint": !t.readOnly, "openWorldHint": true]]
            }])
        case "tools/call":
            let name = params["name"] as? String ?? ""
            guard let tool = Tools.all.first(where: { $0.name == name }) else {
                return ["jsonrpc": "2.0", "id": id, "error": ["code": -32602, "message": "unknown tool \(name)"]]
            }
            do {
                let out = try tool.run(params["arguments"] as? [String: Any] ?? [:])
                return ok(["content": [["type": "text", "text": JSON.string(out)]], "structuredContent": out, "isError": false])
            } catch {
                return ok(["content": [["type": "text", "text": "\(error)"]], "isError": true])
            }
        default:
            return ["jsonrpc": "2.0", "id": id, "error": ["code": -32601, "message": "method not found: \(method)"]]
        }
    }

    private static func reply(_ obj: [String: Any]) {
        FileHandle.standardOutput.write(Data((JSON.string(obj, pretty: false) + "\n").utf8))
    }
}

enum JSON {
    static func string(_ obj: Any, pretty: Bool = true) -> String {
        var opts: JSONSerialization.WritingOptions = [.sortedKeys, .withoutEscapingSlashes]
        if pretty { opts.insert(.prettyPrinted) }
        guard let d = try? JSONSerialization.data(withJSONObject: obj, options: opts) else { return "{}" }
        return String(decoding: d, as: UTF8.self)
    }
}

enum Version {
    static let string = "0.1.0"
}
