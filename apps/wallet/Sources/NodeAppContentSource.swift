import Foundation

/// Implements the sea-names lane's ContentSource seam. Its immutable snapshot
/// is also the only source of bytes for the running private-scheme handler.
struct NodeAppContentSource: ContentSource {
    let app: SeaAppRecord
    let bundle: AppBundle

    init(app: SeaAppRecord, endpoint: URL) async throws {
        self.app = app
        bundle = try await AppBundleRPCClient(endpoint: endpoint).load(bundleHash: app.bundleHash)
    }

    func page(for app: SeaAppRecord, at link: SeaURL.NameLink) async throws -> ContentPage? {
        guard self.app == app, bundle.isVerified,
              AppBundleHash.normalized(app.bundleHash) == bundle.bundleHash else { throw AppContentError.unverifiedContent }
        let path: String
        if link.path.isEmpty || link.path == "/" {
            path = bundle.entryPoint
        } else {
            guard link.path.hasPrefix("/") else { throw AppContentError.invalidPath(link.path) }
            path = String(link.path.dropFirst())
        }
        guard AppBundlePath.isSafe(path), let bytes = bundle.files[path],
              let mime = AppBundlePath.mimeType(for: path) else { throw AppContentError.missingFile(path) }
        guard mime == "text/html; charset=utf-8" else { throw AppContentError.unsupportedFile(path) }
        return ContentPage(bytes: bytes, mimeType: mime)
    }
}
