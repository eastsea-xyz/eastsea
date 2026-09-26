import Foundation

// aether-agent: the Aether wallet for AI agents on a Mac.
//   aether-agent mcp                 MCP server on stdio (for agent harnesses)
//   aether-agent <tool> [--k v ...]  the same tools as a JSON CLI
//   aether-agent init | policy ...   owner setup (Touch ID)
//   aether-agent setup <harness>     register with Claude Code, Codex, Antigravity, OpenClaw, Hermes

let usage = """
aether-agent \(Version.string) — Aether wallet for AI agents (key in this Mac's Secure Enclave)

Owner (asks for Touch ID):
  init                                   create the agent's keys and default limits (1 AETH/payment, 10 AETH/day)
  policy show
  policy set [--per-tx X] [--per-day Y] [--allow 0x..,0x..|anyone]
  policy reset-ledger                    accept the current on-chain state as the spending log

Agents (JSON out):
  mcp                                    run as an MCP server on stdio
  status | wallet | history [--limit N]
  balance --address 0x..
  send --to 0x.. --amount 0.5 [--dry-run]
  pay-many --to 0x..,0x.. --amount 0.1 [--dry-run]     (each recipient gets --amount)
  receipt --hash 0x..
  get-test-tokens

Register with agent tools:
  setup [claude|codex|antigravity|openclaw|hermes|all] [--apply]
"""

func flags(_ args: ArraySlice<String>) -> [String: String] {
    var out: [String: String] = [:]
    var i = args.startIndex
    while i < args.endIndex {
        let a = args[i]
        if a.hasPrefix("--") {
            let key = String(a.dropFirst(2)).replacingOccurrences(of: "-", with: "_")
            if i + 1 < args.endIndex, !args[i + 1].hasPrefix("--") {
                out[key] = args[i + 1]
                i += 2
                continue
            }
            out[key] = "true"
        }
        i += 1
    }
    return out
}

func printJSON(_ obj: [String: Any]) { print(JSON.string(obj)) }

func fail(_ error: Error) -> Never {
    printJSON(["error": "\(error)"])
    exit(1)
}

func runTool(_ name: String, _ f: [String: String]) {
    var args: [String: Any] = f
    if let d = f["dry_run"] { args["dry_run"] = d == "true" }
    if name == "aether_pay_many" {
        let tos = (f["to"] ?? "").split(separator: ",").map { String($0).trimmingCharacters(in: .whitespaces) }
        args = ["payments": tos.map { ["to": $0, "amount": f["amount"] ?? ""] }, "dry_run": f["dry_run"] == "true"]
    }
    guard let tool = Tools.all.first(where: { $0.name == name }) else { fail(AgentError.input("unknown command")) }
    do { printJSON(try tool.run(args)) } catch { fail(error) }
}

let argv = CommandLine.arguments
let cmd = argv.count > 1 ? argv[1] : "help"
let rest = argv.dropFirst(2)

switch cmd {
case "mcp":
    MCPServer.run()
case "status", "wallet", "balance", "send", "pay-many", "receipt", "history", "get-test-tokens":
    runTool("aether_" + cmd.replacingOccurrences(of: "-", with: "_"), flags(rest))
case "init":
    do { printJSON(try Owner.initialize()) } catch { fail(error) }
case "policy":
    do { printJSON(try Owner.policy(Array(rest))) } catch { fail(error) }
case "setup":
    Setup.run(Array(rest))
case "version", "--version":
    print(Version.string)
default:
    print(usage)
}
