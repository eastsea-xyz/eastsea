import Foundation

func check(_ condition: Bool, _ message: String) {
    if !condition { print("FAIL", message); exit(1) }
}

check(LiveGlobeBundlePolicy.path(for: LiveGlobeBundlePolicy.entry) == "index.html", "entry is bundled")
check(LiveGlobeBundlePolicy.path(for: URL(string: "eastsea-globe://network/live-globe/live-globe.js")!) == "live-globe/live-globe.js", "canonical ES module is allowed")
for input in [
    "https://eastsea.xyz/index.html", "http://127.0.0.1:18545/", "file:///etc/passwd",
    "eastsea-globe://other/index.html", "eastsea-globe://user@network/index.html",
    "eastsea-globe://network:80/index.html", "eastsea-globe://network/index.html?rpc=remote",
    "eastsea-globe://network/index.html#fragment", "eastsea-globe://network/../network.json",
    "eastsea-globe://network/live-globe/%2e%2e/index.html", "eastsea-globe://network/network.json",
    "eastsea-globe://network/provider.js", "eastsea-globe://network/live-globe/fixture.json",
    "eastsea-globe://network//index.html",
] {
    check(LiveGlobeBundlePolicy.path(for: URL(string: input)!) == nil, "only globe resources are served: \(input)")
}
check(LiveGlobeBundlePolicy.allowsNavigation(to: LiveGlobeBundlePolicy.entry, mainFrame: true), "entry main frame allowed")
check(!LiveGlobeBundlePolicy.allowsNavigation(to: LiveGlobeBundlePolicy.entry, mainFrame: false), "subframes blocked")
check(!LiveGlobeBundlePolicy.allowsNavigation(to: URL(string: "https://eastsea.xyz"), mainFrame: true), "outside navigation blocked")
check(LiveGlobeBundlePolicy.contentSecurityPolicy.contains("connect-src 'none'"), "even loopback fetch is blocked")
check(!LiveGlobeBundlePolicy.contentSecurityPolicy.contains("unsafe-"), "no CSP bypass for script/style")

check(!LiveGlobeRenderingPolicy.paused(windowVisible: true, viewHidden: false, lowPower: false), "visible window can render")
check(LiveGlobeRenderingPolicy.paused(windowVisible: false, viewHidden: false, lowPower: false), "closed/minimized/occluded window pauses")
check(LiveGlobeRenderingPolicy.paused(windowVisible: true, viewHidden: true, lowPower: false), "hidden view pauses")
check(LiveGlobeRenderingPolicy.paused(windowVisible: true, viewHidden: false, lowPower: true), "battery saver pauses")
func update(_ aggregate: String? = "snapshot", paused: Bool = false, dark: Bool = false, width: Double = 712) -> LiveGlobeRenderingPolicy.Update {
    .init(aggregate: aggregate, state: aggregate == nil ? "loading" : "ready", paused: paused,
          reducedMotion: false, dark: dark, language: "en", fixture: false,
          evidenceAvailable: true, width: width)
}
check(update().shouldSend(after: nil), "first snapshot is sent")
check(!update().shouldSend(after: update()), "unrelated wallet publications cause no renderer redraw")
check(update(paused: true).shouldSend(after: update()), "entering battery saver sends the pause")
check(!update("new snapshot", paused: true).shouldSend(after: update(paused: true)), "battery saver holds new data without redrawing")
check(update("new snapshot").shouldSend(after: update(paused: true)), "resuming sends the newest held data")
check(update(paused: true).shouldSend(after: update(nil, paused: true)), "a first static snapshot can load in battery saver")
check(update(nil, paused: true).shouldSend(after: update(paused: true)), "network switch clears the old data even when paused")
check(update(paused: true, dark: true).shouldSend(after: update(paused: true)), "appearance changes remain readable while paused")
check(update(paused: true, width: 380).shouldSend(after: update(paused: true)), "explicit resize still lays out while paused")
print("ok")
