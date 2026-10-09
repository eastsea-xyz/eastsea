import Foundation
import Darwin

var fixtureProcesses: [Process] = []
func stopFixtures() {
    for process in fixtureProcesses {
        if process.isRunning { process.terminate() }
        process.waitUntilExit()
    }
}
func check(_ c: Bool, _ message: String) {
    if !c { stopFixtures(); print("FAIL", message); exit(1) }
    print("ok  ", message)
}
func ready(_ file: URL, process: Process, label: String) async throws {
    let deadline = ProcessInfo.processInfo.systemUptime + 30
    while process.isRunning {
        if let text = try? String(contentsOf: file, encoding: .utf8),
           text == "\(process.processIdentifier) \(label)\n" { return }
        guard ProcessInfo.processInfo.systemUptime < deadline else { break }
        try await Task.sleep(nanoseconds: 10_000_000)
    }
    let status = process.isRunning ? "still running" : "exited \(process.terminationStatus)"
    throw NSError(domain: "R11Fixture", code: 1, userInfo: [NSLocalizedDescriptionKey:
        "helper readiness missing within 30s: pid=\(process.processIdentifier) \(status) file=\(file.path)"])
}
guard CommandLine.arguments.count == 4 else { fatalError("usage: fixture OLD NEW TASK-DIR") }
let old = URL(fileURLWithPath: CommandLine.arguments[1])
let new = URL(fileURLWithPath: CommandLine.arguments[2])
let root = URL(fileURLWithPath: CommandLine.arguments[3], isDirectory: true)
let live = root.appendingPathComponent("R11-live")
let staged = root.appendingPathComponent("R11-staged")
do {
defer { stopFixtures() }
try FileManager.default.copyItem(at: old, to: live)
let a = Process()
a.executableURL = live
let oldReady = root.appendingPathComponent("R11-old-ready")
a.arguments = [oldReady.path]
try a.run()
fixtureProcesses.append(a)
try await ready(oldReady, process: a, label: "old")
check(NodeReleaseIdentity.matches(pid: a.processIdentifier, expected: old), "R11 original signed helper matches its expected CDHash")
try FileManager.default.copyItem(at: new, to: staged)
check(rename(staged.path, live.path) == 0, "R11 replacement fixture atomically replaces the executable path")
check(a.isRunning, "R11 the old helper survives path replacement")
check(!NodeReleaseIdentity.matches(pid: a.processIdentifier, expected: live), "R11 surviving old image is rejected against the replacement CDHash")
let b = Process()
b.executableURL = live
let newReady = root.appendingPathComponent("R11-new-ready")
b.arguments = [newReady.path]
try b.run()
fixtureProcesses.append(b)
try await ready(newReady, process: b, label: "new")
check(NodeReleaseIdentity.matches(pid: b.processIdentifier, expected: live), "R11 newly launched replacement matches the expected CDHash")
check(!NodeReleaseIdentity.matches(pid: -1, expected: live), "R11 invalid process identity fails closed")
let damaged = root.appendingPathComponent("R11-damaged")
try FileManager.default.copyItem(at: new, to: damaged)
let h = try FileHandle(forWritingTo: damaged)
try h.write(contentsOf: Data([0, 0, 0, 0]))
try h.close()
check(!NodeReleaseIdentity.matches(pid: b.processIdentifier, expected: damaged), "R11 invalid expected signature fails closed")
print("all passed")
} catch {
    stopFixtures()
    print("FAIL R11 fixture error:", error)
    exit(1)
}
