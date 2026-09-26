import Foundation

/// Register the MCP server with each agent tool through that tool's own CLI
/// (no hand-editing of their config files), and install the skill file where
/// the tool reads skills. Without --apply it only prints what it would run.
enum Setup {
    struct Harness {
        let name: String
        let binary: String
        let command: (String) -> [String]
        /// Skill directory (AgentSkills SKILL.md layout), if the tool reads one.
        let skillDir: String?
    }

    static let harnesses: [Harness] = [
        Harness(name: "claude", binary: "claude", command: { ["claude", "mcp", "add", "--scope", "user", "aether", "--", $0, "mcp"] }, skillDir: "~/.claude/skills"),
        Harness(name: "codex", binary: "codex", command: { ["codex", "mcp", "add", "aether", "--", $0, "mcp"] }, skillDir: "~/.codex/skills"),
        Harness(name: "antigravity", binary: "agy", command: { ["agy", "mcp", "add", "aether", $0, "mcp"] }, skillDir: nil),
        Harness(name: "openclaw", binary: "openclaw", command: { ["openclaw", "mcp", "add", "aether", "--command", $0, "--arg", "mcp"] }, skillDir: "~/.openclaw/skills"),
        Harness(name: "hermes", binary: "hermes", command: { ["hermes", "mcp", "add", "aether", "--command", $0, "--args", "mcp"] }, skillDir: nil),
    ]

    static func run(_ args: [String]) {
        let apply = args.contains("--apply")
        let target = args.first { !$0.hasPrefix("--") } ?? "all"
        let bin = selfPath()
        let chosen = target == "all" ? harnesses : harnesses.filter { $0.name == target }
        if chosen.isEmpty {
            print("Unknown harness \(target). Any other MCP client (Muse, Cursor, ...): add a stdio server named \"aether\":")
            print(JSON.string(["mcpServers": ["aether": ["command": bin, "args": ["mcp"]]]]))
            return
        }
        for h in chosen {
            let cmd = h.command(bin)
            guard let tool = which(h.binary) else {
                print("· \(h.name): not installed (\(h.binary) not on PATH), skipped")
                continue
            }
            if !apply {
                print("· \(h.name): \(cmd.map(quote).joined(separator: " "))")
                if let d = h.skillDir { print("    skill → \(d)/aether-wallet/SKILL.md") }
                continue
            }
            let status = exec(tool, Array(cmd.dropFirst()))
            print("· \(h.name): \(status == 0 ? "registered" : "failed (exit \(status)); run it yourself: \(cmd.map(quote).joined(separator: " "))")")
            if let d = h.skillDir { installSkill(into: d) }
        }
        if !apply { print("\nRun `aether-agent setup \(target) --apply` to do this. Other MCP clients: `aether-agent setup other`.") }
    }

    private static func installSkill(into dir: String) {
        let url = URL(fileURLWithPath: NSString(string: dir).expandingTildeInPath).appendingPathComponent("aether-wallet/SKILL.md")
        do {
            try FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
            try Skill.markdown.write(to: url, atomically: true, encoding: .utf8)
            print("    skill → \(url.path)")
        } catch {
            print("    skill not written: \(error)")
        }
    }

    private static func selfPath() -> String {
        let p = CommandLine.arguments[0]
        if p.contains("/") { return URL(fileURLWithPath: p).standardizedFileURL.resolvingSymlinksInPath().path }
        return which(p) ?? p
    }

    private static func which(_ name: String) -> String? {
        let path = (ProcessInfo.processInfo.environment["PATH"] ?? "") + ":" + NSString(string: "~/.local/bin").expandingTildeInPath + ":/opt/homebrew/bin"
        return path.split(separator: ":").map { "\($0)/\(name)" }.first { FileManager.default.isExecutableFile(atPath: $0) }
    }

    private static func exec(_ tool: String, _ args: [String]) -> Int32 {
        let p = Process()
        p.executableURL = URL(fileURLWithPath: tool)
        p.arguments = args
        do {
            try p.run()
            p.waitUntilExit()
            return p.terminationStatus
        } catch {
            return -1
        }
    }

    private static func quote(_ s: String) -> String {
        s.contains(" ") ? "'\(s)'" : s
    }
}
