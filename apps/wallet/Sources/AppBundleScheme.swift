import Foundation
import WebKit

/// This handler has no filesystem or network access. An app receives only the
/// immutable assets from its completely verified bundle (or a gated developer
/// snapshot). Each web view registers its own handler and app authority.
final class AppBundleScheme: NSObject, WKURLSchemeHandler {
    let appKey: String
    let bundle: AppBundle

    init(bundle: AppBundle, appKey: String, allowUnverifiedDeveloperContent: Bool = false) throws {
        guard AppBundlePath.isAppKey(appKey) else { throw AppContentError.invalidResponse }
        guard bundle.isVerified || allowUnverifiedDeveloperContent else { throw AppContentError.unverifiedContent }
        self.appKey = appKey
        self.bundle = bundle
        super.init()
    }

    var entryURL: URL { URL(string: "\(AppBundlePath.scheme)://\(appKey)/\(bundle.entryPoint)")! }

    func webView(_ webView: WKWebView, start task: WKURLSchemeTask) {
        guard let url = task.request.url,
              task.request.httpMethod == nil || task.request.httpMethod == "GET" || task.request.httpMethod == "HEAD",
              let path = AppBundlePath.requestPath(url, appKey: appKey, entryPoint: bundle.entryPoint),
              let original = bundle.files[path], let mime = AppBundlePath.mimeType(for: path) else {
            task.didFailWithError(AppContentError.invalidResponse)
            return
        }
        do {
            let data = try AppBundlePath.renderData(original, path: path)
            guard let response = HTTPURLResponse(url: url, statusCode: 200, httpVersion: "HTTP/1.1", headerFields: [
                "Content-Type": mime,
                "Content-Length": String(data.count),
                "Content-Security-Policy": AppBundlePath.contentSecurityPolicy,
                "X-Content-Type-Options": "nosniff",
                "Cache-Control": "no-store",
                "Referrer-Policy": "no-referrer",
            ]) else { throw AppContentError.invalidResponse }
            task.didReceive(response)
            if task.request.httpMethod != "HEAD" { task.didReceive(data) }
            task.didFinish()
        } catch {
            task.didFailWithError(error)
        }
    }

    func webView(_ webView: WKWebView, stop urlSchemeTask: WKURLSchemeTask) {
        // All responses are synchronous snapshots; nothing remains in flight.
    }
}
