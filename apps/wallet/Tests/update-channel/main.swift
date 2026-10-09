// The canary channel switch (docs/design/32-health-signal.md §5.2, W5): only
// `updateChannel == "canary"` lets Sparkle see canary items. Pure logic:
//   swiftc -o ./tmp/update-channel apps/wallet/Sources/ReleaseApproval.swift apps/wallet/Sources/UpdateChannel.swift apps/wallet/Tests/update-channel/main.swift && ./tmp/update-channel
import Foundation
func check(_ c: Bool, _ m: String) { if !c { print("FAIL", m); exit(1) } }

#if os(macOS)
check(!UpdateChannel.isNewerBuild("74", than: "74"), "installed announcements preserve post-install health tracking")
check(!UpdateChannel.isNewerBuild("73", than: "74"), "an older chain announcement cannot restart discovery")
check(UpdateChannel.isNewerBuild("75", than: "74"), "a newer signed build remains discoverable")
check(UpdateChannel.isNewerBuild("75", than: nil), "missing bundle metadata still reaches the proof gate")
let legacy = ReleaseTrust(chainId: 7_780, logAddress: "", codeHash: "", builderKeys: [], legacy: true)
check(UpdateChannel.pollsForDiscovery(trust: legacy, activeChainId: 7_780), "the bundled legacy chain keeps discovery")
check(!UpdateChannel.pollsForDiscovery(trust: legacy, activeChainId: 9_001), "switching to a new chain turns discovery polling off")
check(!UpdateChannel.pollsForDiscovery(trust: legacy, activeChainId: 0), "an unknown active chain cannot enable discovery polling")
#endif

let suite = "eastsea.test.update-channel.\(ProcessInfo.processInfo.processIdentifier)"
guard let defaults = UserDefaults(suiteName: suite) else { print("FAIL no defaults suite"); exit(1) }
defer { defaults.removePersistentDomain(forName: suite) }

check(UpdateChannel.allowedChannels(defaults).isEmpty, "default: the empty set (channel items are ignored)")

defaults.set("canary", forKey: UpdateChannel.defaultsKey)
check(UpdateChannel.allowedChannels(defaults) == ["canary"], "updateChannel = canary allows exactly the canary channel")

for other in ["Canary", "CANARY", " canary", "canary ", "beta", "stable", "", "canary,beta"] {
    defaults.set(other, forKey: UpdateChannel.defaultsKey)
    check(UpdateChannel.allowedChannels(defaults).isEmpty, "\"\(other)\" is not the canary switch")
}

// `defaults write … updateChannel -bool YES` and other types: not a string, not canary.
defaults.set(true, forKey: UpdateChannel.defaultsKey)
check(UpdateChannel.allowedChannels(defaults).isEmpty, "a boolean is not the canary switch")
defaults.set(1, forKey: UpdateChannel.defaultsKey)
check(UpdateChannel.allowedChannels(defaults).isEmpty, "a number is not the canary switch")
defaults.set(["canary"], forKey: UpdateChannel.defaultsKey)
check(UpdateChannel.allowedChannels(defaults).isEmpty, "an array is not the canary switch")

defaults.removeObject(forKey: UpdateChannel.defaultsKey)
check(UpdateChannel.allowedChannels(defaults).isEmpty, "removing the key returns to the default")

print("update-channel: all checks passed")
