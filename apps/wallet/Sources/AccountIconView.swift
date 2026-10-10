import SwiftUI
#if os(macOS)
import AppKit
#endif

/// Decorative beside the authoritative address text. A single coastline is
/// visible below 32 px; larger icons add two neighboring islands.
struct AccountIcon: View {
    let spec: AccountIconSpec?
    var size: CGFloat = 32

    init(spec: AccountIconSpec?, size: CGFloat = 32) {
        self.spec = spec
        self.size = size
    }

    init(address: String?, version: UInt8 = 3, size: CGFloat = 32) {
        self.init(spec: AccountIconSpec.of(address: address, version: version), size: size)
    }

    var body: some View {
        Canvas { context, canvasSize in
            context.scaleBy(x: canvasSize.width / 64, y: canvasSize.height / 64)
            let bounds = CGRect(x: 0, y: 0, width: 64, height: 64)
            let background = RoundedRectangle(cornerRadius: 12, style: .circular).path(in: bounds)
            if let spec {
                context.fill(background, with: .linearGradient(
                    Gradient(colors: [Self.color(spec.colors.start), Self.color(spec.colors.end)]),
                    startPoint: .zero, endPoint: CGPoint(x: 64, y: 64)))
                context.translateBy(x: 32, y: 32)
                context.rotate(by: .degrees(Double(spec.rotation) * 90))
                context.translateBy(x: -32, y: -32)
                let ink = GraphicsContext.Shading.color(Self.color(spec.colors.ink))
                var mainContext = context
                mainContext.translateBy(x: 0, y: size < 32 ? 5 : 0)
                mainContext.scaleBy(x: 1, y: 0.8)
                mainContext.fill(Self.mainPaths[Int(spec.silhouetteClass)], with: ink)
                if size >= 32 {
                    for island in spec.satellitePaths { context.fill(Self.path(island), with: ink) }
                }
            } else {
                context.fill(background, with: .color(Self.color("#808890")))
            }
        }
        .frame(width: size, height: size)
        .accessibilityHidden(true)
    }

    private static func color(_ hex: String) -> Color {
        let value = UInt32(hex.dropFirst(), radix: 16)!
        return Color(.sRGB, red: Double((value >> 16) & 255) / 255,
                     green: Double((value >> 8) & 255) / 255,
                     blue: Double(value & 255) / 255, opacity: 1)
    }

    private static let mainPaths = AccountIconSpec.silhouettes.map { path($0.path) }

    /// The frozen cross-language geometry uses only integer M/L/C/Z commands.
    private static func path(_ source: String) -> Path {
        let tokens = source.split(separator: " ")
        var index = 0
        func point() -> CGPoint {
            let x = CGFloat(Int(tokens[index])!)
            let y = CGFloat(Int(tokens[index + 1])!)
            index += 2
            return CGPoint(x: x, y: y)
        }
        var path = Path()
        while index < tokens.count {
            let command = tokens[index]
            index += 1
            switch command {
            case "M": path.move(to: point())
            case "L": path.addLine(to: point())
            case "C":
                let first = point(), second = point(), end = point()
                path.addCurve(to: end, control1: first, control2: second)
            case "Z": path.closeSubpath()
            default: preconditionFailure("Invalid account icon geometry")
            }
        }
        return path
    }
}

#if os(macOS)
extension AccountIcon {
    /// MenuBarExtra's image label consumes an NSImage reliably. Rasterize the
    /// same view at Retina scale with a 16-point logical size, without storing
    /// an avatar or reading any account state. Other surfaces keep the Canvas.
    @MainActor
    func menuBarImage(scale: CGFloat = 2) -> NSImage? {
        let renderer = ImageRenderer(content: self)
        renderer.scale = scale
        guard let image = renderer.cgImage else { return nil }
        return NSImage(cgImage: image, size: NSSize(width: size, height: size))
    }
}
#endif
