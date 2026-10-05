// Checks the Explore tab's origin rules without an app or a node (mirrors
// the extension's safety.js look-alike checks, over curated domains):
//   swiftc -o ./tmp/browser-origin-check apps/wallet/Sources/BrowserOriginPolicy.swift apps/wallet/Tests/browser-origin/main.swift && ./tmp/browser-origin-check
import Foundation
func check(_ c: Bool, _ m: String) { if !c { print("FAIL", m); exit(1) } }

func classify(_ s: String) -> BrowserOriginPolicy.Classification {
    BrowserOriginPolicy.classify(URL(string: s)!)
}

// What may load: bundled pages and https, nothing else.
check(classify("eastsea-page://explorer/index.html") == .bundled, "bundled scheme")
check(classify("eastsea-page://explorer/js/app.js") == .bundled, "bundled asset")
check(classify("https://dapp.test/swap?x=1") == .external(host: "dapp.test", lookalike: nil, punycode: false), "plain https")
check(classify("HTTPS://UPPER.test") == .external(host: "UPPER.test", lookalike: nil, punycode: false), "scheme case")
if case .blocked(let why) = classify("HTTP://UPPER.test") { check(why.contains("http"), "upper-case http refused") } else { check(false, "upper-case http refused") }

// Refusals, each with a sentence for under the address bar.
if case .blocked = classify("http://neverssl.com") { } else { check(false, "http refused") }
if case .blocked(let why) = classify("http://neverssl.com") { check(why.contains("http"), "http reason") } else { check(false, "http reason") }
if case .blocked = classify("file:///etc/passwd") { } else { check(false, "file refused") }
if case .blocked = classify("about:blank") { } else { check(false, "about refused") }
if case .blocked = classify("ftp://files.test") { } else { check(false, "ftp refused") }
if case .blocked = classify("javascript:alert(1)") { } else { check(false, "javascript refused") }

// The phishing guard: look-alikes of the curated domains are flagged, the
// domains themselves and their subdomains are not.
check(classify("https://eastsea.xyz") == .external(host: "eastsea.xyz", lookalike: nil, punycode: false), "curated itself")
check(classify("https://app.eastsea.xyz") == .external(host: "app.eastsea.xyz", lookalike: nil, punycode: false), "curated subdomain")
check(classify("https://eeastsea.xyz/airdrop") == .external(host: "eeastsea.xyz", lookalike: "eastsea.xyz", punycode: false), "typo domain")
check(classify("https://eastsea.xyz.evil.com") == .external(host: "eastsea.xyz.evil.com", lookalike: "eastsea.xyz", punycode: false), "suffix abuse")
check(classify("https://xn--estsea-3ua.xyz") == .external(host: "xn--estsea-3ua.xyz", lookalike: nil, punycode: true), "punycode flagged")
check(classify("https://dapp.xn--80ak6aa92e.com") == .external(host: "dapp.xn--80ak6aa92e.com", lookalike: nil, punycode: true), "punycode subdomain flagged")

// The pieces the guard is built from (safety.js's folds and distances).
check(BrowserOriginPolicy.normalized("EastSEA.xyz") == "eastseaxyz", "normalized folds case")
check(BrowserOriginPolicy.normalized("Æastsea") == "aeastsea", "accent folded (Æ becomes AE)")
check(BrowserOriginPolicy.editDistance("eastsea", "eastsae") == 2, "transposition is two edits")
check(BrowserOriginPolicy.editDistance("eastsea", "eastseas") == 1, "one insertion")
check(BrowserOriginPolicy.resembles("eeastseaxyz", "eastsea.xyz"), "one edit away")
check(BrowserOriginPolicy.resembles("eastsea.xyz.evil.com", "eastsea.xyz"), "contains the domain")
check(!BrowserOriginPolicy.resembles("dapp.test", "eastsea.xyz"), "unrelated host")
check(BrowserOriginPolicy.isCurated("EASTSEA.XYZ"), "curated case-insensitive")
check(BrowserOriginPolicy.isCurated("names.eastsea.xyz"), "curated subdomain")
check(!BrowserOriginPolicy.isCurated("eastsea.xyz.evil.com"), "suffix abuse is not curated")

// Bundled pages: traversal and odd files are refused, html/js/css/json are served.
check(BundledPagePath.safePath(dir: "explorer", path: "/index.html") == "explorer/index.html", "bundled index")
check(BundledPagePath.safePath(dir: "explorer", path: "/") == "explorer/index.html", "bundled root defaults to index")
check(BundledPagePath.safePath(dir: "explorer", path: "/js/app.js") == "explorer/js/app.js", "bundled asset")
check(BundledPagePath.safePath(dir: "explorer", path: "/js/../index.html") == "explorer/index.html", "inner .. folds")
check(BundledPagePath.safePath(dir: "explorer", path: "/../ecrets") == nil, "escape refused")
check(BundledPagePath.safePath(dir: "explorer", path: "/a/../../x") == nil, "deep escape refused")
check(BundledPagePath.safePath(dir: "", path: "/index.html") == nil, "no dir refused")
check(BundledPagePath.mimeType(for: "a/b.html") == "text/html; charset=utf-8", "html mime")
check(BundledPagePath.mimeType(for: "app.js") == "text/javascript; charset=utf-8", "js mime")
check(BundledPagePath.mimeType(for: "token-sources.json") == "application/json; charset=utf-8", "json mime")
check(BundledPagePath.mimeType(for: "binary.dat") == nil, "unknown mime refused")
check(BundledPagePath.contentSecurityPolicy.contains("connect-src 'self' http://127.0.0.1:18545"), "csp names the node")

// Permission keys: one origin per scheme://host[:port], default port folded away.
check(BrowserOriginPolicy.permissionKey(scheme: "https", host: "DApp.test", port: 0) == "https://dapp.test", "no port")
check(BrowserOriginPolicy.permissionKey(scheme: "https", host: "dapp.test", port: 443) == "https://dapp.test", "default port folded")
check(BrowserOriginPolicy.permissionKey(scheme: "https", host: "dapp.test", port: 8443) == "https://dapp.test:8443", "custom port kept")
check(BrowserOriginPolicy.permissionKey(scheme: "http", host: "127.0.0.1", port: 18545) == "http://127.0.0.1:18545", "loopback port kept")

print("browser-origin OK")
