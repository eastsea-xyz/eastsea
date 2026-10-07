#if os(macOS)
import Foundation
import Security
import Darwin

/// The signed helper's exact code identity. Dynamic validity ties the kernel's
/// loaded CodeDirectory to disk before the exact CDHash requirement is checked.
/// Merely copying a running guest's signing information is not sufficient:
/// that API may reopen the executable now present at the original path.
enum NodeReleaseIdentity {
    static func validatedHash(of expected: URL) -> Data? {
        var code: SecStaticCode?
        let flags = SecCSFlags(rawValue: kSecCSStrictValidate)
        guard SecStaticCodeCreateWithPath(expected as CFURL, SecCSFlags(), &code) == errSecSuccess,
              let code, SecStaticCodeCheckValidity(code, flags, nil) == errSecSuccess else { return nil }
        var information: CFDictionary?
        guard SecCodeCopySigningInformation(code, SecCSFlags(), &information) == errSecSuccess,
              let information,
              let hash = (information as NSDictionary)[kSecCodeInfoUnique] as? Data,
              hash.count == 20 else { return nil }
        return hash
    }

    /// The supervisor's signature is not the RPC server's signature.
    /// Inspect only that root and its bounded descendants, require exactly
    /// one loopback LISTEN owner, and validate both signed images. Any
    /// incomplete/recycled/reparented process or FD observation fails closed.
    /// Identifies the same signed listener instance across an RPC read.
    struct Binding: Equatable {
        let rootPID: Int32
        let rootStartedSeconds: UInt64
        let rootStartedMicros: UInt64
        let ownerPID: Int32
        let ownerStartedSeconds: UInt64
        let ownerStartedMicros: UInt64
        let fd: Int32
        let socket: UInt64
        let socketGeneration: UInt64
        let expectedHash: Data
    }

    /// The response is usable only if a fresh connection read was bracketed
    /// by the same complete signed-listener proof. Never accept a response
    /// merely because a replacement listener became valid afterwards.
    static func readVerified<T>(rootPID: Int32, port: UInt16, expected: URL,
                               operation: () async -> T?) async -> (value: T, binding: Binding)? {
        guard let before = binding(rootPID: rootPID, port: port, expected: expected) else { return nil }
        guard let value = await operation(),
              binding(rootPID: rootPID, port: port, expected: expected) == before else { return nil }
        return (value, before)
    }

    static func hasWriterLease(status: Any?) -> Bool {
        guard let status = status as? [String: Any],
              let value = status["writer_lease_protocol"] as? NSNumber,
              CFGetTypeID(value) != CFBooleanGetTypeID(),
              ["c", "s", "i", "l", "q", "C", "S", "I", "L", "Q"].contains(String(cString: value.objCType)) else { return false }
        return value.stringValue == "1"
    }

    static func matchesNode(rootPID: Int32, port: UInt16, expected: URL) -> Bool {
        binding(rootPID: rootPID, port: port, expected: expected) != nil
    }

    static func matches(binding attested: Binding, port: UInt16, expected: URL) -> Bool {
        binding(rootPID: attested.rootPID, port: port, expected: expected) == attested
    }

    static func binding(rootPID: Int32, port: UInt16, expected: URL) -> Binding? {
        guard port > 0, let expectedHash = validatedHash(of: expected),
              matches(pid: rootPID, expected: expected),
              let before = processTree(rootPID: rootPID), let root = before[rootPID] else { return nil }
        var owners: [Listener] = []
        for process in before.values {
            guard let owned = Self.listeners(of: process, port: port) else { return nil }
            owners.append(contentsOf: owned)
        }
        guard owners.count == 1, let listener = owners.first,
              matches(pid: listener.process.pid, expected: expected),
              let after = processTree(rootPID: rootPID), after == before,
              processStamp(pid: listener.process.pid) == listener.process,
              let current = listenerSocket(pid: listener.process.pid, fd: listener.fd, port: port),
              current == listener.socket, validatedHash(of: expected) == expectedHash else { return nil }
        return Binding(rootPID: rootPID, rootStartedSeconds: root.startedSeconds,
                       rootStartedMicros: root.startedMicros, ownerPID: listener.process.pid,
                       ownerStartedSeconds: listener.process.startedSeconds,
                       ownerStartedMicros: listener.process.startedMicros, fd: listener.fd,
                       socket: listener.socket.socket, socketGeneration: listener.socket.generation,
                       expectedHash: expectedHash)
    }

    private struct ProcessStamp: Equatable {
        let pid: Int32
        let parent: UInt32
        let startedSeconds: UInt64
        let startedMicros: UInt64
    }

    private struct SocketStamp: Equatable {
        let socket: UInt64
        let generation: UInt64
    }

    private struct Listener {
        let process: ProcessStamp
        let fd: Int32
        let socket: SocketStamp
    }

    private static func processStamp(pid: Int32) -> ProcessStamp? {
        guard pid > 1 else { return nil }
        var info = proc_bsdinfo()
        let size = Int32(MemoryLayout<proc_bsdinfo>.stride)
        let count = withUnsafeMutablePointer(to: &info) {
            proc_pidinfo(pid, Int32(PROC_PIDTBSDINFO), 0, $0, size)
        }
        guard count == size, info.pbi_pid == UInt32(pid),
              (info.pbi_flags & UInt32(PROC_FLAG_INEXIT)) == 0 else { return nil }
        return ProcessStamp(pid: pid, parent: info.pbi_ppid,
                            startedSeconds: info.pbi_start_tvsec, startedMicros: info.pbi_start_tvusec)
    }

    private static func childPIDs(of pid: Int32) -> [Int32]? {
        // Unlike proc_pidinfo, proc_listchildpids returns a PID COUNT.
        errno = 0
        let estimated = proc_listchildpids(pid, nil, 0)
        guard estimated >= 0, errno == 0, estimated <= 16_384 else { return nil }
        let capacity = max(Int(estimated) + 16, 16)
        var pids = [Int32](repeating: 0, count: capacity)
        errno = 0
        let count = pids.withUnsafeMutableBytes {
            proc_listchildpids(pid, $0.baseAddress, Int32($0.count))
        }
        guard count >= 0, errno == 0, Int(count) < capacity else { return nil }
        return Array(pids.prefix(Int(count)).filter { $0 > 1 })
    }

    private static func processTree(rootPID: Int32) -> [Int32: ProcessStamp]? {
        guard let root = processStamp(pid: rootPID) else { return nil }
        var result = [rootPID: root]
        var pending = [rootPID]
        var next = 0
        while next < pending.count {
            let parent = pending[next]
            next += 1
            guard let children = childPIDs(of: parent) else { return nil }
            for pid in children {
                guard result[pid] == nil, result.count < 64,
                      let child = processStamp(pid: pid), child.parent == UInt32(parent) else { return nil }
                result[pid] = child
                pending.append(pid)
            }
        }
        for process in result.values {
            guard processStamp(pid: process.pid) == process else { return nil }
        }
        return result
    }

    private static func socketInfo(pid: Int32, fd: Int32) -> socket_fdinfo? {
        var info = socket_fdinfo()
        let size = Int32(MemoryLayout<socket_fdinfo>.stride)
        let count = withUnsafeMutablePointer(to: &info) {
            proc_pidfdinfo(pid, fd, Int32(PROC_PIDFDSOCKETINFO), $0, size)
        }
        return count == size ? info : nil
    }

    private static func isEndpointListener(_ info: socket_fdinfo, port: UInt16) -> Bool {
        let socket = info.psi
        guard socket.soi_family == Int32(AF_INET), socket.soi_kind == Int32(SOCKINFO_TCP),
              socket.soi_type == Int32(SOCK_STREAM), socket.soi_protocol == Int32(IPPROTO_TCP) else { return false }
        let tcp = socket.soi_proto.pri_tcp
        let address = tcp.tcpsi_ini.insi_laddr.ina_46.i46a_addr4.s_addr
        return tcp.tcpsi_state == Int32(TSI_S_LISTEN)
            && UInt16(bigEndian: UInt16(truncatingIfNeeded: tcp.tcpsi_ini.insi_lport)) == port
            && address == UInt32(0x7f00_0001).bigEndian
    }

    private static func listener(_ info: socket_fdinfo, port: UInt16) -> SocketStamp? {
        guard isEndpointListener(info, port: port),
              (UInt32(UInt16(bitPattern: info.psi.soi_options)) & UInt32(SO_REUSEPORT)) == 0 else { return nil }
        return SocketStamp(socket: info.psi.soi_so, generation: info.psi.soi_proto.pri_tcp.tcpsi_ini.insi_gencnt)
    }

    private static func listenerSocket(pid: Int32, fd: Int32, port: UInt16) -> SocketStamp? {
        socketInfo(pid: pid, fd: fd).flatMap { listener($0, port: port) }
    }

    private static func listeners(of process: ProcessStamp, port: UInt16) -> [Listener]? {
        let stride = MemoryLayout<proc_fdinfo>.stride
        errno = 0
        let needed = proc_pidinfo(process.pid, Int32(PROC_PIDLISTFDS), 0, nil, 0)
        guard needed >= 0, errno == 0, Int(needed) % stride == 0,
              Int(needed) / stride <= 16_384 else { return nil }
        let capacity = max(Int(needed) / stride + 16, 16)
        var fds = [proc_fdinfo](repeating: proc_fdinfo(), count: capacity)
        errno = 0
        let bytes = fds.withUnsafeMutableBytes {
            proc_pidinfo(process.pid, Int32(PROC_PIDLISTFDS), 0, $0.baseAddress, Int32($0.count))
        }
        guard bytes >= 0, errno == 0, Int(bytes) % stride == 0,
              Int(bytes) < capacity * stride else { return nil }
        var result: [Listener] = []
        for fd in fds.prefix(Int(bytes) / stride) where fd.proc_fdtype == UInt32(PROX_FDTYPE_SOCKET) {
            guard let info = socketInfo(pid: process.pid, fd: fd.proc_fd) else { return nil }
            if isEndpointListener(info, port: port) {
                guard let socket = listener(info, port: port) else { return nil }
                result.append(Listener(process: process, fd: fd.proc_fd, socket: socket))
            }
        }
        guard processStamp(pid: process.pid) == process else { return nil }
        return result
    }

    static func matches(pid: Int32, expected: URL) -> Bool {
        guard pid > 1, let hash = validatedHash(of: expected) else { return false }
        let hex = hash.map { String(format: "%02x", $0) }.joined()
        var requirement: SecRequirement?
        guard SecRequirementCreateWithString("cdhash H\"\(hex)\"" as CFString,
                    SecCSFlags(), &requirement) == errSecSuccess, let requirement else { return false }
        var guest: SecCode?
        let attributes = [kSecGuestAttributePid as String: NSNumber(value: pid)] as CFDictionary
        guard SecCodeCopyGuestWithAttributes(nil, attributes, SecCSFlags(), &guest) == errSecSuccess,
              let guest else { return false }
        let flags = SecCSFlags(rawValue: kSecCSStrictValidate)
        return SecCodeCheckValidity(guest, flags, requirement) == errSecSuccess
    }
}
#endif
