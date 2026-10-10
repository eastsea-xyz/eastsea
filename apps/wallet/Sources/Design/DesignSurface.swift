#if canImport(SwiftUI)
import SwiftUI
import CoreImage.CIFilterBuiltins

/// Namespace keeps this phase's components separate from existing wallet views.
enum EastSeaDesign {}

/// Exact vector geometry from design/brand/dawn-flat.svg. No asset lookup or download.
struct EastSeaDawnMark: View {
    var body: some View {
        GeometryReader { geometry in
            let size = min(geometry.size.width, geometry.size.height)
            ZStack {
                Circle().fill(DesignTokens.Palette.gold.color).padding(size * 1.5 / 32)
                DawnFace().fill(DesignTokens.Palette.sea.colorForDawn)
                DawnRays().stroke(DesignTokens.Palette.sea.colorForDawn,
                                  style: StrokeStyle(lineWidth: size * 2.2 / 32, lineCap: .round))
            }
            .frame(width: size, height: size)
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        .accessibilityHidden(true)
    }
}

private extension DesignTokens.ColorPair {
    // The flat coin stays navy on gold in both themes, matching the sea art token.
    var colorForDawn: Color {
        Color(red: Double((light >> 16) & 255) / 255, green: Double((light >> 8) & 255) / 255, blue: Double(light & 255) / 255)
    }
}

private struct DawnFace: Shape {
    func path(in rect: CGRect) -> Path {
        var path = Path()
        path.move(to: CGPoint(x: 9, y: 17))
        path.addCurve(to: CGPoint(x: 23, y: 18.5), control1: CGPoint(x: 9.2, y: 8.5), control2: CGPoint(x: 22.8, y: 8.5))
        path.addCurve(to: CGPoint(x: 9, y: 17), control1: CGPoint(x: 18, y: 18.5), control2: CGPoint(x: 14.3, y: 14.8))
        path.closeSubpath()
        path.move(to: CGPoint(x: 5, y: 21))
        path.addCurve(to: CGPoint(x: 17, y: 19), control1: CGPoint(x: 8.8, y: 15.8), control2: CGPoint(x: 12.8, y: 17))
        path.addCurve(to: CGPoint(x: 27, y: 19), control1: CGPoint(x: 20.6, y: 20.7), control2: CGPoint(x: 23.7, y: 21.6))
        path.addCurve(to: CGPoint(x: 17.4, y: 23.1), control1: CGPoint(x: 25, y: 24.9), control2: CGPoint(x: 21.8, y: 25.2))
        path.addCurve(to: CGPoint(x: 5, y: 21), control1: CGPoint(x: 12.6, y: 20.9), control2: CGPoint(x: 9.6, y: 19.1))
        path.closeSubpath()
        path.move(to: CGPoint(x: 8.5, y: 26))
        path.addCurve(to: CGPoint(x: 17, y: 25.2), control1: CGPoint(x: 11.7, y: 23), control2: CGPoint(x: 14, y: 23.8))
        path.addCurve(to: CGPoint(x: 24, y: 24.8), control1: CGPoint(x: 19.4, y: 26.3), control2: CGPoint(x: 21.6, y: 26.2))
        path.addCurve(to: CGPoint(x: 15.4, y: 27.4), control1: CGPoint(x: 21.8, y: 28.8), control2: CGPoint(x: 19.4, y: 29.1))
        path.addCurve(to: CGPoint(x: 8.5, y: 26), control1: CGPoint(x: 12.9, y: 26.3), control2: CGPoint(x: 11.4, y: 25))
        path.closeSubpath()
        return path.applying(CGAffineTransform(scaleX: rect.width / 32, y: rect.height / 32))
    }
}

private struct DawnRays: Shape {
    func path(in rect: CGRect) -> Path {
        var path = Path()
        for points in [(CGPoint(x: 16, y: 5.8), CGPoint(x: 16, y: 8.4)),
                       (CGPoint(x: 8.6, y: 8.8), CGPoint(x: 10.4, y: 10.6)),
                       (CGPoint(x: 23.4, y: 8.8), CGPoint(x: 21.6, y: 10.6))] {
            path.move(to: points.0); path.addLine(to: points.1)
        }
        return path.applying(CGAffineTransform(scaleX: rect.width / 32, y: rect.height / 32))
    }
}

/// Opaque black-on-white code with a four-module quiet zone in either theme.
/// Rebuilds only when the supplied address changes; no background task remains.
struct EastSeaReceiveCode: View {
    let address: String
    let accessibilityText: String
    @State private var image: CGImage?

    var body: some View {
        Group {
            if let image {
                Image(decorative: image, scale: 1).resizable().interpolation(.none).scaledToFit()
            } else {
                Color.white
            }
        }
        .background(Color.white)
        .accessibilityLabel(accessibilityText)
        .task(id: address) { image = Self.render(address) }
    }

    static func render(_ address: String) -> CGImage? {
        guard !address.isEmpty else { return nil }
        let filter = CIFilter.qrCodeGenerator()
        filter.message = Data(address.utf8)
        filter.correctionLevel = "M"
        guard let code = filter.outputImage else { return nil }
        let extent = code.extent.insetBy(dx: -4, dy: -4)
        let paper = CIImage(color: .white).cropped(to: extent)
        let opaque = code.composited(over: paper).cropped(to: extent)
            .transformed(by: CGAffineTransform(scaleX: 8, y: 8))
        return CIContext().createCGImage(opaque, from: opaque.extent)
    }
}
#endif
