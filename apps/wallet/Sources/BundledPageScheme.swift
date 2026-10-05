import Foundation
import WebKit

/// Serves the explorer shipped inside the app under a private scheme
/// (`eastsea-page://explorer/…`) so it never touches file:// — the app bundle
/// path is not exposed, traversal is refused (BundledPagePath), and every
/// answer carries a content-security policy that keeps the page from
/// reaching anywhere but itself and this Mac's node (docs/design/09-wallet.md
/// "인앱 브라우저").
final class BundledPageScheme: NSObject, WKURLSchemeHandler {
    /// Where bundled pages live (the build script copies apps/explorer here).
    private let root: URL

    init(root: URL) {
        self.root = root
        super.init()
    }

    /// The default root: the Explorer folder inside the app's resources.
    static func defaultRoot() -> URL? {
        Bundle.main.resourceURL?.appendingPathComponent("Explorer", isDirectory: true)
    }

    // MARK: WKURLSchemeHandler

    func webView(_ webView: WKWebView, start task: WKURLSchemeTask) {
        let url = task.request.url
        guard let url, url.scheme?.lowercased() == BrowserOriginPolicy.bundledScheme,
              let rel = BundledPagePath.safePath(dir: url.host ?? "", path: url.path),
              let mime = BundledPagePath.mimeType(for: rel) else {
            task.didFailWithError(NSError(domain: NSURLErrorDomain, code: NSURLErrorBadURL,
                                          userInfo: [NSLocalizedDescriptionKey: "This page is not part of the app."]))
            return
        }
        let file = root.appendingPathComponent(rel)
        guard let data = try? Data(contentsOf: file) else {
            task.didFailWithError(NSError(domain: NSURLErrorDomain, code: NSURLErrorFileDoesNotExist,
                                          userInfo: [NSLocalizedDescriptionKey: "This page is not part of the app."]))
            return
        }
        let response = HTTPURLResponse(url: url, statusCode: 200, httpVersion: "HTTP/1.1",
                                       headerFields: [
                                        "Content-Type": mime,
                                        "Content-Security-Policy": BundledPagePath.contentSecurityPolicy,
                                        "X-Content-Type-Options": "nosniff",
                                        "Cache-Control": "no-cache",
                                       ])!
        task.didReceive(response)
        task.didReceive(data)
        task.didFinish()
    }

    func webView(_ webView: WKWebView, stop urlSchemeTask: WKURLSchemeTask) {
        // Nothing streams; a stopped task needs no cleanup.
    }
}
