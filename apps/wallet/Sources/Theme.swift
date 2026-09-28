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

extension Color {
    /// Warning text and dots. System orange is 2.6:1 on white, so light mode gets
    /// a dark amber (4.7:1) and dark mode a bright one — both over 4.5:1 on the
    /// card backgrounds (docs/research/design-critique-2026-09.md).
    static let warn: Color = {
        #if os(macOS)
        Color(NSColor(name: nil, dynamicProvider: { appearance in
            appearance.bestMatch(from: [.darkAqua, .vibrantDark]) == nil
                ? NSColor(red: 0.68, green: 0.38, blue: 0.0, alpha: 1)
                : NSColor(red: 1.0, green: 0.72, blue: 0.34, alpha: 1)
        }))
        #else
        Color(UIColor { trait in
            trait.userInterfaceStyle == .dark
                ? UIColor(red: 1.0, green: 0.72, blue: 0.34, alpha: 1)
                : UIColor(red: 0.68, green: 0.38, blue: 0.0, alpha: 1)
        })
        #endif
    }()
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
