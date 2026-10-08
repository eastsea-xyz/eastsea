import Foundation
#if canImport(SwiftUI)
import CoreImage
#if os(macOS)
import AppKit
#endif
#endif

func check(_ condition: Bool, _ message: String) {
    if !condition { print("FAIL", message); exit(1) }
}

let full = DesignEffectPolicy(reduceMotion: false, reduceTransparency: false)
check(full.animatesBalance && full.animatesPresentation && full.showsTransientLight, "standard effects enabled")
check(full.usesCompositedDepth && full.emitsSuccessFeedback(enabled: true), "static depth and feedback enabled")
check(!full.emitsSuccessFeedback(enabled: false), "haptics preference honored")

let still = DesignEffectPolicy(reduceMotion: true, reduceTransparency: false)
check(!still.animatesBalance && !still.animatesPresentation && !still.showsTransientLight, "Reduce Motion stops every animation")
check(!still.emitsSuccessFeedback(enabled: true), "Reduce Motion stops celebratory haptics")
check(still.usesCompositedDepth, "Reduce Motion preserves static depth")

let opaque = DesignEffectPolicy(reduceMotion: false, reduceTransparency: true)
check(!opaque.showsTransientLight && !opaque.usesCompositedDepth, "Reduce Transparency removes glow and translucent depth")
check(opaque.animatesBalance && opaque.animatesPresentation, "opaque motion remains available")
check(opaque.emitsSuccessFeedback(enabled: true), "nonvisual haptics unaffected by Reduce Transparency")

let accessible = DesignEffectPolicy(reduceMotion: true, reduceTransparency: true)
check(!accessible.showsTransientLight && !accessible.usesCompositedDepth && !accessible.animatesBalance,
      "both accessibility settings compose")

let exact = Decimal(string: "123.000000000000000001")!
check(BalanceCountUpValue.sample(presentation: BalanceCountUpValue.presentationValue(exact), target: exact) == exact,
      "final balance keeps all Decimal digits")
let fractional = Decimal(string: "0.100000000000000001")!
check(BalanceCountUpValue.sample(presentation: 0.1, target: fractional) == fractional, "final fractional balance remains exact")
check(BalanceCountUpValue.sample(presentation: 2.5, target: Decimal(3)) == Decimal(string: "2.5")!,
      "intermediate presentation can be fractional")
check(BalanceCountUpValue.canAnimate(from: 0, to: 10), "incoming balance animates")
check(BalanceCountUpValue.canAnimate(from: 10, to: 0), "outgoing balance animates to zero")
check(BalanceCountUpValue.canAnimate(from: -2, to: 2), "intermediate value may cross zero")
check(!BalanceCountUpValue.canAnimate(from: 10, to: 10), "unchanged amount does no work")
check(!BalanceCountUpValue.canAnimate(from: fractional, to: Decimal(string: "0.100000000000000002")!),
      "sub-Double differences settle immediately")
check(!BalanceCountUpValue.canAnimate(from: 0, to: Decimal(string: "9007199254740993")!),
      "balances above the exact integer range settle immediately")
check(!BalanceCountUpValue.canAnimate(from: Decimal(string: "-9007199254740993")!, to: 0),
      "negative values below the exact integer range settle immediately")
check(BalanceCountUpValue.canAnimate(from: 0, to: Decimal(string: "9007199254740992")!),
      "exact integer boundary remains representable")
check(!BalanceCountUpValue.canAnimate(from: 0, to: Decimal(string: "1e120")!), "extreme Decimal values never interpolate")
check(!BalanceCountUpValue.canAnimate(from: .nan, to: 3), "invalid source does not animate")
check(!BalanceCountUpValue.canAnimate(from: 3, to: .nan), "invalid target does not animate")
check(BalanceCountUpValue.presentationValue(.nan) == 0, "invalid Decimal never enters SwiftUI animation data")
for value in [Double.nan, Double.infinity, -Double.infinity] {
    check(BalanceCountUpValue.sample(presentation: value, target: exact) == exact, "nonfinite samples settle to exact target")
}

for value in [-Double.infinity, Double.nan, Double.infinity] {
    let sample = TransientLightSample(progress: value)
    check(sample.progress == 1 && sample.opacity == 0, "invalid effect progress is invisible and settled")
}
check(TransientLightSample(progress: -1).progress == 0, "negative progress clamps")
check(TransientLightSample(progress: 2).progress == 1, "overshooting progress clamps")
check(TransientLightSample(progress: 0).opacity == 0 && TransientLightSample(progress: 1).opacity == 0,
      "both finite-effect endpoints are invisible")
check(TransientLightSample(progress: 0.5).opacity == 1, "one bounded light peak")
check(TransientLightSample(progress: 0).travel == -1 && TransientLightSample(progress: 1).travel == 1,
      "shine crosses once")
for step in 0...100 {
    let sample = TransientLightSample(progress: Double(step) / 100)
    check(sample.opacity >= 0 && sample.opacity <= 1 && sample.travel >= -1 && sample.travel <= 1,
          "effect envelope remains bounded")
}

check(EastSeaNodeStatus.connected.permitsPulse && EastSeaNodeStatus.checking.permitsPulse, "live node states allow an event pulse")
check(!EastSeaNodeStatus.paused.permitsPulse && !EastSeaNodeStatus.offline.permitsPulse,
      "inactive states never suggest live work")

check(DesignTokens.Palette.bg.light == 0xF4EFE6 && DesignTokens.Palette.bg.dark == 0x071320,
      "generated palette has intentional light and dark values")
check(DesignTokens.Motion.base == 0.24 && DesignTokens.Radius.plate == 18,
      "generated motion and shape values match system units")
check(DesignTokens.Shadows.plate.light.first?.opacity == 0.22,
      "plate shadow keeps its source opacity")
#if canImport(SwiftUI)
#if os(macOS)
for name in [NSAppearance.Name.aqua, .accessibilityHighContrastAqua] {
    if let appearance = NSAppearance(named: name) {
        check(DesignTokens.Palette.bg.hex(for: appearance) == DesignTokens.Palette.bg.light,
              "light and high-contrast-light appearances use the light palette")
    } else { check(false, "native light appearance is available") }
}
for name in [NSAppearance.Name.darkAqua, .accessibilityHighContrastDarkAqua] {
    if let appearance = NSAppearance(named: name) {
        check(DesignTokens.Palette.bg.hex(for: appearance) == DesignTokens.Palette.bg.dark,
              "dark and high-contrast-dark appearances use the dark palette")
    } else { check(false, "native dark appearance is available") }
}
#endif
let receiveAddress = "0x5397a1c0De4b1b8F6A3cB2d1E0f9C7a6B5d4E502"
check(EastSeaReceiveCode.render("") == nil, "empty address never presents a code")
if let code = EastSeaReceiveCode.render(receiveAddress) {
    check(code.width == code.height && code.width >= 37 * 8, "QR contains a full quiet zone")
    let detector = CIDetector(ofType: CIDetectorTypeQRCode, context: CIContext(), options: [CIDetectorAccuracy: CIDetectorAccuracyHigh])
    let features = detector?.features(in: CIImage(cgImage: code)) ?? []
    check((features.first as? CIQRCodeFeature)?.messageString == receiveAddress,
          "native receive QR decodes to the exact supplied address")
} else {
    check(false, "native QR rendering succeeds")
}
#endif

print("OK design-effects")
