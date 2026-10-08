// The install-location rule (docs/design/24-self-healing.md, red team #10):
// where the app runs from decides whether its node may start here at all.
// Pure decision on a bundle path and a volume flag — no app:
//   swiftc -o ./tmp/install-location-check apps/wallet/Sources/Brand.swift apps/wallet/Sources/InstallLocation.swift apps/wallet/Tests/install-location/main.swift && ./tmp/install-location-check
import Foundation
func check(_ c: Bool, _ m: String) { if !c { print("FAIL", m); exit(1) } }

let home = "/Users/tester"
let homeApps = home + "/Applications"
func runnable(_ p: String, readOnly: Bool = false) -> Bool {
    InstallLocation.isRunnableLocation(bundlePath: p, homeApplicationsPath: homeApps, volumeIsReadOnly: readOnly)
}

// The places a person is told to use: runnable, subfolders included.
check(runnable("/Applications/EastSea.app"), "/Applications is runnable")
check(runnable("/Applications/Utilities/EastSea.app"), "an /Applications subfolder is runnable")
check(runnable(homeApps + "/EastSea.app"), "~/Applications is runnable")
check(runnable(homeApps + "/Games/EastSea.app"), "a ~/Applications subfolder is runnable")

// The wrong places (red team #10): the node's data and login item would point
// into a bundle that disappears at the next unmount or quarantine cleanup.
check(!runnable(home + "/Downloads/EastSea.app"), "Downloads is not runnable")
check(!runnable(home + "/Desktop/EastSea.app"), "Desktop is not runnable")
check(!runnable(home + "/Documents/EastSea.app"), "Documents is not runnable")
check(!runnable("/Volumes/EastSea/EastSea.app"), "a mounted DMG is not runnable")
check(!runnable("/Volumes/EastSea/EastSea.app", readOnly: true), "a read-only mounted DMG is not runnable (path and flag agree)")
check(!runnable("/Applications/EastSea.app", readOnly: true), "even under /Applications, a read-only volume is not runnable")

// App Translocation (a quarantined copy macOS runs from a randomized
// read-only shadow): never runnable, whatever the volume flag says.
check(!runnable("/private/var/folders/ab/C1d2Ef3g/AppTranslocation/1234567890abcdef/d/EastSea.app"), "a translocated path is not runnable")
check(!runnable("/private/var/folders/ab/C1d2Ef3g/AppTranslocation/1234567890abcdef/d/EastSea.app", readOnly: true), "a translocated path on a read-only volume: still not runnable")

// Developer builds are exempt from the nag: we build and run the app from
// these places ourselves (the explicit rule, so a dev preview never shows the
// move sentence).
check(runnable(home + "/Library/Developer/Xcode/DerivedData/EastSea-abcd/Build/Products/Debug/EastSea.app"), "a DerivedData build is runnable")
check(runnable("/Volumes/workspace/aether-node/build/EastSea.app"), "a repository build/ output is runnable")

// Boundary: "/Applications-Backup" is not "/Applications", and a home
// Applications path that only shares a prefix with another user's does not
// count either.
check(!runnable("/Applications-Backup/EastSea.app"), "a prefix look-alike is not /Applications")
check(!runnable("/Applications Backup/EastSea.app"), "a space look-alike is not /Applications")
check(!runnable("/Users/tester2/Applications/EastSea.app"), "another user's Applications is not this home's")
check(runnable("/Applications"), "the container itself counts")

// A tilde form of the home path standardizes to the real home before the
// rule runs (the test process's own home, so compare against that one).
let realHomeApps = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Applications").path
check(InstallLocation.isRunnableLocation(bundlePath: "~/Applications/EastSea.app", homeApplicationsPath: realHomeApps, volumeIsReadOnly: false),
      "a tilde path standardizes to the real home's Applications")
check(!InstallLocation.isRunnableLocation(bundlePath: "~/Downloads/EastSea.app", homeApplicationsPath: realHomeApps, volumeIsReadOnly: false),
      "a tilde path outside Applications is still not runnable")

// The sentence (layer 4): one plain line, in the app's language, naming the
// app — and no path or jargon in it.
check(InstallLocation.moveSentence.contains(Brand.name),
      "the move sentence names the app")
check(!InstallLocation.moveSentence.contains("/"), "the move sentence has no paths")
check(!InstallLocation.moveSentence.isEmpty, "the move sentence says something")

check(InstallLocation.moveSentence(locale: walletTestLocale("en"), bundle: walletTestBundle("en")) == "Move EastSea to your Applications folder to run it.", "the reviewed English install sentence")
check(InstallLocation.moveSentence(locale: walletTestLocale("ko"), bundle: walletTestBundle("ko")) == "동해를 응용 프로그램 폴더로 옮긴 뒤 실행해 주세요.", "the reviewed Korean install sentence")
check(InstallLocation.moveSentence(locale: walletTestLocale("ja"), bundle: walletTestBundle("ja")) == "EastSeaをアプリケーションフォルダに移してから起動してください。", "the Japanese install sentence")
print("install-location: all checks passed")
