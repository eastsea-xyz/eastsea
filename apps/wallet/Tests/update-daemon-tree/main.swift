import Foundation
import Darwin

var owned: [Process] = []
func cleanup() {
    for process in owned.reversed() {
        if process.isRunning { process.terminate() }
        process.waitUntilExit()
    }
}
func check(_ value: Bool, _ message: String) {
    if !value { cleanup(); print("FAIL", message); exit(1) }
    print("ok  ", message)
}
func spawn(_ binary: URL, _ args: [String]) throws -> Process {
    let process = Process()
    process.executableURL = binary
    process.arguments = args
    try process.run()
    owned.append(process)
    return process
}
func ready(_ file: URL, process: Process) throws -> (Int32, UInt16) {
    let until = Date().addingTimeInterval(5)
    while Date() < until, process.isRunning {
        if let text = try? String(contentsOf: file, encoding: .utf8) {
            let fields = text.split(separator: " ")
            if fields.count == 2, let pid = Int32(fields[0]), let port = UInt16(fields[1].trimmingCharacters(in: .whitespacesAndNewlines)) {
                return (pid, port)
            }
        }
        Thread.sleep(forTimeInterval: 0.01)
    }
    throw NSError(domain: "R11Fixture", code: 1, userInfo: [NSLocalizedDescriptionKey: "listener did not become ready"])
}
func reply(port: UInt16) -> String? {
    let fd = Darwin.socket(AF_INET, SOCK_STREAM, 0)
    guard fd >= 0 else { return nil }
    defer { close(fd) }
    var timeout = timeval(tv_sec: 2, tv_usec: 0)
    _ = withUnsafePointer(to: &timeout) { setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, $0, socklen_t(MemoryLayout<timeval>.stride)) }
    var address = sockaddr_in()
    address.sin_len = UInt8(MemoryLayout<sockaddr_in>.stride)
    address.sin_family = sa_family_t(AF_INET)
    address.sin_port = port.bigEndian
    address.sin_addr.s_addr = UInt32(0x7f00_0001).bigEndian
    let connected = withUnsafePointer(to: &address) {
        $0.withMemoryRebound(to: sockaddr.self, capacity: 1) { Darwin.connect(fd, $0, socklen_t(MemoryLayout<sockaddr_in>.stride)) }
    }
    guard connected == 0 else { return nil }
    var buffer = [UInt8](repeating: 0, count: 32)
    let count = buffer.withUnsafeMutableBytes { Darwin.read(fd, $0.baseAddress, $0.count) }
    guard count > 0 else { return nil }
    return String(decoding: buffer.prefix(count), as: UTF8.self).trimmingCharacters(in: .whitespacesAndNewlines)
}
func accepted(rootPID: Int32, port: UInt16, expected: URL) -> Bool {
    NodeReleaseIdentity.matchesNode(rootPID: rootPID, port: port, expected: expected)
}
guard CommandLine.arguments.count == 4 else { fatalError("usage: fixture A B TASK-DIR") }
let a = URL(fileURLWithPath: CommandLine.arguments[1])
let b = URL(fileURLWithPath: CommandLine.arguments[2])
let directory = URL(fileURLWithPath: CommandLine.arguments[3], isDirectory: true)
do {
    defer { cleanup() }
    let badReady = directory.appendingPathComponent("R11-child-A-ready")
    let mixed = try spawn(b, ["parent", a.path, badReady.path])
    let (badPID, badPort) = try ready(badReady, process: mixed)
    check(badPID != mixed.processIdentifier, "R11 parent and actual listener have distinct PIDs")
    check(reply(port: badPort) == "A", "R11 status reply actually comes from old listener A")
    check(NodeReleaseIdentity.matches(pid: mixed.processIdentifier, expected: b), "R11 supervisor B alone is correctly signed")
    check(!accepted(rootPID: mixed.processIdentifier, port: badPort, expected: b),
          "R11 signed supervisor B with old listener A cannot authenticate RPC")
    let goodReady = directory.appendingPathComponent("R11-child-B-ready")
    let good = try spawn(b, ["parent", b.path, goodReady.path])
    let (_, goodPort) = try ready(goodReady, process: good)
    check(reply(port: goodPort) == "B", "R11 current listener B returns its own reply")
    var currentListenerAccepted = false
    let settleUntil = Date().addingTimeInterval(2)
    while !currentListenerAccepted, Date() < settleUntil {
        currentListenerAccepted = accepted(rootPID: good.processIdentifier, port: goodPort, expected: b)
        if !currentListenerAccepted { Thread.sleep(forTimeInterval: 0.01) }
    }
    check(currentListenerAccepted,
          "R11 supervisor B with signed listener B authenticates RPC")
    let idle = try spawn(b, ["idle"])
    let unrelatedReady = directory.appendingPathComponent("R11-unrelated-A-ready")
    let unrelated = try spawn(a, ["listen", unrelatedReady.path])
    let (_, unrelatedPort) = try ready(unrelatedReady, process: unrelated)
    check(reply(port: unrelatedPort) == "A", "R11 unrelated listener is live")
    check(!accepted(rootPID: idle.processIdentifier, port: unrelatedPort, expected: b),
          "R11 unrelated listener outside supervisor descendants is excluded")
    check(!accepted(rootPID: good.processIdentifier, port: 0, expected: b), "R11 missing endpoint identity fails closed")
// The same signed images are insufficient if the listener changes
// while a response is in flight. Controlled helpers alone are restarted.
let stableReady = directory.appendingPathComponent("R11-stable-ready")
let control = directory.appendingPathComponent("R11-stable-control")
let stable = try spawn(b, ["parent", b.path, stableReady.path, b.path, control.path])
let (oldOwner, stablePort) = try ready(stableReady, process: stable)
let unchanged = await NodeReleaseIdentity.readVerified(rootPID: stable.processIdentifier, port: stablePort, expected: b,
    operation: { reply(port: stablePort) })
check(unchanged?.value == "B", "R11 unchanged listener proof brackets a fresh response")
check(unchanged.map { NodeReleaseIdentity.matches(binding: $0.binding, port: stablePort, expected: b) } == true,
      "R11 returned binding still authenticates the same live listener")
let changed = await NodeReleaseIdentity.readVerified(rootPID: stable.processIdentifier, port: stablePort, expected: b,
    operation: {
        let priorReply = reply(port: stablePort)
        try? Data("restart".utf8).write(to: control, options: .atomic)
        let until = Date().addingTimeInterval(5)
        while Date() < until {
            if let (newOwner, newPort) = try? ready(stableReady, process: stable),
               newOwner != oldOwner, newPort == stablePort { return priorReply }
            Thread.sleep(forTimeInterval: 0.01)
        }
        return nil
    })
check(changed == nil, "R11 response crossing a same-release listener replacement is refused")
check(unchanged.map { NodeReleaseIdentity.matches(binding: $0.binding, port: stablePort, expected: b) } == false,
      "R11 returned binding cannot authorize shutdown of a replacement listener")
check(NodeReleaseIdentity.hasWriterLease(status: ["writer_lease_protocol": 1]), "R11 live inherited lease capability is accepted")
let invalidCapabilities: [Any] = [0, 2, true, "1", 1.0, NSNull()]
for invalid in invalidCapabilities {
    check(!NodeReleaseIdentity.hasWriterLease(status: ["writer_lease_protocol": invalid]), "R11 invalid or unsupported writer lease is unknown")
}
check(!NodeReleaseIdentity.hasWriterLease(status: [:]), "R11 missing writer lease is unknown")
    print("all passed")
} catch { cleanup(); print("FAIL R11 listener fixture:", error); exit(1) }
