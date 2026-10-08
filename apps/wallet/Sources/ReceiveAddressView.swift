import CoreImage.CIFilterBuiltins
import SwiftUI

/// The same receive address and actions in the wallet sheet and menu bar.
struct ReceiveAddressView: View {
    let address: String
    var compact = false
    @State private var copied = false

    var body: some View {
        VStack(spacing: compact ? DesignTokens.Space.s3 : DesignTokens.Space.s4) {
            if address.isEmpty {
                Text("The wallet key is not ready yet.")
                    .font(.aeBody).foregroundStyle(DesignTokens.Palette.textMuted.color)
                    .fixedSize(horizontal: false, vertical: true)
            } else {
                EastSeaReceiveCode(address: address, accessibilityText: String(localized: "Receive address QR code"))
                    .frame(width: compact ? 192 : 200, height: compact ? 192 : 200)
                    .clipShape(RoundedRectangle(cornerRadius: DesignTokens.Radius.sm))
                    .accessibilityElement(children: .ignore)
                    .accessibilityLabel(Text("Receive address QR code"))
                Text(address)
                    .font(compact ? .aeCaption.monospaced() : .aeBody.monospaced())
                    .foregroundStyle(DesignTokens.Palette.text.color)
                    .multilineTextAlignment(.center)
                    .textSelection(.enabled)
                    .fixedSize(horizontal: false, vertical: true)
                    .frame(maxWidth: .infinity)
                    .padding(DesignTokens.Space.s3)
                    .background(DesignTokens.Palette.surfaceSunken.color,
                                in: RoundedRectangle(cornerRadius: DesignTokens.Radius.md))
            }
            HStack(spacing: DesignTokens.Space.s2) {
                Button {
                    Clipboard.copy(address)
                    copied = true
                } label: {
                    Label(copied ? String(localized: "Copied") : String(localized: "Copy address"),
                          systemImage: copied ? "checkmark" : "doc.on.doc")
                }
                .buttonStyle(EastSeaPrimaryButtonStyle())
                ShareLink(item: address) {
                    Label("Share", systemImage: "square.and.arrow.up")
                }
                .buttonStyle(EastSeaQuietButtonStyle())
            }
            .controlSize(compact ? .small : .regular)
            .disabled(address.isEmpty)
        }
        .frame(maxWidth: .infinity)
        .onChange(of: address) { _ in copied = false }
    }
}

/// A white quiet zone stays white in both appearances so phones can scan it.
struct QRCode: View {
    let text: String

    var body: some View {
        Group {
            if let img = Self.render(text) {
                Image(decorative: img, scale: 1)
                    .interpolation(.none).resizable().scaledToFit()
            } else {
                Image(systemName: "qrcode")
                    .resizable().scaledToFit().foregroundStyle(.tertiary)
            }
        }
        .background(Color.white)
        .clipShape(RoundedRectangle(cornerRadius: DesignTokens.Radius.sm))
    }

    static func render(_ s: String) -> CGImage? {
        guard !s.isEmpty else { return nil }
        let f = CIFilter.qrCodeGenerator()
        f.message = Data(s.utf8)
        f.correctionLevel = "M"
        guard let code = f.outputImage else { return nil }
        // Core Image's unscaled QR uses one pixel per module. Keep four
        // additional white modules around it before scaling any appearance.
        let quietZone = code.extent.insetBy(dx: -4, dy: -4)
        let white = CIImage(color: CIColor(red: 1, green: 1, blue: 1)).cropped(to: quietZone)
        let out = code.composited(over: white)
            .transformed(by: CGAffineTransform(scaleX: 8, y: 8))
        return CIContext().createCGImage(out, from: out.extent)
    }
}
