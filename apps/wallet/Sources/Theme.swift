import SwiftUI

// Native aliases keep every existing screen on the generated EastSea system.
// Values belong in design/brand/tokens.json, including both appearances.

extension Font {
    static let display = DesignTokens.TypeScale.amountXl.font
    static let aeTitle = DesignTokens.TypeScale.title3.font
    static let aeHeadline = DesignTokens.TypeScale.headline.font
    static let aeBody = DesignTokens.TypeScale.bodyUi.font
    static let aeFootnote = DesignTokens.TypeScale.footnote.font
    static let aeCaption = DesignTokens.TypeScale.caption.font
}

extension Font {
    static let heroNumber = DesignTokens.TypeScale.amountLg.font
    static let heroNumberNarrow = DesignTokens.TypeScale.amountMd.font
}

extension Color {
    static let aether = DesignTokens.Palette.accent.color
    static let warn = DesignTokens.Palette.warn.color
}

/// Inside every card, the same padding.
enum CardPadding {
    static let narrow: CGFloat = DesignTokens.Space.s4
    static let wide: CGFloat = DesignTokens.Space.s5
}

enum Radius {
    /// Every card-shaped surface.
    static let card: CGFloat = DesignTokens.Radius.lg
    /// Tiles and buttons nested inside a card.
    static let inner: CGFloat = DesignTokens.Radius.md
}

extension View {
    func eastSeaPage() -> some View {
        font(.aeBody)
            .foregroundStyle(DesignTokens.Palette.text.color)
            .tint(DesignTokens.Palette.accent.color)
            .background(DesignTokens.Palette.bg.color)
    }

    func eastSeaSheet() -> some View {
        font(.aeBody)
            .foregroundStyle(DesignTokens.Palette.text.color)
            .tint(DesignTokens.Palette.accent.color)
            .background(DesignTokens.Palette.surface.color)
    }
}

struct EastSeaPrimaryButtonStyle: ButtonStyle {
    @Environment(\.isEnabled) private var isEnabled

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(DesignTokens.TypeScale.bodyUi.font.weight(.semibold))
            .padding(.horizontal, DesignTokens.Space.s4)
            .frame(minHeight: DesignTokens.Space.s10)
            .foregroundStyle(DesignTokens.Palette.onAccentFill.color)
            .background(configuration.role == .destructive
                        ? DesignTokens.Palette.danger.color
                        : DesignTokens.Palette.accentFill.color, in: Capsule())
            .opacity(isEnabled ? (configuration.isPressed ? 0.8 : 1) : 0.45)
    }
}

struct EastSeaQuietButtonStyle: ButtonStyle {
    @Environment(\.isEnabled) private var isEnabled

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(DesignTokens.TypeScale.bodyUi.font)
            .padding(.horizontal, DesignTokens.Space.s4)
            .frame(minHeight: DesignTokens.Space.s10)
            .foregroundStyle(configuration.role == .destructive
                             ? DesignTokens.Palette.danger.color
                             : DesignTokens.Palette.text.color)
            .background(DesignTokens.Palette.surfaceSunken.color, in: Capsule())
            .opacity(isEnabled ? (configuration.isPressed ? 0.7 : 1) : 0.45)
    }
}

struct EastSeaTextFieldStyle: TextFieldStyle {
    func _body(configuration: TextField<Self._Label>) -> some View {
        configuration
            .textFieldStyle(.plain)
            .font(DesignTokens.TypeScale.bodyUi.font)
            .padding(DesignTokens.Space.s3)
            .foregroundStyle(DesignTokens.Palette.text.color)
            .background(DesignTokens.Palette.surfaceSunken.color,
                        in: RoundedRectangle(cornerRadius: DesignTokens.Radius.sm))
            .overlay {
                RoundedRectangle(cornerRadius: DesignTokens.Radius.sm)
                    .stroke(DesignTokens.Palette.lineControl.color, lineWidth: 1)
            }
    }
}
