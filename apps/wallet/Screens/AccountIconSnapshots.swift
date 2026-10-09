import AppKit
import Foundation
import SwiftUI

/// Static icon-only review. This executable never imports wallet state, node
/// code, accounts, defaults or signing keys. It renders the shared vectors.
@main
enum AccountIconSnapshots {
    struct Fixture: Decodable {
        struct Vector: Decodable { let address: String }
        let vectors: [Vector]
    }

    @MainActor
    static func main() throws {
        guard CommandLine.arguments.count == 3 else {
            fputs("usage: account-icon-snapshots VECTORS_JSON OUTPUT_DIRECTORY\n", stderr)
            exit(2)
        }
        NSApplication.shared.setActivationPolicy(.prohibited)
        let fixture = try JSONDecoder().decode(Fixture.self, from: Data(contentsOf:
            URL(fileURLWithPath: CommandLine.arguments[1])))
        let output = URL(fileURLWithPath: CommandLine.arguments[2], isDirectory: true)
        var written = 0
        for dark in [false, true] {
            let directory = output.appendingPathComponent(dark ? "dark" : "light", isDirectory: true)
            try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
            for (index, vector) in fixture.vectors.enumerated() {
                guard AccountIconSpec.of(address: vector.address) != nil else {
                    throw SnapshotError.invalidVector
                }
                for size in [16, 32, 64] {
                    let view = AccountIcon(address: vector.address, size: CGFloat(size))
                        .background(surface(dark: dark))
                        .environment(\.colorScheme, dark ? .dark : .light)
                    try write(view, to: directory.appendingPathComponent(String(format: "vector-%02d-%d.png", index, size)))
                    written += 1
                }
            }
            for size in [16, 32, 64] {
                try write(AccountIcon(address: nil, size: CGFloat(size)).background(surface(dark: dark)),
                          to: directory.appendingPathComponent("placeholder-\(size).png"))
                written += 1
            }
            guard let image = AccountIcon(address: fixture.vectors[5].address, size: 16).menuBarImage(scale: 1),
                  image.size == NSSize(width: 16, height: 16) else { throw SnapshotError.renderFailed }
            try write(Image(nsImage: image).renderingMode(.original).background(surface(dark: dark)),
                      to: directory.appendingPathComponent("menubar-image-16.png"))
            written += 1
            guard let retinaImage = AccountIcon(address: fixture.vectors[5].address, size: 16).menuBarImage(),
                  retinaImage.size == NSSize(width: 16, height: 16),
                  retinaImage.representations.first?.pixelsWide == 32,
                  retinaImage.representations.first?.pixelsHigh == 32 else { throw SnapshotError.renderFailed }
            try write(sheet(fixture.vectors, dark: dark),
                      to: output.appendingPathComponent(dark ? "swiftui-review-dark.png" : "swiftui-review-light.png"))
            written += 1
        }
        print("Rendered \(written) static SwiftUI PNGs (all 16 vectors at 16/32/64 px in light/dark, placeholders and sheets)")
    }

    private enum SnapshotError: Error { case invalidVector, renderFailed }

    private static func surface(dark: Bool) -> Color {
        dark ? Color(.sRGB, red: 16.0 / 255, green: 24.0 / 255, blue: 32.0 / 255, opacity: 1)
             : Color(.sRGB, red: 247.0 / 255, green: 245.0 / 255, blue: 240.0 / 255, opacity: 1)
    }

    @MainActor
    private static func sheet(_ vectors: [Fixture.Vector], dark: Bool) -> some View {
        VStack(alignment: .leading, spacing: 24) {
            Text(verbatim: "Archipelago v1 · SwiftUI · \(dark ? "dark" : "light")")
                .font(.system(size: 24, weight: .semibold))
            Text(verbatim: "Frozen address vectors · 16 / 32 / 64 px at natural size")
                .font(.system(size: 14)).foregroundStyle(.secondary)
            Grid(alignment: .leading, horizontalSpacing: 24, verticalSpacing: 24) {
                ForEach(0..<4) { row in
                    GridRow {
                        ForEach(0..<4) { column in
                            let index = row * 4 + column
                            let address = vectors[index].address
                            VStack(alignment: .leading, spacing: 10) {
                                Text(verbatim: String(format: "%02d", index))
                                    .font(.system(size: 13, weight: .semibold, design: .monospaced))
                                HStack(alignment: .center, spacing: 20) {
                                    AccountIcon(address: address, size: 16)
                                    AccountIcon(address: address, size: 32)
                                    AccountIcon(address: address, size: 64)
                                }
                                Text(verbatim: address).font(.system(size: 9, design: .monospaced))
                                    .fixedSize()
                            }
                            .frame(width: 270, height: 130, alignment: .topLeading)
                        }
                    }
                }
            }
            HStack(spacing: 16) {
                Text(verbatim: "Unseeded placeholder").font(.system(size: 13))
                AccountIcon(address: nil, size: 16)
                AccountIcon(address: nil, size: 32)
                AccountIcon(address: nil, size: 64)
            }
        }
        .padding(32)
        .background(surface(dark: dark))
        .environment(\.colorScheme, dark ? .dark : .light)
    }

    @MainActor
    private static func write<V: View>(_ view: V, to file: URL) throws {
        let renderer = ImageRenderer(content: view)
        renderer.scale = 1
        renderer.isOpaque = true
        guard let image = renderer.cgImage,
              let png = NSBitmapImageRep(cgImage: image).representation(using: .png, properties: [:]) else {
            throw SnapshotError.renderFailed
        }
        try png.write(to: file, options: .atomic)
    }
}
