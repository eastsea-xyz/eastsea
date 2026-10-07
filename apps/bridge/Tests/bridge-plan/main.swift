// bridge-plan: the Aether 0.6.7 bridge's decisions (release-070 review, B2) —
// which EastSea it takes from the feed, which downloads it accepts, how it
// checks Sparkle's signature, which processes are the old node, and where
// EastSea goes.
import Foundation
import CryptoKit

var failures = 0
func expect(_ ok: Bool, _ what: String) {
    print(ok ? "ok   \(what)" : "FAIL \(what)")
    if !ok { failures += 1 }
}

// 1. The feed: the newest default-channel EastSea >= 0.7.0 from this repo.
let feed = """
<?xml version="1.0" encoding="utf-8"?>
<rss version="2.0" xmlns:sparkle="http://www.andymatuschak.org/xml-namespaces/sparkle">
  <channel>
    <title>EastSea</title>
    <item>
      <title>EastSea 0.7.0</title>
      <sparkle:version>13</sparkle:version>
      <sparkle:shortVersionString>0.7.0</sparkle:shortVersionString>
      <sparkle:minimumSystemVersion>14.0</sparkle:minimumSystemVersion>
      <enclosure url="https://github.com/eastsea-xyz/eastsea/releases/download/app-v0.7.0/EastSea-0.7.0.dmg" type="application/octet-stream" sparkle:edSignature="c2ln" length="123" />
    </item>
    <item>
      <title>EastSea 0.7.1 canary</title>
      <sparkle:version>14</sparkle:version>
      <sparkle:shortVersionString>0.7.1</sparkle:shortVersionString>
      <sparkle:channel>canary</sparkle:channel>
      <enclosure url="https://github.com/eastsea-xyz/eastsea/releases/download/app-v0.7.1/EastSea-0.7.1.dmg" sparkle:edSignature="c2ln" length="5" />
    </item>
    <item>
      <title>Elsewhere</title>
      <sparkle:version>99</sparkle:version>
      <sparkle:shortVersionString>9.9.9</sparkle:shortVersionString>
      <enclosure url="https://evil.example/EastSea.dmg" sparkle:edSignature="c2ln" length="5" />
    </item>
    <item>
      <title>Aether bridge (wrong feed)</title>
      <sparkle:version>12</sparkle:version>
      <sparkle:shortVersionString>0.6.7</sparkle:shortVersionString>
      <enclosure url="https://github.com/eastsea-xyz/eastsea/releases/download/app-v0.6.7/Aether-0.6.7.dmg" sparkle:edSignature="c2ln" length="5" />
    </item>
  </channel>
</rss>
"""
let items = BridgePlan.parseAppcast(Data(feed.utf8))
expect(items.count == 4, "the appcast parses into its four items: \(items.count)")
expect(items.first?.build == "13" && items.first?.version == "0.7.0" && items.first?.length == 123
       && items.first?.edSignature == "c2ln" && items.first?.minimumSystemVersion == "14.0",
       "an item's build, version, length, signature and minimum system come through")
expect(items[1].channel == "canary", "the channel is read")
let chosen = BridgePlan.choose(items, systemVersion: "15.1.0")
expect(chosen?.version == "0.7.0", "the bridge takes 0.7.0: not the canary, not a foreign host, not the bridge itself: \(String(describing: chosen?.version))")
expect(BridgePlan.choose(items, systemVersion: "13.6") == nil, "nothing for a macOS below the item's minimum")
expect(BridgePlan.parseAppcast(Data("not xml".utf8)).isEmpty, "garbage is no feed")
// Enclosure-attribute style (what generate_appcast writes) parses the same.
let attrStyle = """
<rss xmlns:sparkle="http://www.andymatuschak.org/xml-namespaces/sparkle"><channel><item>
<enclosure url="https://github.com/eastsea-xyz/eastsea/releases/download/app-v0.7.2/EastSea-0.7.2.dmg" sparkle:version="15" sparkle:shortVersionString="0.7.2" sparkle:edSignature="c2ln" length="9"/>
</item></channel></rss>
"""
expect(BridgePlan.choose(BridgePlan.parseAppcast(Data(attrStyle.utf8)), systemVersion: "14.0")?.build == "15",
       "versions carried as enclosure attributes are read too")

// 2. Downloads only from this repository's releases, over https.
for (url, ok) in [
    ("https://github.com/eastsea-xyz/eastsea/releases/download/app-v0.7.0/EastSea-0.7.0.dmg", true),
    ("http://github.com/eastsea-xyz/eastsea/releases/download/app-v0.7.0/EastSea-0.7.0.dmg", false),
    ("https://github.com/someone/aether-node/releases/download/app-v0.7.0/EastSea-0.7.0.dmg", false),
    ("https://github.com.evil.example/eastsea-xyz/eastsea/releases/download/x/EastSea.dmg", false),
    ("https://github.com/eastsea-xyz/eastsea/releases/download/app-v0.7.0/EastSea-0.7.0.zip", false),
] {
    expect(BridgePlan.isAllowedDownload(URL(string: url)!) == ok, "\(ok ? "accepts" : "refuses") \(url)")
}

// 3. Sparkle's EdDSA is Ed25519 over the archive bytes (checked against
//    Sparkle's own sign_update with a throwaway key file: same signatures).
do {
    let dir = FileManager.default.temporaryDirectory.appendingPathComponent("bridge-plan-\(UUID().uuidString)")
    try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
    let file = dir.appendingPathComponent("EastSea.dmg")
    let bytes = Data((0..<200_000).map { UInt8($0 % 251) })
    try? bytes.write(to: file)
    let key = Curve25519.Signing.PrivateKey()
    let pub = key.publicKey.rawRepresentation.base64EncodedString()
    let sig = (try? key.signature(for: bytes))?.base64EncodedString() ?? ""
    expect(BridgePlan.verifyEdDSA(file: file, signatureBase64: sig, publicKeyBase64: pub), "a correct signature verifies")
    var tampered = bytes
    tampered[1234] ^= 1
    try? tampered.write(to: file)
    expect(!BridgePlan.verifyEdDSA(file: file, signatureBase64: sig, publicKeyBase64: pub), "one flipped bit fails")
    let other = Curve25519.Signing.PrivateKey().publicKey.rawRepresentation.base64EncodedString()
    try? bytes.write(to: file)
    expect(!BridgePlan.verifyEdDSA(file: file, signatureBase64: sig, publicKeyBase64: other), "another key's signature fails")
    expect(!BridgePlan.verifyEdDSA(file: file, signatureBase64: "not base64", publicKeyBase64: pub), "a malformed signature fails")
    try? FileManager.default.removeItem(at: dir)
}

// 4. The code requirement names Developer ID, Pipln's team and EastSea's id.
let req = BridgePlan.requirement
expect(req.contains("anchor apple generic") && req.contains("identifier \"com.pipln.eastsea\"")
       && req.contains("certificate leaf[subject.OU] = \"45WU468FZE\"")
       && req.contains("1.2.840.113635.100.6.1.13"), "the requirement pins Developer ID, the team and the bundle id")
var compiled: SecRequirement?
expect(SecRequirementCreateWithString(req as CFString, [], &compiled) == errSecSuccess, "the requirement compiles")
expect(BridgePlan.gatekeeperAccepts(output: "/Applications/EastSea.app: accepted\nsource=Notarized Developer ID\norigin=Developer ID Application: Pipln (45WU468FZE)", exitCode: 0),
       "Gatekeeper's notarized verdict is accepted")
expect(!BridgePlan.gatekeeperAccepts(output: "accepted\nsource=Developer ID", exitCode: 0), "signed but not notarized is refused")
expect(!BridgePlan.gatekeeperAccepts(output: "rejected\nsource=Notarized Developer ID", exitCode: 3), "a rejection is refused")

// 5. The old node: Aether.app's helper of this user — not EastSea's, not root's.
let ps = """
  101   501 /Applications/Aether.app/Contents/Helpers/aether run --data /Users/a/Library/Application Support/Aether/node --exit-with-parent
  102   501 /Applications/EastSea.app/Contents/Helpers/aether run --data /Users/a/Library/Application Support/EastSea/node
  103     0 /Applications/Aether.app/Contents/Helpers/aether run --data /x
  104   501 /Applications/Aether.app/Contents/MacOS/Aether
  105   501 /Users/a/Downloads/Aether.app/Contents/Helpers/aether.prev follow --data /y
  106   501 /usr/bin/grep Aether.app/Contents/Helpers/aether
"""
expect(BridgePlan.oldNodePIDs(psOutput: ps, uid: 501) == [101, 105], "only this user's Aether node processes: \(BridgePlan.oldNodePIDs(psOutput: ps, uid: 501))")

// 6. Where EastSea goes, and what happens to one already there.
let home = URL(fileURLWithPath: "/Users/a")
expect(BridgePlan.installDirectory(applicationsWritable: true, home: home).path == "/Applications", "admins: /Applications")
expect(BridgePlan.installDirectory(applicationsWritable: false, home: home).path == "/Users/a/Applications", "others: ~/Applications")
expect(BridgePlan.existingDecision(installedVersion: nil, installedValid: false, candidateVersion: "0.7.0") == .none, "nothing there")
expect(BridgePlan.existingDecision(installedVersion: "0.7.1", installedValid: true, candidateVersion: "0.7.0") == .keep, "a newer genuine copy is kept")
expect(BridgePlan.existingDecision(installedVersion: "0.7.0", installedValid: true, candidateVersion: "0.7.0") == .keep, "the same genuine version is kept")
expect(BridgePlan.existingDecision(installedVersion: "0.6.9", installedValid: true, candidateVersion: "0.7.0") == .replace, "an older copy is replaced")
expect(BridgePlan.existingDecision(installedVersion: "0.7.5", installedValid: false, candidateVersion: "0.7.0") == .replace, "a copy that fails the signature check is replaced")
expect(BridgePlan.installedIsEnough(version: "0.7.0", valid: true), "an installed genuine 0.7.0 needs no download")
expect(!BridgePlan.installedIsEnough(version: "0.7.0", valid: false), "an unsigned or foreign EastSea is not enough")
expect(!BridgePlan.installedIsEnough(version: "0.6.6", valid: true), "a pre-0.7 EastSea build is not enough")

exit(Int32(failures))
