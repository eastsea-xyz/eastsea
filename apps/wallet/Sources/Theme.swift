import SwiftUI

// Design tokens (docs/research/design-benchmark-2026.md §6): six type sizes, one card
// radius, and colors that each mean one thing. Green is for money coming in only.

extension Font {
    /// The Home balance: the largest thing on any screen.
    #if os(macOS)
    static let display = Font.system(size: 56, weight: .semibold, design: .rounded)
    #else
    static let display = Font.system(size: 48, weight: .semibold, design: .rounded)
    #endif
    /// Page and sheet titles, headline numbers.
    #if os(macOS)
    static let aeTitle = Font.system(size: 22, weight: .semibold)
    #else
    static let aeTitle = Font.title2.weight(.semibold)
    #endif
    /// Card titles.
    static let aeHeadline = Font.headline
    /// Body text (at most two lines on a card).
    #if os(macOS)
    static let aeBody = Font.body
    static let aeFootnote = Font.callout
    static let aeCaption = Font.caption
    #else
    static let aeBody = Font.subheadline
    static let aeFootnote = Font.footnote
    static let aeCaption = Font.caption2
    #endif
}

extension Font {
    /// The earnings number: loud, but never larger than the balance above it.
    static let heroNumber = Font.system(size: 48, weight: .heavy, design: .rounded)
    static let heroNumberNarrow = Font.system(size: 40, weight: .heavy, design: .rounded)
}

/// Inside every card, the same padding.
enum CardPadding {
    static let narrow: CGFloat = 16
    static let wide: CGFloat = 20
}

enum Radius {
    /// Every card-shaped surface.
    static let card: CGFloat = 16
    /// Tiles and buttons nested inside a card.
    static let inner: CGFloat = 12
}
