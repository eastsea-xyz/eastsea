import SwiftUI
#if os(macOS)
import AppKit
#endif

/// Decorative beside the authoritative address text. Geometry, ink and fill
/// are identical in both appearances, at 16 px and at larger review sizes.
struct AccountIcon: View {
    let spec: AccountIconSpec?
    var size: CGFloat = 32

    init(spec: AccountIconSpec?, size: CGFloat = 32) {
        self.spec = spec
        self.size = size
    }

    init(address: String?, version: UInt8 = 1, size: CGFloat = 32) {
        self.init(spec: AccountIconSpec.of(address: address, version: version), size: size)
    }

    var body: some View {
        Canvas { context, canvasSize in
            context.scaleBy(x: canvasSize.width / 64, y: canvasSize.height / 64)
            let bounds = CGRect(x: 0, y: 0, width: 64, height: 64)
            context.fill(RoundedRectangle(cornerRadius: 12, style: .circular).path(in: bounds),
                         with: .color(Self.color(spec?.paletteHex ?? "#8b8b8b")))
            if let spec {
                context.translateBy(x: 32, y: 32)
                context.rotate(by: .degrees(Double(spec.rotation) * 90))
                context.translateBy(x: -32, y: -32)
                for cell in spec.occupiedCells {
                    context.fill(Self.island(cell: cell, shape: spec.shape),
                                 with: .color(Self.color(AccountIconSpec.ink)))
                }
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

    private static func island(cell: Int, shape: UInt8) -> Path {
        let x = CGFloat(9 + 12 * (cell % 4))
        let y = CGFloat(9 + 12 * (cell / 4))
        let bounds = CGRect(x: x, y: y, width: 10, height: 10)
        switch shape {
        case 0: return Path(bounds)
        case 1: return Path(ellipseIn: bounds)
        case 2:
            return Path { path in
                path.move(to: CGPoint(x: x + 5, y: y))
                path.addLine(to: CGPoint(x: x + 10, y: y + 10))
                path.addLine(to: CGPoint(x: x, y: y + 10))
                path.closeSubpath()
            }
        default:
            return Path { path in
                path.move(to: CGPoint(x: x, y: y))
                path.addLine(to: CGPoint(x: x + 10, y: y))
                path.addArc(center: CGPoint(x: x, y: y), radius: 10,
                            startAngle: .degrees(0), endAngle: .degrees(90), clockwise: false)
                path.closeSubpath()
            }
        }
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
