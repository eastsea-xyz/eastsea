#if os(macOS)
import Darwin
import Foundation

/// An owner typed the address in the wallet and authenticated it. A PTY
/// preserves the CLI's mandatory interactive-confirmation check; a pipe or
/// an automatic-confirmation flag would bypass that protection.
enum NodeKeyRebindCommand {
    struct Failure: LocalizedError {
        let detail: String
        var errorDescription: String? { detail }
    }

    static func run(binary: URL, dataDirectory: URL, approval: NodeKeyRebind.Approval) throws -> String {
        guard approval.dataDirectory == dataDirectory.path else {
            throw Failure(detail: "Owner approval names a different node data directory.")
        }
        var master: Int32 = -1, slave: Int32 = -1
        guard openpty(&master, &slave, nil, nil, nil) == 0 else {
            throw Failure(detail: "The confirmation terminal could not be opened: \(String(cString: strerror(errno)))")
        }
        let input = FileHandle(fileDescriptor: master, closeOnDealloc: true)
        let terminal = FileHandle(fileDescriptor: slave, closeOnDealloc: true)
        defer { try? input.close(); try? terminal.close() }
        let command = Process()
        command.executableURL = binary
        command.arguments = ["keys", "rebind", "--data", dataDirectory.path]
        command.standardInput = terminal
        command.standardOutput = terminal
        command.standardError = terminal
        defer {
            if command.isRunning { command.terminate(); command.waitUntilExit() }
        }
        try command.run()
        // The child owns copies of its terminal descriptors. Closing ours
        // allows the PTY reader to finish once the CLI exits.
        try terminal.close()
        try input.write(contentsOf: Data(approval.confirmationLine.utf8))

        var output = Data(), buffer = [UInt8](repeating: 0, count: 4_096)
        while true {
            let count = buffer.withUnsafeMutableBytes { Darwin.read(master, $0.baseAddress, $0.count) }
            if count > 0 {
                output.append(contentsOf: buffer.prefix(count))
                if output.count > 65_536 { output.removeFirst(output.count - 65_536) }
            } else if count == 0 || errno == EIO {
                break
            } else if errno != EINTR {
                throw Failure(detail: "The confirmation terminal could not be read: \(String(cString: strerror(errno)))")
            }
        }
        command.waitUntilExit()
        let text = String(decoding: output, as: UTF8.self)
            .replacingOccurrences(of: "\r", with: "").trimmingCharacters(in: .whitespacesAndNewlines)
        guard command.terminationReason == .exit, command.terminationStatus == 0 else {
            throw Failure(detail: text.isEmpty ? "The node keys were not rebound." : text)
        }
        return text
    }
}
#endif
