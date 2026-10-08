import Foundation

var checks = 0
func check(_ value: Bool, _ message: String) {
    checks += 1
    if !value { fatalError(message) }
}
let appID = "0x" + String(repeating: "5", count: 64)
let registry = "0x" + String(repeating: "4", count: 40)
let app = try AppBrowserIdentity(appID: appID, name: "demo.sea", chainID: 7794, registry: registry)
check(app.appKey.count == 52 && app.appKey.allSatisfy { "abcdefghijklmnopqrstuvwxyz234567".contains($0) }, "app origin is one bounded host label")
check(app.displayOrigin == "sea://demo.sea", "the native display uses the resolved name")
check(app.accepts(scheme: "eastsea-app", host: app.appKey, port: 0, mainFrame: true,
                  currentChainID: 7794, developerMode: false), "registered frame is accepted")
for (scheme, host, port, main, chain) in [
    ("https", app.appKey, 0, true, UInt64(7794)),
    ("eastsea-app", "other", 0, true, UInt64(7794)),
    ("eastsea-app", app.appKey, 443, true, UInt64(7794)),
    ("eastsea-app", app.appKey, 0, false, UInt64(7794)),
    ("eastsea-app", app.appKey, 0, true, UInt64(1))
] {
    check(!app.accepts(scheme: scheme, host: host, port: port, mainFrame: main,
                      currentChainID: chain, developerMode: false), "foreign frame or chain cannot inherit the bridge")
}
let otherChain = try AppBrowserIdentity(appID: appID, name: "demo.sea", chainID: 1, registry: registry)
let otherRegistry = try AppBrowserIdentity(appID: appID, name: "demo.sea", chainID: 7794,
                                           registry: "0x" + String(repeating: "6", count: 40))
check(app.permissionKey != otherChain.permissionKey && app.storeID != otherChain.storeID, "permissions and website data are chain scoped")
check(app.permissionKey != otherRegistry.permissionKey && app.storeID != otherRegistry.storeID, "registries do not share permissions or data")
let alias = try AppBrowserIdentity(appID: appID, name: "alias.sea", chainID: 7794, registry: registry)
check(alias.permissionKey == app.permissionKey && alias.storeID == app.storeID, "app identity survives name aliases")
let dev = AppBrowserIdentity(developerSession: UUID(), chainID: 7777)
check(!dev.accepts(scheme: "eastsea-app", host: dev.appKey, port: 0, mainFrame: true,
                  currentChainID: 7777, developerMode: false), "turning developer mode off revokes local content")
check(dev.permitsSigning(developerMode: true) && !dev.permitsSigning(developerMode: false), "development signing requires the explicit mode")
check(!AppBrowserIdentity(developerSession: UUID(), chainID: 7780).permitsSigning(developerMode: true), "local files cannot sign on the live testnet")
check(!AppBrowserIdentity(developerSession: UUID(), chainID: 1).permitsSigning(developerMode: true), "local files cannot sign on mainnet")
for name in ["evil.com", "demo.sea@evil", "../demo.sea", "demo..sea", "Demo.sea", "-demo.sea"] {
    do { _ = try AppBrowserIdentity(appID: appID, name: name, chainID: 7794, registry: registry); fatalError("unsafe identity accepted") }
    catch AppBrowserIdentity.Failure.invalidIdentity { checks += 1 }
}
print("OK app-identity (\(checks) checks)")
