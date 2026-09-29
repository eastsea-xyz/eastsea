import CryptoKit
import Darwin
import Foundation

/// The local signing service the registrar node talks to
/// (docs/ops/registrar.md). One JSON object per line, one request per
/// connection:
///
/// ```text
/// → {"op":"public"}
/// ← {"ok":true,"public":"<x hex><y hex>","secure_enclave":true}
/// → {"op":"sign","msg":"<message hex>"}
/// ← {"ok":true,"r":"<32-byte hex>","s":"<32-byte hex>"}
/// ← {"ok":false,"error":"…"}
/// ```
///
/// The socket lives in `~/Library/Application Support/Aether/registrar`
/// (0700), is chmod 0600, and only this user's processes are served: the key
/// signs for the registrar node on this Mac and nobody else.
final class Server {
    /// Cap on one request line, as on the node side (`MAX_LINE`).
    static let maxLine = 1 << 16

    private let key: SecureEnclave.P256.Signing.PrivateKey
    /// Signing is serialized: one Secure Enclave operation (and, with `--touch-id`,
    /// one Touch ID prompt) at a time.
    private let signLock = NSLock()

    init(key: SecureEnclave.P256.Signing.PrivateKey) {
        self.key = key
    }

    // MARK: - serving

    func serve(path: String) throws -> Never {
        if Self.isServed(path) {
            throw SignerError.io("\(path) is already served: one signing Mac runs one helper (quit the other one first)")
        }
        let fd = try listen(path: path)
        try? FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: path)
        print("aether-registrar-signer: serving \(path)")
        print("registrar public key \(Key.publicHex(key))")
        while true {
            let client = accept(fd, nil, nil)
            if client < 0 {
                if errno == EINTR { continue }
                throw SignerError.io("accept: \(String(cString: strerror(errno)))")
            }
            // Only this user's processes: the socket is 0600 and the directory
            // 0700, but say no on purpose rather than by accident.
            var uid: uid_t = 0
            var gid: gid_t = 0
            if getpeereid(client, &uid, &gid) == 0 && uid != getuid() {
                _ = Self.reply(client, ["ok": false, "error": "only the owner of the signing Mac may use the registrar key"])
                close(client)
                continue
            }
            DispatchQueue.global().async { [self] in
                Self.handle(client: client, key: key, lock: signLock)
            }
        }
    }

    private func listen(path: String) throws -> Int32 {
        var addr = try Self.address(path)
        let fd = socket(AF_UNIX, SOCK_STREAM, 0)
        guard fd >= 0 else { throw SignerError.io("socket: \(String(cString: strerror(errno)))") }
        let size = socklen_t(MemoryLayout<sockaddr_un>.size)
        let rc = withUnsafePointer(to: &addr) { p in
            p.withMemoryRebound(to: sockaddr.self, capacity: 1) { bind(fd, $0, size) }
        }
        if rc != 0 {
            let why = String(cString: strerror(errno))
            close(fd)
            throw SignerError.io("cannot serve \(path): \(why)")
        }
        guard Darwin.listen(fd, 8) == 0 else {
            let why = String(cString: strerror(errno))
            close(fd)
            throw SignerError.io("listen \(path): \(why)")
        }
        return fd
    }

    /// Whether a helper is already serving `path`. A socket file left behind by
    /// a crash is removed here: nothing answers on it.
    private static func isServed(_ path: String) -> Bool {
        guard var addr = try? address(path) else { return false }
        let fd = socket(AF_UNIX, SOCK_STREAM, 0)
        guard fd >= 0 else { return false }
        let size = socklen_t(MemoryLayout<sockaddr_un>.size)
        let rc = withUnsafePointer(to: &addr) { p in
            p.withMemoryRebound(to: sockaddr.self, capacity: 1) { connect(fd, $0, size) }
        }
        close(fd)
        if rc == 0 { return true }
        try? FileManager.default.removeItem(atPath: path)
        return false
    }

    /// The `sockaddr_un` of a filesystem socket path.
    private static func address(_ path: String) throws -> sockaddr_un {
        var addr = sockaddr_un()
        addr.sun_family = sa_family_t(AF_UNIX)
        addr.sun_len = UInt8(MemoryLayout<sockaddr_un>.size)
        let bytes = Array(path.utf8)
        guard bytes.count < MemoryLayout.size(ofValue: addr.sun_path) else {
            throw SignerError.input("socket path is too long: \(path)")
        }
        withUnsafeMutableBytes(of: &addr.sun_path) { buf in
            for (i, b) in bytes.enumerated() { buf[i] = b }
        }
        return addr
    }

    // MARK: - one connection

    private static func handle(client: Int32, key: SecureEnclave.P256.Signing.PrivateKey, lock: NSLock) {
        defer { close(client) }
        guard let line = readLine(client), let request = try? JSONSerialization.jsonObject(with: line) as? [String: Any] else {
            _ = reply(client, ["ok": false, "error": "expected one JSON object per line"])
            return
        }
        switch request["op"] as? String {
        case "public":
            _ = reply(client, ["ok": true, "public": Key.publicHex(key), "secure_enclave": true])
        case "sign":
            guard let hex = request["msg"] as? String, let message = Self.hex(hex) else {
                _ = reply(client, ["ok": false, "error": "\"msg\" must be hex"])
                return
            }
            do {
                lock.lock()
                defer { lock.unlock() }
                let sig = try Key.sign(key, message: message)
                _ = reply(client, [
                    "ok": true,
                    "r": Self.hex(sig.prefix(32)),
                    "s": Self.hex(sig.suffix(32)),
                ])
            } catch {
                _ = reply(client, ["ok": false, "error": "the Secure Enclave refused to sign: \(error)"])
            }
        case .some(let op):
            _ = reply(client, ["ok": false, "error": "unknown op \"\(op)\""])
        case .none:
            _ = reply(client, ["ok": false, "error": "no \"op\""])
        }
    }

    /// One bounded line (the node sends exactly one, then waits).
    private static func readLine(_ fd: Int32) -> Data? {
        var out = Data()
        var buf = [UInt8](repeating: 0, count: 4096)
        while out.count <= maxLine {
            let n = read(fd, &buf, buf.count)
            if n <= 0 { break }
            out.append(contentsOf: buf[0..<n])
            if out.contains(0x0A) { break }
        }
        guard !out.isEmpty else { return nil }
        return out.prefix { $0 != 0x0A }
    }

    private static func reply(_ fd: Int32, _ obj: [String: Any]) -> Bool {
        guard var data = try? JSONSerialization.data(withJSONObject: obj) else { return false }
        data.append(0x0A)
        return data.withUnsafeBytes { raw -> Bool in
            var sent = 0
            while sent < raw.count {
                let n = write(fd, raw.baseAddress!.advanced(by: sent), raw.count - sent)
                if n <= 0 { return false }
                sent += n
            }
            return true
        }
    }

    private static func hex(_ bytes: Data) -> String {
        bytes.map { String(format: "%02x", $0) }.joined()
    }

    private static func hex(_ s: String) -> Data? {
        let clean = s.hasPrefix("0x") ? String(s.dropFirst(2)) : s
        guard clean.count % 2 == 0 else { return nil }
        var out = Data(capacity: clean.count / 2)
        var i = clean.startIndex
        while i < clean.endIndex {
            let j = clean.index(i, offsetBy: 2)
            guard let b = UInt8(clean[i..<j], radix: 16) else { return nil }
            out.append(b)
            i = j
        }
        return out
    }
}
