import SwiftUI

/// Width-driven layout: the same views serve an iPhone, a narrow Mac window and a
/// wide one. A container measures its width and tells the views inside whether to
/// use their narrow (stacked) arrangement.
private struct NarrowLayoutKey: EnvironmentKey {
    #if os(macOS)
    static let defaultValue = false
    #else
    static let defaultValue = true
    #endif
}

extension EnvironmentValues {
    /// True when the surrounding container is about phone-wide.
    var narrowLayout: Bool {
        get { self[NarrowLayoutKey.self] }
        set { self[NarrowLayoutKey.self] = newValue }
    }
}

/// Below this width, pages stack side-by-side content vertically.
enum LayoutWidth {
    static let narrow: CGFloat = 560
    /// Below this window width the Mac sidebar collapses into a toolbar picker.
    static let compactWindow: CGFloat = 680
}

private struct NarrowLayoutReader: ViewModifier {
    let threshold: CGFloat
    @State private var narrow = NarrowLayoutKey.defaultValue

    func body(content: Content) -> some View {
        content
            .environment(\.narrowLayout, narrow)
            .onGeometryChange(for: Bool.self) { $0.size.width < threshold } action: { narrow = $0 }
    }
}

extension View {
    /// Measure this view's width and set `narrowLayout` for everything inside it.
    func measuringNarrowLayout(below threshold: CGFloat = LayoutWidth.narrow) -> some View {
        modifier(NarrowLayoutReader(threshold: threshold))
    }

    /// A minimum size that only applies on macOS (sheets and windows); on iPhone
    /// the screen decides and the content wraps.
    @ViewBuilder func macMinSize(width: CGFloat? = nil, height: CGFloat? = nil) -> some View {
        #if os(macOS)
        frame(minWidth: width, minHeight: height)
        #else
        self
        #endif
    }
}

/// Stacks horizontally when there is room, vertically when narrow.
struct AdaptiveStack<Content: View>: View {
    @Environment(\.narrowLayout) private var narrow
    var spacing: CGFloat = 12
    @ViewBuilder var content: () -> Content

    var body: some View {
        if narrow {
            VStack(alignment: .leading, spacing: spacing, content: content)
        } else {
            HStack(alignment: .top, spacing: spacing, content: content)
        }
    }
}

extension View {
    /// Sheets scroll on iPhone (small screens, keyboard); on the Mac they size to fit.
    @ViewBuilder func sheetScroll() -> some View {
        #if os(macOS)
        self
        #else
        ScrollView { self }.scrollBounceBehavior(.basedOnSize)
        #endif
    }
}

extension View {
    /// Keeps `visible` in step with whether this view is scrolled into view
    /// (macOS 15 / iOS 18; earlier systems only track appear and disappear).
    @ViewBuilder func trackingScrollVisibility(_ visible: Binding<Bool>) -> some View {
        if #available(macOS 15.0, iOS 18.0, *) {
            onScrollVisibilityChange(threshold: 0.05) { visible.wrappedValue = $0 }
        } else {
            self
        }
    }
}
