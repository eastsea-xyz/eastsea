import Foundation
import CryptoKit

/// Native identity of a running release. A page cannot choose its displayed
/// name, permission namespace or website data store.
struct AppBrowserIdentity: Equatable, Sendable {
    static let scheme = "eastsea-app"
    let appKey: String
    let displayOrigin: String
    let permissionKey: String
    let chainID: UInt64
    let isDeveloper: Bool
    let storeID: UUID

    enum Failure: Error { case invalidIdentity }

    init(appID: String, name: String? = nil, chainID: UInt64, registry: String) throws {
        let hex = appID.hasPrefix("0x") ? String(appID.dropFirst(2)) : appID
        let registryHex = registry.hasPrefix("0x") ? String(registry.dropFirst(2)) : registry
        guard let id = Self.hexBytes(hex), id.count == 32,
              let registryBytes = Self.hexBytes(registryHex), registryBytes.count == 20,
              chainID != 0, name.map(Self.validName) ?? true else { throw Failure.invalidIdentity }
        let key = Self.base32(id)
        appKey = key
        displayOrigin = "sea://\(name ?? key)"
        permissionKey = "\(Self.scheme)://\(chainID)/\(registryHex.lowercased())/\(hex.lowercased())"
        self.chainID = chainID
        isDeveloper = false
        storeID = Self.storeIdentifier(permissionKey)
    }

    /// A fresh namespace on every folder open prevents local files from
    /// inheriting a registered app's connection or saved website data.
    init(developerSession: UUID, chainID: UInt64) {
        let id = Data(SHA256.hash(data: Data(developerSession.uuidString.utf8)))
        appKey = Self.base32(id)
        displayOrigin = "eastsea-dev://local"
        permissionKey = "eastsea-dev://\(chainID)/\(appKey)"
        self.chainID = chainID
        isDeveloper = true
        storeID = Self.storeIdentifier(permissionKey)
    }

    func accepts(scheme: String, host: String, port: Int, mainFrame: Bool,
                 currentChainID: UInt64, developerMode: Bool) -> Bool {
        mainFrame && scheme.lowercased() == Self.scheme && host.lowercased() == appKey
            && port == 0 && chainID == currentChainID && (!isDeveloper || developerMode)
    }

    /// Local files may sign only against the wallet's owned development chain.
    func permitsSigning(developerMode: Bool) -> Bool {
        !isDeveloper || (developerMode && chainID == 7777)
    }

    private static func validName(_ name: String) -> Bool {
        guard name == name.lowercased(), name.hasSuffix(".sea"), name.utf8.count <= 253 else { return false }
        let labels = name.split(separator: ".", omittingEmptySubsequences: false)
        return labels.count >= 2 && labels.allSatisfy { label in
            !label.isEmpty && label.utf8.count <= 63 && label.first != "-" && label.last != "-"
                && label.utf8.allSatisfy { (97...122).contains($0) || (48...57).contains($0) || $0 == 45 }
        }
    }

    private static func hexBytes(_ hex: String) -> Data? {
        guard !hex.isEmpty, hex.utf8.count % 2 == 0,
              hex.utf8.allSatisfy({ (48...57).contains($0) || (97...102).contains($0) || (65...70).contains($0) }) else { return nil }
        let bytes = Array(hex.utf8)
        return Data(stride(from: 0, to: bytes.count, by: 2).map {
            UInt8(String(decoding: bytes[$0...($0 + 1)], as: UTF8.self), radix: 16)!
        })
    }

    private static func base32(_ data: Data) -> String {
        let alphabet = Array("abcdefghijklmnopqrstuvwxyz234567".utf8)
        var output = [UInt8](), bits = 0, buffer: UInt32 = 0
        for byte in data {
            buffer = (buffer << 8) | UInt32(byte)
            bits += 8
            while bits >= 5 {
                bits -= 5
                output.append(alphabet[Int((buffer >> bits) & 31)])
            }
            buffer &= (1 << bits) - 1
        }
        if bits > 0 { output.append(alphabet[Int((buffer << (5 - bits)) & 31)]) }
        return String(decoding: output, as: UTF8.self)
    }

    private static func storeIdentifier(_ namespace: String) -> UUID {
        var bytes = Array(SHA256.hash(data: Data(namespace.utf8)).prefix(16))
        bytes[6] = (bytes[6] & 0x0f) | 0x80
        bytes[8] = (bytes[8] & 0x3f) | 0x80
        return UUID(uuid: (bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
                           bytes[8], bytes[9], bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15]))
    }
}

/// The privileged handler lives in its own WebKit content world. The page
/// facade transports requests; Swift still authenticates the view and origin.
enum AppProviderBridge {
    static let worldName = "eastsea-app-provider"
    static let relay = #"""
    (() => {
      const channels = new Set(['aether', 'eastsea']);
      window.addEventListener('message', async (event) => {
        const m = event.data;
        if (event.source !== window || !m || m.type !== 'eastsea-app-request'
            || !channels.has(m.channel) || typeof m.id !== 'string' || m.id.length > 128) return;
        let reply;
        try { reply = await window.webkit.messageHandlers[m.channel].postMessage(m.payload); }
        catch (_) { reply = {error: {code: -32603, message: 'The wallet did not answer.'}}; }
        window.postMessage({type: 'eastsea-app-reply', id: m.id, reply}, '*');
      });
    })();
    """#

    static func facade(provider: String) -> String {
        let transport = #"""
        (() => {
          const pending = new Map(); let sequence = 0;
          window.addEventListener('message', (event) => {
            const m = event.data;
            if (event.source !== window || !m || m.type !== 'eastsea-app-reply') return;
            const answer = pending.get(m.id);
            if (answer) { pending.delete(m.id); answer(m.reply); }
          });
          Object.defineProperty(window, '__eastseaAppTransport', {value: (channel) => ({
            postMessage: (payload) => new Promise((resolve) => {
              const id = `app-${++sequence}`;
              pending.set(id, resolve);
              window.postMessage({type: 'eastsea-app-request', channel, id, payload}, '*');
            })
          }), writable: false, configurable: false});
        })();
        """#
        return transport + "\n" + provider
            .replacingOccurrences(of: "window.webkit && window.webkit.messageHandlers && window.webkit.messageHandlers.aether",
                                  with: "window.__eastseaAppTransport('aether')")
            .replacingOccurrences(of: "window.webkit && window.webkit.messageHandlers && window.webkit.messageHandlers.eastsea",
                                  with: "window.__eastseaAppTransport('eastsea')")
    }
}
