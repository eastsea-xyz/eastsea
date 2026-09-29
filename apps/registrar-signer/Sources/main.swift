import CryptoKit
import Darwin
import Foundation

// aether-registrar-signer: the registrar's P-256 attestation key, in the Secure
// Enclave of a signing-only Mac (docs/ops/registrar.md).
//
//   aether-registrar-signer init [--touch-id]      owner, once: create the key
//   aether-registrar-signer serve [--socket PATH]  sign for the registrar node
//   aether-registrar-signer public                 print the key (x‖y hex)
//   aether-registrar-signer sign --msg HEX         one signature (checks, tests)
//
// The node signs with this key instead of `<data>/registrar.key` when it runs
// with `--registrar-signer <socket>`. The key cannot be exported, copied to
// another Mac or read out of the process; a committee-signed upgrade can rotate
// it or stop the registrar entirely, and the node refuses to sign with a key the
// registry no longer holds.

let version = "0.1.0"

let usage = """
aether-registrar-signer \(version) — the registrar key in this Mac's Secure Enclave

  init [--touch-id]        create the key (once, by the owner). Without --touch-id the
                           helper signs unattended (from the first login after a reboot);
                           with it every signature asks for Touch ID.
  serve [--socket PATH]    serve the registrar node over a local socket
                           (default \(Paths.socket.path))
  public                   print the registrar public key (x‖y hex) for
                           `aether network --registrar` / the committee upgrade
  sign --msg HEX           sign one message and print r and s (diagnostics)
  version

The node starts with:
  aether run … --registrar-signer \(Paths.socket.path)
"""

func fail(_ message: String) -> Never {
    FileHandle.standardError.write(Data("aether-registrar-signer: \(message)\n".utf8))
    exit(1)
}

func argument(_ name: String, _ args: [String]) -> String? {
    guard let i = args.firstIndex(of: name), i + 1 < args.count else { return nil }
    return args[i + 1]
}

/// Writing to a socket whose node went away must not kill the helper.
signal(SIGPIPE, SIG_IGN)

let argv = CommandLine.arguments
let command = argv.count > 1 ? argv[1] : "help"
let rest = Array(argv.dropFirst(2))

switch command {
case "init":
    do {
        let key = try Key.create(touchID: rest.contains("--touch-id"))
        print("registrar key \(Key.publicHex(key))")
        print("in \(Paths.key.path) (Secure Enclave: it cannot leave this Mac)")
        print("put it on chain with `aether network --registrar \(Key.publicHex(key))`,")
        print("then start the service: aether-registrar-signer serve")
    } catch {
        fail("\(error)")
    }

case "serve":
    do {
        let key = try Key.load()
        let socket = argument("--socket", rest) ?? Paths.socket.path
        try Paths.ensure()
        try Server(key: key).serve(path: socket)
    } catch {
        fail("\(error)")
    }

case "public":
    do {
        let key = try Key.load()
        print(Key.publicHex(key))
    } catch {
        fail("\(error)")
    }

case "sign":
    guard let hex = argument("--msg", rest) else { fail("sign needs --msg <hex>") }
    guard let message = Data(hexString: hex) else { fail("--msg must be hex") }
    do {
        let key = try Key.load()
        let sig = try Key.sign(key, message: message)
        print("r \(hexString(sig.prefix(32)))")
        print("s \(hexString(sig.suffix(32)))")
    } catch {
        fail("\(error)")
    }

case "version", "--version":
    print(version)

default:
    print(usage)
}

/// Lowercase hex of `bytes`.
func hexString(_ bytes: Data) -> String {
    bytes.map { String(format: "%02x", $0) }.joined()
}

extension Data {
    /// Hex → bytes, tolerant of a `0x` prefix.
    init?(hexString: String) {
        let clean = hexString.hasPrefix("0x") ? String(hexString.dropFirst(2)) : hexString
        guard clean.count % 2 == 0 else { return nil }
        var out = Data(capacity: clean.count / 2)
        var i = clean.startIndex
        while i < clean.endIndex {
            let j = clean.index(i, offsetBy: 2)
            guard let b = UInt8(clean[i..<j], radix: 16) else { return nil }
            out.append(b)
            i = j
        }
        self = out
    }
}
