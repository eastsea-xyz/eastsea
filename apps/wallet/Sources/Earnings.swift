import SwiftUI
#if os(macOS)
import Combine
#endif

// "Your Mac is working, and this is what it earned": a loud hero card on the Network page
// (and on Home, under the balance, once a reward has arrived), a badge in the sidebar and a
// line in the menu-bar panel. Loud in looks only: every amount shown is
// a reward the chain actually paid (from `aether_rewards`), never a projection or a price.
//
// The drawing views take plain values, so they compile on iOS too (for the DEBUG preview
// harness); only the data source (`Earnings`) and the wrappers that read the node are macOS.

// MARK: - Values the views draw

/// What the node on this Mac is doing right now.
struct NodeWork: Equatable {
    enum Phase: Equatable {
        case starting
        /// Verifying every block (a node-only Mac: earns nothing yet on testnet).
        case verifying
        /// Proving blocks on the GPU: the first valid proof of a block is paid.
        case proving
        case paused(String)

        /// The live pill's word.
        var pillText: String {
            switch self {
            case .proving: String(localized: "PROVING")
            case .verifying: String(localized: "WORKING")
            case .starting: String(localized: "STARTING")
            case .paused: String(localized: "PAUSED")
            }
        }
    }

    var phase: Phase
    var height: UInt64 = 0
    /// Blocks this node has followed and checked since it started.
    var blocksVerified: UInt64 = 0
    var runningSince: Date?
    /// Proofs made on the GPU this session.
    var proofs: UInt64 = 0
    var proofsFailing = false
    /// Consecutive hours online as a registered voting node (nil: not registered).
    var streakHours: UInt64?
    var votingNow = false
    /// A wallet address exists, so proving can be switched on from the card.
    var canProve = true

    var isProving: Bool { phase == .proving }
    var isLive: Bool { phase == .proving || phase == .verifying }
}

/// A reward that just arrived (bumps `id` each time; drives the burst).
struct RewardCelebration: Equatable {
    let id: Int
    let amountWei: String
}

enum EarningsText {
    static var unit: String { String(localized: "test \(Brand.networkCoinTicker)") }

    static func aeth(_ wei: String) -> String { Wei.format(wei) }

    /// "3m ago", "2h ago".
    static func ago(_ date: Date, now: Date) -> String {
        let f = RelativeDateTimeFormatter()
        f.unitsStyle = .abbreviated
        return now.timeIntervalSince(date) < 45 ? String(localized: "just now") : f.localizedString(for: date, relativeTo: now)
    }

    /// "3h 12m".
    static func duration(_ seconds: TimeInterval) -> String {
        let f = DateComponentsFormatter()
        f.allowedUnits = seconds >= 3_600 ? [.day, .hour, .minute] : [.minute, .second]
        f.unitsStyle = .abbreviated
        f.maximumUnitCount = 2
        return f.string(from: max(0, seconds)) ?? "–"
    }

    /// Fraction digits for the count-up: as many as the amount needs, at most 4.
    static func decimals(_ wei: String) -> Int {
        let s = Wei.format(wei)
        guard let dot = s.firstIndex(of: ".") else { return 0 }
        return min(4, s.distance(from: dot, to: s.endIndex) - 1)
    }
}

// MARK: - Palette

/// Aether's violet into pink, plus the accents the celebration uses.
enum EarnInk {
    static let violet = Color(red: 0.49, green: 0.40, blue: 0.95)
    static let pink = Color(red: 1.00, green: 0.26, blue: 0.58)
    static let magenta = Color(red: 0.78, green: 0.22, blue: 0.86)
    static let sky = Color(red: 0.36, green: 0.62, blue: 1.00)
    static let night = Color(red: 0.16, green: 0.09, blue: 0.42)
    static let gold = Color(red: 1.00, green: 0.84, blue: 0.36)
    static let mint = Color(red: 0.40, green: 1.00, blue: 0.66)

    static let brand = LinearGradient(colors: [violet, pink], startPoint: .leading, endPoint: .trailing)
}

// MARK: - Hero card

/// The big earnings card while the node is on.
struct EarningsHero: View {
    let summary: EarningsSummary
    let work: NodeWork
    var celebration: RewardCelebration?
    /// Turns GPU proving on (nil hides the call to action).
    var onProve: (() -> Void)?
    @Environment(\.narrowLayout) private var narrow
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    /// On screen (scrolled into view); the aurora only moves while it is.
    @State private var visible = true

    /// Money is the headline once the Mac has been paid (never a row of zeros before).
    private var showsEarnings: Bool { summary.count > 0 }

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: Radius.card, style: .continuous)
        VStack(alignment: .leading, spacing: narrow ? 14 : 18) {
            header
            if showsEarnings { EarnedBlock(summary: summary, celebration: celebration) } else { VerifiedBlock(work: work) }
            tiles
            footer
        }
        .foregroundStyle(.white)
        .padding(narrow ? CardPadding.narrow : CardPadding.wide)
        .frame(maxWidth: .infinity, alignment: .leading)
        // One thing moves continuously per screen: the aurora while proving, else the live dot.
        .background { AuroraBackground(hot: showsEarnings, live: work.isProving && visible) }
        .overlay { ConfettiBurst(trigger: celebration?.id ?? 0) }
        .clipShape(shape)
        .overlay(shape.strokeBorder(.white.opacity(0.28), lineWidth: 1))
        .shadow(color: (showsEarnings ? EarnInk.pink : EarnInk.violet).opacity(0.30), radius: 10, y: 4)
        .keyframeAnimator(initialValue: 1.0, trigger: reduceMotion ? 0 : celebration?.id ?? 0) { view, s in
            view.scaleEffect(s)
        } keyframes: { _ in
            KeyframeTrack {
                SpringKeyframe(1.035, duration: 0.18)
                SpringKeyframe(1.0, duration: 0.5)
            }
        }
        .accessibilityElement(children: .combine)
        .onAppear { visible = true }
        .onDisappear { visible = false }
        .trackingScrollVisibility($visible)
    }

    /// The live indicator sits on the card's own color, on the padding grid;
    /// the coin (the shipped render, 48 pt and up) anchors the money side.
    private var header: some View {
        HStack(alignment: .center) {
            LivePill(text: pillText, live: work.isLive, beat: work.height, ring: work.isLive && !work.isProving)
            Spacer(minLength: 0)
            TokenIcon(chainId: Brand.networkChainId, address: nil, symbol: Brand.networkCoinTicker, size: 48)
        }
    }

    private var pillText: String {
        work.phase.pillText
    }

    @ViewBuilder private var tiles: some View {
        HStack(spacing: narrow ? 8 : 12) {
            if showsEarnings {
                if summary.todayWei != "0" {
                    StatTile(label: "Today", value: "+\(EarningsText.aeth(summary.todayWei))", unit: EarningsText.unit)
                }
                StatTile(label: "Received", value: "\(summary.count)", unit: summary.count == 1 ? String(localized: "reward") : String(localized: "rewards"))
                TimelineView(.periodic(from: .now, by: 30)) { tl in
                    StatTile(label: "Last reward", value: summary.lastRewardAt.map { EarningsText.ago($0, now: tl.date) } ?? String(localized: "none yet"),
                             unit: summary.latestHeight.map { String(localized: "in block #\(String($0))") } ?? String(localized: "keep proving"))
                }
            } else {
                TimelineView(.periodic(from: .now, by: 30)) { tl in
                    StatTile(label: "Online", value: work.runningSince.map { EarningsText.duration(tl.date.timeIntervalSince($0)) } ?? "–", unit: String(localized: "this session"))
                }
                StatTile(label: work.votingNow ? "Voting" : "Streak", value: streakValue,
                         unit: work.streakHours == nil ? String(localized: "not registered") : String(localized: "hours online"))
                StatTile(label: "Latest block", value: work.height > 0 ? "#\(String(work.height))" : "–", unit: String(localized: "checked here"))
            }
        }
        .fixedSize(horizontal: false, vertical: true)
    }

    private var streakValue: String {
        guard let s = work.streakHours else { return "–" }
        return work.votingNow ? String(localized: "\(s) h") : "\(min(s, VotingRules.minStreakEpochs))/\(VotingRules.minStreakEpochs)"
    }

    @ViewBuilder private var footer: some View {
        VStack(alignment: .leading, spacing: 10) {
            footerLine
            if !work.isProving, work.canProve, let onProve {
                ProveCallToAction(action: onProve)
            }
        }
    }

    private var footerLine: some View {
        HStack(alignment: .firstTextBaseline, spacing: 6) {
            Image(systemName: footerIcon)
            Text(footerText).fixedSize(horizontal: false, vertical: true)
        }
        .font(.aeBody.weight(.medium))
        .foregroundStyle(.white.opacity(0.92))
    }

    /// " · 57 proofs this session" (nothing before the first one).
    private var proofsText: String {
        work.proofs == 0 ? "" : " · " + String(localized: "\(work.proofs) proofs this session")
    }

    private var footerIcon: String {
        switch work.phase {
        case .proving: "cpu"
        case .verifying: "checkmark.shield.fill"
        case .starting: "arrow.down.circle"
        case .paused: "pause.circle"
        }
    }

    private var footerText: String {
        switch work.phase {
        case .proving where summary.count > 0:
            String(localized: "This Mac's GPU is proving blocks\(proofsText). Every amount here was paid on chain.")
        case .proving:
            String(localized: "This Mac's GPU is proving blocks\(proofsText). The first valid proof of a block gets a reward.")
        case .verifying:
            String(localized: "Your Mac checks every block itself. Checking alone earns no reward.")
        case .starting:
            work.height > 0 ? String(localized: "Catching up with the network · block #\(String(work.height))") : String(localized: "Starting the node…")
        case .paused(let why):
            why
        }
    }
}

/// "EARNED SO FAR  12.5 test DBLN  +1.5 in the last hour", counting up.
private struct EarnedBlock: View {
    let summary: EarningsSummary
    let celebration: RewardCelebration?
    @Environment(\.narrowLayout) private var narrow

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("Received so far").font(.aeFootnote.weight(.semibold)).foregroundStyle(.white.opacity(0.85))
            BigNumber(value: WeiMath.aeth(summary.totalWei), decimals: EarningsText.decimals(summary.totalWei),
                      unit: EarningsText.unit, glow: EarnInk.gold)
                // An overlay, so the label's width never shifts the number.
                .overlay(alignment: .topLeading) {
                    FloatingReward(celebration: celebration).fixedSize().offset(x: narrow ? 40 : 60, y: narrow ? -26 : -34)
                }
            if summary.lastHourWei != "0" { HourDelta(wei: summary.lastHourWei) }
        }
    }
}

/// Node-only: the work this Mac did, counting up block by block.
private struct VerifiedBlock: View {
    let work: NodeWork

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("Blocks verified this session").font(.aeFootnote.weight(.semibold)).foregroundStyle(.white.opacity(0.85))
            BigNumber(value: Double(work.blocksVerified), decimals: 0, unit: work.blocksVerified == 1 ? String(localized: "block") : String(localized: "blocks"), glow: EarnInk.sky)
        }
    }
}

/// The headline number: huge, rounded, glowing, and counting up to its value.
private struct BigNumber: View {
    let value: Double
    let decimals: Int
    let unit: String
    let glow: Color
    @Environment(\.narrowLayout) private var narrow
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var shown = 0.0

    var body: some View {
        ViewThatFits(in: .horizontal) {
            HStack(alignment: .firstTextBaseline, spacing: 10) { number; unitText }
            VStack(alignment: .leading, spacing: 0) { number; unitText }
        }
        .onAppear { count(to: value) }
        .onChange(of: value) { _, v in count(to: v) }
    }

    private var number: some View {
        CountingText(value: shown, decimals: decimals)
            .font(narrow ? .heroNumberNarrow : .heroNumber)
            .monospacedDigit()
            .lineLimit(1)
            .minimumScaleFactor(0.5)
            .shadow(color: glow.opacity(0.75), radius: 14)
            .shadow(color: .black.opacity(0.18), radius: 2, y: 2)
    }

    private var unitText: some View {
        Text(unit).font(narrow ? .aeHeadline : .aeTitle).foregroundStyle(.white.opacity(0.9))
    }

    private func count(to v: Double) {
        guard !reduceMotion else { shown = v; return }
        withAnimation(.easeOut(duration: shown == 0 ? 1.6 : 0.9)) { shown = v }
    }
}

/// A number SwiftUI can animate digit by digit (a count-up, not a crossfade).
private struct CountingText: View, Animatable {
    var value: Double
    let decimals: Int

    var animatableData: Double {
        get { value }
        set { value = newValue }
    }

    var body: some View {
        Text(value, format: .number.precision(.fractionLength(decimals)).grouping(.automatic))
    }
}

private struct HourDelta: View {
    let wei: String

    var body: some View {
        let some = wei != "0"
        HStack(spacing: 6) {
            Image(systemName: some ? "plus.circle.fill" : "clock")
            Text(some ? String(localized: "+\(EarningsText.aeth(wei)) \(EarningsText.unit) in the last hour") : String(localized: "Nothing in the last hour"))
        }
        .font(.aeBody.weight(.bold))
        .foregroundStyle(some ? EarnInk.night : .white.opacity(0.9))
        .padding(.horizontal, 12).padding(.vertical, 6)
        .background(some ? AnyShapeStyle(EarnInk.mint) : AnyShapeStyle(.white.opacity(0.16)), in: Capsule())
    }
}

/// A frosted tile on the aurora.
private struct StatTile: View {
    let label: LocalizedStringKey
    let value: String
    let unit: String?
    @Environment(\.narrowLayout) private var narrow

    var body: some View {
        VStack(alignment: .leading, spacing: 3) {
            Text(label).font(.aeCaption.weight(.semibold)).foregroundStyle(.white.opacity(0.82))
                .lineLimit(1).minimumScaleFactor(0.7)
            Text(value).font(narrow ? .aeHeadline : .aeTitle).monospacedDigit()
                .lineLimit(1).minimumScaleFactor(0.55)
                .contentTransition(.numericText())
            if let unit {
                Text(unit).font(.aeCaption).foregroundStyle(.white.opacity(0.82)).lineLimit(1).minimumScaleFactor(0.7)
            }
        }
        .padding(.horizontal, narrow ? 10 : 14).padding(.vertical, narrow ? 9 : 12)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .background(.white.opacity(0.14), in: RoundedRectangle(cornerRadius: Radius.inner, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: Radius.inner, style: .continuous).strokeBorder(.white.opacity(0.18), lineWidth: 1))
    }
}

/// "● PROVING", with a ring that keeps pulsing and a heartbeat on every new block.
struct LivePill: View {
    let text: String
    let live: Bool
    let beat: UInt64
    /// The ever-pulsing ring (off while something else on screen already moves).
    var ring = true
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var ringOut = false

    var body: some View {
        HStack(spacing: 8) {
            ZStack {
                if live && ring && !reduceMotion {
                    Circle().stroke(EarnInk.mint, lineWidth: 2)
                        .scaleEffect(ringOut ? 2.8 : 1).opacity(ringOut ? 0 : 0.9)
                }
                Circle().fill(live ? EarnInk.mint : .orange).shadow(color: EarnInk.mint.opacity(live ? 0.9 : 0), radius: 5)
            }
            .frame(width: 9, height: 9)
            .keyframeAnimator(initialValue: 1.0, trigger: reduceMotion ? 0 : beat) { v, s in v.scaleEffect(s) } keyframes: { _ in
                KeyframeTrack {
                    SpringKeyframe(1.8, duration: 0.12)
                    SpringKeyframe(0.9, duration: 0.14)
                    SpringKeyframe(1.0, duration: 0.3)
                }
            }
            // Letter-spacing suits capitals, not Hangul.
            Text(text).font(.aeCaption.weight(.heavy)).tracking(AppLanguage.korean ? 0 : 1.5)
        }
        .padding(.horizontal, 12).padding(.vertical, 6)
        .background(.black.opacity(0.25), in: Capsule())
        .overlay(Capsule().strokeBorder(.white.opacity(0.3), lineWidth: 1))
        .onAppear { startRing() }
        .onChange(of: live) { _, _ in startRing() }
        .onChange(of: ring) { _, _ in startRing() }
    }

    private func startRing() {
        guard live, ring, !reduceMotion, !ringOut else { return }
        withAnimation(.easeOut(duration: 1.5).repeatForever(autoreverses: false)) { ringOut = true }
    }
}

/// The one thing a node-only Mac can do to start earning.
private struct ProveCallToAction: View {
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(spacing: 10) {
                Image(systemName: "bolt.fill").font(.aeHeadline)
                VStack(alignment: .leading, spacing: 1) {
                    Text("Prove blocks on this Mac's GPU").font(.aeHeadline)
                    Text("for test \(Brand.networkCoinTicker) rewards").font(.aeCaption.weight(.semibold)).opacity(0.8)
                }
                Spacer(minLength: 4)
                Image(systemName: "chevron.right").font(.aeHeadline)
            }
            .foregroundStyle(EarnInk.night)
            .padding(.horizontal, 16).padding(.vertical, 12)
            .background(.white, in: RoundedRectangle(cornerRadius: Radius.inner, style: .continuous))
            .shadow(color: .black.opacity(0.2), radius: 8, y: 4)
        }
        .buttonStyle(.plain)
        .help("Uses the GPU and power while on, at your cost. The first valid proof of a block gets a test \(Brand.networkCoinTicker) reward in this wallet.")
    }
}

/// "+0.5 test DBLN" that pops, rises and fades when a reward lands.
private struct FloatingReward: View {
    let celebration: RewardCelebration?
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    private struct Frame {
        var y: CGFloat = 0
        var opacity: Double = 0
        var scale: CGFloat = 0.6
    }

    var body: some View {
        Text("+\(EarningsText.aeth(celebration?.amountWei ?? "0")) \(EarningsText.unit)")
            .font(.aeTitle.weight(.black))
            .foregroundStyle(EarnInk.night)
            .padding(.horizontal, 12).padding(.vertical, 6)
            .background(LinearGradient(colors: [EarnInk.gold, .white], startPoint: .leading, endPoint: .trailing), in: Capsule())
            .shadow(color: EarnInk.gold.opacity(0.9), radius: 12)
            .keyframeAnimator(initialValue: Frame(), trigger: celebration?.id ?? 0) { view, f in
                view.scaleEffect(f.scale).offset(y: f.y).opacity(f.opacity)
            } keyframes: { _ in
                KeyframeTrack(\.opacity) {
                    LinearKeyframe(1, duration: 0.15)
                    LinearKeyframe(1, duration: 1.5)
                    LinearKeyframe(0, duration: 0.6)
                }
                KeyframeTrack(\.y) {
                    LinearKeyframe(0, duration: 0.1)
                    CubicKeyframe(reduceMotion ? 0 : -80, duration: 2.15)
                }
                KeyframeTrack(\.scale) {
                    SpringKeyframe(reduceMotion ? 1 : 1.25, duration: 0.2)
                    SpringKeyframe(1, duration: 0.4)
                    LinearKeyframe(1, duration: 1.65)
                }
            }
            .allowsHitTesting(false)
            .accessibilityHidden(true)
    }
}

// MARK: - Aurora

/// Drifting violet, pink and blue light under a sweeping sheen and a few twinkling
/// sparks. One Canvas at 30 fps; a still frame with Reduce Motion or while paused.
struct AuroraBackground: View {
    /// Earning: hotter pinks. Working: cooler blues.
    var hot: Bool
    var live: Bool
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    /// While the window is being resized the aurora holds its last frame instead of
    /// rasterizing its blurs onto every layout pass of the drag.
    @Environment(\.liveResize) private var resizing
    @State private var epoch = Date()

    var body: some View {
        let still = reduceMotion || !live
        TimelineView(.animation(minimumInterval: 1.0 / 30, paused: still || resizing)) { tl in
            Canvas { ctx, size in
                let t = still ? 3.0 : tl.date.timeIntervalSince(epoch)
                AuroraBackground.draw(in: &ctx, size: size, time: t, hot: hot)
            }
        }
        .accessibilityHidden(true)
    }

    static func draw(in ctx: inout GraphicsContext, size: CGSize, time t: Double, hot: Bool) {
        let w = size.width, h = size.height
        guard w > 2, h > 2 else { return }
        let base = hot ? [EarnInk.night, EarnInk.violet, EarnInk.pink] : [EarnInk.night, EarnInk.violet, EarnInk.sky]
        ctx.fill(Path(CGRect(origin: .zero, size: size)),
                 with: .linearGradient(Gradient(colors: base), startPoint: .zero, endPoint: CGPoint(x: w, y: h)))
        drawBlobs(in: &ctx, w: w, h: h, t: t, hot: hot)
        drawSheen(in: &ctx, w: w, h: h, t: t)
        drawSparks(in: &ctx, w: w, h: h, t: t)
    }

    private static func drawBlobs(in ctx: inout GraphicsContext, w: CGFloat, h: CGFloat, t: Double, hot: Bool) {
        let blobs: [(Color, Double, Double, Double)] = [  // color, alpha, speed, phase
            (hot ? EarnInk.pink : EarnInk.sky, 0.85, 0.23, 0.0),
            (EarnInk.magenta, 0.65, 0.17, 2.1),
            (hot ? EarnInk.gold : EarnInk.mint, hot ? 0.30 : 0.22, 0.13, 4.0),
            (EarnInk.sky, 0.55, 0.19, 5.2),
        ]
        ctx.drawLayer { g in
            g.addFilter(.blur(radius: min(w, h) * 0.28))
            for (color, alpha, speed, phase) in blobs {
                let x = w * (0.5 + 0.45 * sin(t * speed + phase))
                let y = h * (0.5 + 0.40 * cos(t * speed * 1.3 + phase * 0.7))
                let r = min(w, h) * (0.42 + 0.08 * sin(t * 0.5 + phase))
                g.fill(Path(ellipseIn: CGRect(x: x - r * 1.4, y: y - r, width: r * 2.8, height: r * 2)), with: .color(color.opacity(alpha)))
            }
        }
    }

    private static func drawSheen(in ctx: inout GraphicsContext, w: CGFloat, h: CGFloat, t: Double) {
        let cycle = 5.5
        let u = t.truncatingRemainder(dividingBy: cycle) / cycle
        guard u < 0.45 else { return }
        let f = u / 0.45
        let x = -w * 0.5 + w * 2 * f * f * (3 - 2 * f)
        let band = w * 0.18
        var p = Path()
        p.move(to: CGPoint(x: x, y: 0))
        p.addLine(to: CGPoint(x: x + band, y: 0))
        p.addLine(to: CGPoint(x: x + band - h * 0.5, y: h))
        p.addLine(to: CGPoint(x: x - h * 0.5, y: h))
        p.closeSubpath()
        ctx.fill(p, with: .linearGradient(Gradient(colors: [.white.opacity(0), .white.opacity(0.20), .white.opacity(0)]),
                                          startPoint: CGPoint(x: x - h * 0.25, y: 0), endPoint: CGPoint(x: x + band - h * 0.25, y: 0)))
    }

    private static func drawSparks(in ctx: inout GraphicsContext, w: CGFloat, h: CGFloat, t: Double) {
        var rng = SeededRandom(seed: 7)
        for _ in 0..<16 {
            let p = CGPoint(x: w * rng.next(), y: h * rng.next())
            let speed = 0.6 + rng.next() * 1.4, phase = rng.next() * 6.3
            let a = max(0, sin(t * speed + phase))
            let r = 1.0 + 2.2 * rng.next() * a
            Sparkle.draw(in: &ctx, at: p, radius: r * 2.2, color: .white.opacity(0.75 * a))
        }
    }
}

enum Sparkle {
    /// A four-pointed star.
    static func draw(in ctx: inout GraphicsContext, at c: CGPoint, radius r: CGFloat, color: Color, angle: Double = 0) {
        guard r > 0.3 else { return }
        var p = Path()
        for i in 0..<8 {
            let a = angle + Double(i) * .pi / 4
            let rr = i.isMultiple(of: 2) ? r : r * 0.28
            let pt = CGPoint(x: c.x + rr * cos(a), y: c.y + rr * sin(a))
            if i == 0 { p.move(to: pt) } else { p.addLine(to: pt) }
        }
        p.closeSubpath()
        ctx.fill(p, with: .color(color))
    }
}

/// Small deterministic generator (same sparks every frame, same burst per reward).
struct SeededRandom {
    private var state: UInt64
    init(seed: UInt64) { state = seed &* 0x9E37_79B9_7F4A_7C15 | 1 }
    mutating func next() -> Double {
        state ^= state << 13
        state ^= state >> 7
        state ^= state << 17
        return Double(state % 1_000_000) / 1_000_000
    }
}

// MARK: - Burst

/// Confetti, sparks and a shockwave for about two seconds after each new reward.
/// Draws nothing (and its timeline sleeps) the rest of the time; off with Reduce Motion.
struct ConfettiBurst: View {
    let trigger: Int
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.liveResize) private var resizing
    @State private var start: Date?
    static let duration = 2.4

    var body: some View {
        TimelineView(.animation(minimumInterval: 1.0 / 60, paused: start == nil || resizing)) { tl in
            Canvas { ctx, size in
                guard let start else { return }
                let t = tl.date.timeIntervalSince(start)
                guard t < Self.duration else { return }
                Self.draw(in: &ctx, size: size, time: t, seed: UInt64(trigger))
            }
        }
        .allowsHitTesting(false)
        .accessibilityHidden(true)
        .onChange(of: trigger) { _, id in
            guard id > 0, !reduceMotion else { return }
            let begun = Date()
            start = begun
            Task { @MainActor in
                try? await Task.sleep(for: .seconds(Self.duration))
                if start == begun { start = nil }
            }
        }
    }

    static func draw(in ctx: inout GraphicsContext, size: CGSize, time t: Double, seed: UInt64) {
        let origin = CGPoint(x: size.width * 0.32, y: size.height * 0.36)
        drawFlash(in: &ctx, size: size, origin: origin, t: t)
        let colors: [Color] = [.white, EarnInk.gold, EarnInk.mint, EarnInk.sky, EarnInk.pink, .white]
        var rng = SeededRandom(seed: seed &+ 11)
        for i in 0..<140 {
            let angle = -Double.pi / 2 + (rng.next() - 0.5) * 2.6
            let speed = 260 + rng.next() * 520
            let spin = (rng.next() - 0.5) * 14
            let life = 1.4 + rng.next() * 1.0
            guard t < life else { continue }
            let drag = (1 - exp(-2.2 * t)) / 2.2
            let x = origin.x + cos(angle) * speed * drag + (rng.next() - 0.5) * 30
            let y = origin.y + sin(angle) * speed * drag + 260 * t * t
            let fade = min(1, (life - t) / 0.5)
            let color = colors[i % colors.count].opacity(fade)
            drawPiece(in: &ctx, kind: i % 3, at: CGPoint(x: x, y: y), angle: spin * t, color: color, size: 4 + rng.next() * 5)
        }
    }

    private static func drawFlash(in ctx: inout GraphicsContext, size: CGSize, origin: CGPoint, t: Double) {
        if t < 0.5 {  // a white bloom
            let a = 0.45 * (1 - t / 0.5)
            let r = max(size.width, size.height)
            ctx.fill(Path(CGRect(origin: .zero, size: size)), with: .radialGradient(
                Gradient(colors: [.white.opacity(a), .clear]), center: origin, startRadius: 0, endRadius: r * 0.7))
        }
        if t < 0.8 {  // a ring racing outwards
            let f = t / 0.8
            let r = 20 + f * max(size.width, size.height) * 0.8
            ctx.stroke(Path(ellipseIn: CGRect(x: origin.x - r, y: origin.y - r, width: 2 * r, height: 2 * r)),
                       with: .color(.white.opacity(0.7 * (1 - f))), lineWidth: 3 * (1 - f) + 0.5)
        }
    }

    private static func drawPiece(in ctx: inout GraphicsContext, kind: Int, at p: CGPoint, angle: Double, color: Color, size s: Double) {
        switch kind {
        case 0:
            var g = ctx
            g.translateBy(x: p.x, y: p.y)
            g.rotate(by: .radians(angle))
            g.fill(Path(roundedRect: CGRect(x: -s / 2, y: -s, width: s, height: s * 2), cornerRadius: 1.5), with: .color(color))
        case 1:
            ctx.fill(Path(ellipseIn: CGRect(x: p.x - s / 2, y: p.y - s / 2, width: s, height: s)), with: .color(color))
        default:
            Sparkle.draw(in: &ctx, at: p, radius: s * 1.3, color: color, angle: angle)
        }
    }
}

// MARK: - Compact indicators

/// A small gradient capsule: "● Proving · +1.5 today" or "● Working · 42 blocks".
struct EarningsBadge: View {
    let summary: EarningsSummary
    let work: NodeWork

    var body: some View {
        HStack(spacing: 6) {
            Circle().fill(work.isLive && !work.proofsFailing ? EarnInk.mint : .orange).frame(width: 7, height: 7)
                .keyframeAnimator(initialValue: 1.0, trigger: work.height) { v, s in v.scaleEffect(s) } keyframes: { _ in
                    KeyframeTrack {
                        SpringKeyframe(1.7, duration: 0.12)
                        SpringKeyframe(1.0, duration: 0.3)
                    }
                }
            Text(line).lineLimit(1).minimumScaleFactor(0.6)
        }
        .font(.aeCaption.weight(.bold))
        .foregroundStyle(.white)
        .padding(.horizontal, 9).padding(.vertical, 5)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(LinearGradient(colors: work.isProving || summary.count > 0 ? [EarnInk.violet, EarnInk.pink] : [EarnInk.violet, EarnInk.sky],
                                   startPoint: .leading, endPoint: .trailing), in: Capsule())
        .shadow(color: EarnInk.pink.opacity(work.isLive ? 0.35 : 0), radius: 6, y: 2)
    }

    /// Fits the sidebar (170–190 pt) without an ellipsis; the cards keep the full wording.
    private var line: String {
        switch work.phase {
        case .proving: ProvingBadgeText.line(proofsFailing: work.proofsFailing, today: EarningsText.aeth(summary.todayWei))
        case .verifying: String(localized: "Working · \(work.blocksVerified) blocks")
        case .starting: String(localized: "Starting…")
        case .paused: String(localized: "Paused")
        }
    }
}

// MARK: - macOS: the data, and the views that read it

#if os(macOS)
/// Polls this Mac's rewards from its own node, sums them and notices new ones.
@MainActor
final class Earnings: ObservableObject {
    @Published private(set) var summary = EarningsSummary.empty
    /// The reward rows behind `summary`, exactly as the node reported them —
    /// the export writes what this app saw, not a fresh re-read
    /// (docs/research/node-reward-tax-2026.md).
    @Published private(set) var entries: [RewardEntry] = []
    @Published private(set) var celebration: RewardCelebration?
    @Published private(set) var runningSince: Date?
    @Published private(set) var firstHeight: UInt64?
    /// The chain's node-rewards standing (`aether_rewardStatus`), for the Network page.
    @Published private(set) var status: RewardStatus?
    /// How many rewards the node says exist for this address in total (nil:
    /// not reported). Below `entries.count` nothing is missing; above it, the
    /// cards say "N of M" instead of quietly showing a capped list.
    @Published private(set) var totalOnChain: Int?

    static let pollSeconds: TimeInterval = 20
    /// The node returns at most this many (the newest).
    static let limit = 10_000

    private weak var node: NodeController?
    private var operatorAddress: () -> String = { "" }
    private var timer: Timer?
    private var subscriptions = Set<AnyCancellable>()
    /// Rewards were read once for `address`; only later arrivals are celebrated.
    private var loaded = false
    private var address = ""
    private var fetching = false
    private var statusFetching = false

    /// Start following `node` (once; later calls do nothing). `operatorAddress`
    /// is the wallet address rewards would be paid to (it can change).
    func attach(_ node: NodeController, operatorAddress: @escaping () -> String) {
        guard self.node == nil else { return }
        #if DEBUG
        if DesignPreview.on {
            if DesignPreview.variant != "verifying" {
                let sample = EarningsPreviewHarness.sampleEntries(count: 24)
                entries = sample
                summary = EarningsSummary.aggregate(sample, now: Date())
            }
            if DesignPreview.rewardStatus { status = RewardStatus(json: DesignPreview.sampleRewardStatus) }
            runningSince = Date().addingTimeInterval(-11_520)
            firstHeight = 182_926
            return
        }
        #endif
        self.operatorAddress = operatorAddress
        self.node = node
        node.$state.sink { [weak self] in self?.stateChanged($0) }.store(in: &subscriptions)
        node.$height.sink { [weak self] in self?.heightChanged($0) }.store(in: &subscriptions)
        // A new proof or reward on the prover: look now instead of waiting for the timer.
        node.$prover.map { $0?.last_reward }.removeDuplicates().dropFirst()
            .sink { [weak self] _ in self?.refreshSoon() }.store(in: &subscriptions)
        timer = Timer.scheduledTimer(withTimeInterval: Self.pollSeconds, repeats: true) { [weak self] _ in
            Task { @MainActor in self?.refresh() }
        }
        refresh()
    }

    /// What the node is doing, for the views.
    func work(_ node: NodeController, canProve: Bool) -> NodeWork {
        var w = NodeWork(phase: phase(node))
        if node.prover?.program_unknown == true && node.prover?.paused == "program" {
            w.phase = .paused(String(localized: "Proving is paused: this Mac cannot confirm which proving program the network uses."))
        }
        w.height = node.height
        w.blocksVerified = firstHeight.map { node.height > $0 ? node.height - $0 : 0 } ?? 0
        w.runningSince = runningSince
        w.proofs = node.prover?.proofs ?? 0
        if w.isProving {
            w.proofsFailing = ProvingBadgeText.failing(
                reported: node.prover?.proofs_failing ?? false,
                proverRunning: node.prover?.running,
                runningSeconds: runningSince.map { Date().timeIntervalSince($0) }
            )
        }
        if let v = node.voting, v.registered {
            w.streakHours = v.streak
            w.votingNow = v.voting
        }
        w.canProve = canProve
        return w
    }

    private func phase(_ node: NodeController) -> NodeWork.Phase {
        switch node.state {
        case .running: node.prove ? .proving : .verifying
        case .starting: .starting
        case .waitingForPower: .paused(String(localized: "Paused on battery. It resumes on the power adapter."))
        case .off: .paused(String(localized: "The node is off."))
        case .failed(let why): .paused(why)
        }
    }

    private func stateChanged(_ s: NodeController.State) {
        switch s {
        case .running:
            if runningSince == nil { runningSince = Date() }
            refreshSoon()
        case .starting:
            break
        case .off, .waitingForPower, .failed:
            runningSince = nil
            firstHeight = nil
        }
    }

    private func heightChanged(_ h: UInt64) {
        guard h > 0, firstHeight == nil, node?.state == .running else { return }
        firstHeight = h
    }

    private func refreshSoon() {
        Task { @MainActor in
            try? await Task.sleep(for: .seconds(1))
            refresh()
        }
    }

    func refresh() {
        refreshStatus()
        guard let node, node.state == .running, !node.proveAddress.isEmpty, !fetching else { return }
        if node.proveAddress != address {
            address = node.proveAddress
            loaded = false
            summary = .empty
            entries = []
        }
        fetching = true
        let addr = address
        Task { @MainActor in
            defer { fetching = false }
            // Every page, not one flat read: the node caps a single response,
            // and a history longer than that cap would silently lose its tail
            // (the founder's 2,583 rewards ran past the default page size).
            let (rows, total) = await NodeController.allRewards(port: NodeController.port, address: addr)
            guard addr == address, !rows.isEmpty || (total ?? 0) == 0 else { return }
            let list = rows.compactMap(RewardEntry.init(json:))
            entries = list
            totalOnChain = total
            apply(EarningsSummary.aggregate(list, now: Date()))
        }
    }

    /// The standing behind the rewards (Network page). Asked without an operator
    /// when this Mac has no address to name: N and the cap are still the chain's.
    private func refreshStatus() {
        guard let node, node.state == .running, !statusFetching else { return }
        let op = node.proveAddress.isEmpty ? operatorAddress() : node.proveAddress
        guard !op.isEmpty else { return }
        statusFetching = true
        Task { @MainActor in
            defer { statusFetching = false }
            if let json = await LocalRPC.call(port: NodeController.port, method: "aether_rewardStatus", params: [op]) as? [String: Any] {
                let fresh = RewardStatus(json: json)
                if fresh != status { status = fresh }
            }
        }
    }

    private func apply(_ new: EarningsSummary) {
        if loaded, let amount = new.arrived(since: summary) {
            celebration = RewardCelebration(id: (celebration?.id ?? 0) + 1, amountWei: amount)
        }
        summary = new
        loaded = true
    }
}

/// The hero card on the Network page, while the node switch is on, with what the
/// chain says about node rewards underneath (`aether_rewardStatus`; nothing on a
/// chain without them, like the testnet).
struct NodeEarningsCard: View {
    @EnvironmentObject var node: NodeController
    @EnvironmentObject var earnings: Earnings
    @EnvironmentObject var model: WalletModel

    var body: some View {
        if node.enabled {
            VStack(spacing: 12) {
                EarningsHero(summary: earnings.summary, work: earnings.work(node, canProve: !model.address.isEmpty),
                             celebration: earnings.celebration, onProve: proveOn)
                RewardStandingCard(status: earnings.status)
                EarningsExportCard()
            }
        }
    }

    private func proveOn() {
        node.proveAddress = model.address
        node.prove = true
    }
}

/// "Export earnings (CSV)" on the Earnings screen: the rows this app saw
/// (the same ones the hero card sums), written to a file the user picks —
/// plus the one line that keeps EastSea honest about what it is
/// (docs/research/node-reward-tax-2026.md).
private struct EarningsExportCard: View {
    @EnvironmentObject var earnings: Earnings

    var body: some View {
        Card {
            HStack(alignment: .top, spacing: 12) {
                Image(systemName: "square.and.arrow.down").font(.aeTitle).foregroundStyle(Color.aether)
                VStack(alignment: .leading, spacing: 4) {
                    Text("Export earnings (CSV)").font(.aeHeadline)
                    Text("Every reward this Mac earned, as this app saw it — time, kind and the exact amount — for your own records. This is not tax advice.")
                        .font(.aeBody).foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                    if let total = earnings.totalOnChain, total > earnings.entries.count {
                        Text("The node counts \(total) rewards; \(earnings.entries.count) were read. The numbers here cover only what was read.")
                            .font(.aeFootnote).foregroundStyle(Color.warn)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
                Spacer(minLength: 8)
                Button("Export CSV…") { export() }
                    .disabled(earnings.entries.isEmpty)
                    .help("Saves eastsea-earnings.csv from what this app saw. The records never leave this Mac.")
            }
        }
    }

    private func export() {
        let panel = NSSavePanel()
        panel.nameFieldStringValue = "eastsea-earnings.csv"
        panel.allowedContentTypes = [.commaSeparatedText]
        if panel.runModal() == .OK, let url = panel.url {
            try? EarningsCSV.document(earnings.entries).write(to: url, atomically: true, encoding: .utf8)
        }
    }
}

/// What the chain itself counts (docs/design/15-node-rewards.md): operators
/// online, the 1/16 cap, this Mac's warm-up and its share of the last hour.
/// Testnet answers `enabled: false`, and then nothing is shown here.
struct RewardStandingCard: View {
    let status: RewardStatus?

    var body: some View {
        if let s = status, s.enabled {
            Card {
                VStack(alignment: .leading, spacing: 8) {
                    Text("Node rewards").font(.aeHeadline)
                    Text("Operators online: \(s.operatorsOnline) · one operator gets at most 1/\(s.maxShare) of the rewards")
                        .font(.aeBody).foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                    if let pct = s.warmupPercent, let days = s.warmupDaysLeft, days > 0 {
                        Text("Warm-up: \(pct)% of a full share · full in \(days) days")
                            .font(.aeBody).foregroundStyle(.secondary)
                    }
                    if let share = s.expectedShareWei, share != "0" {
                        Text(s.capped
                             ? String(localized: "Last hour: +\(EarningsText.aeth(share)) \(EarningsText.unit) · capped at 1/\(s.maxShare)")
                             : String(localized: "Last hour: +\(EarningsText.aeth(share)) \(EarningsText.unit)"))
                            .font(.aeBody).foregroundStyle(.secondary)
                    }
                }
            }
        }
    }
}

/// Home, under the balance and the actions: once a reward has arrived, the number
/// in a compact card (160–180 pt — the balance above stays the headline, the
/// reward stays vivid); before that, one quiet line (no zeros).
struct HomeEarnings: View {
    @EnvironmentObject var node: NodeController
    @EnvironmentObject var earnings: Earnings
    @EnvironmentObject var model: WalletModel
    /// Opens the page with the node and the full card.
    let open: () -> Void

    var body: some View {
        if node.enabled {
            let work = earnings.work(node, canProve: !model.address.isEmpty)
            if earnings.summary.count > 0, earnings.summary.totalWei != "0" {
                HomeEarningsCard(summary: earnings.summary, work: work, celebration: earnings.celebration, open: open)
            } else {
                NodeStatusLine(work: work, action: open, onProve: node.prove || model.address.isEmpty ? nil : proveOn)
            }
        }
    }

    /// Same as the Settings toggle "Prove blocks with Metal".
    private func proveOn() {
        node.proveAddress = model.address
        node.prove = true
    }
}

/// The compact earnings card for Home: the aurora and the one reward number at
/// 40 pt (never the 48 pt balance's rival), a delta for the last hour, and a
/// line of facts. Tapping opens the Network page, where the full hero lives.
private struct HomeEarningsCard: View {
    let summary: EarningsSummary
    let work: NodeWork
    var celebration: RewardCelebration?
    let open: () -> Void
    @Environment(\.narrowLayout) private var narrow
    /// On screen (scrolled into view); the aurora only moves while it is.
    @State private var visible = true

    private var pillText: String {
        work.phase.pillText
    }

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: Radius.card, style: .continuous)
        Button(action: open) {
            VStack(alignment: .leading, spacing: narrow ? 10 : 12) {
                HStack {
                    Text("Received so far").font(.aeFootnote.weight(.semibold)).foregroundStyle(.white.opacity(0.85))
                    Spacer(minLength: 8)
                    LivePill(text: pillText, live: work.isLive, beat: work.height, ring: false)
                }
                HStack(alignment: .firstTextBaseline, spacing: 8) {
                    CountingText(value: WeiMath.aeth(summary.totalWei), decimals: EarningsText.decimals(summary.totalWei))
                        .font(.heroNumberNarrow)
                        .monospacedDigit().lineLimit(1).minimumScaleFactor(0.5)
                        .shadow(color: EarnInk.gold.opacity(0.75), radius: 12)
                        .shadow(color: .black.opacity(0.18), radius: 2, y: 2)
                        .overlay(alignment: .topLeading) {
                            // An overlay, so the label's width never shifts the number.
                            FloatingReward(celebration: celebration).fixedSize().offset(x: narrow ? 40 : 60, y: -28)
                        }
                    Text(EarningsText.unit).font(.aeHeadline).foregroundStyle(.white.opacity(0.9))
                    Spacer(minLength: 8)
                    if summary.lastHourWei != "0" { HourDelta(wei: summary.lastHourWei).fixedSize() }
                }
                Text(facts)
                    .font(.aeFootnote.weight(.medium)).foregroundStyle(.white.opacity(0.85))
                    .lineLimit(1).truncationMode(.tail)
            }
            .foregroundStyle(.white)
            .padding(.horizontal, narrow ? CardPadding.narrow : CardPadding.wide)
            .padding(.vertical, narrow ? 20 : 22)
            .frame(maxWidth: .infinity, alignment: .leading)
            // Same rule as the hero: one moving thing per screen, and only while
            // proving and on screen; a live resize holds the last frame.
            .background { AuroraBackground(hot: true, live: work.isProving && visible) }
            .overlay { ConfettiBurst(trigger: celebration?.id ?? 0) }
            .clipShape(shape)
            .overlay(shape.strokeBorder(.white.opacity(0.28), lineWidth: 1))
            .shadow(color: EarnInk.pink.opacity(0.30), radius: 10, y: 4)
            .contentShape(shape)
        }
        .buttonStyle(.plain)
        .help("Open the Network page: the node and the full earnings card")
        .accessibilityElement(children: .combine)
        .onAppear { visible = true }
        .onDisappear { visible = false }
        .trackingScrollVisibility($visible)
    }

    /// "+12 today · 24 rewards · last one 3m ago" — what the number is made of.
    private var facts: String {
        var parts: [String] = []
        if summary.todayWei != "0" { parts.append(String(localized: "+\(EarningsText.aeth(summary.todayWei)) today")) }
        parts.append(String(localized: "\(summary.count) rewards"))
        if let at = summary.lastRewardAt { parts.append(String(localized: "last one \(EarningsText.ago(at, now: Date()))")) }
        return parts.joined(separator: " · ")
    }
}

/// "● This Mac is verifying blocks  ›"
struct NodeStatusLine: View {
    let work: NodeWork
    let action: () -> Void
    /// Proving is off: say where testnet rewards go, and offer to prove (nil: hidden).
    var onProve: (() -> Void)?

    var body: some View {
        if let onProve, work.phase == .verifying {
            proveOffer(onProve)
        } else {
            line
        }
    }

    /// Testnet rewards go only to the Mac that proves a block first; verifying alone earns nothing.
    private func proveOffer(_ prove: @escaping () -> Void) -> some View {
        HStack(spacing: 12) {
            Circle().fill(Color.aether).frame(width: 8, height: 8)
            Text("This Mac verifies blocks. Rewards go to Macs that prove them.")
                .font(.aeBody).foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            Spacer(minLength: 4)
            Button(action: prove) { Label("Prove blocks", systemImage: "bolt.fill") }
                .buttonStyle(.borderedProminent)
                .help("Uses the GPU and power while on, at your cost. The first valid proof of a block gets a test \(Brand.networkCoinTicker) reward in this wallet.")
        }
        .padding(.horizontal, 16).padding(.vertical, 12)
        .background(.background.secondary, in: RoundedRectangle(cornerRadius: Radius.card, style: .continuous))
    }

    private var line: some View {
        Button(action: action) {
            HStack(spacing: 8) {
                Circle().fill(work.isLive ? Color.aether : Color.warn).frame(width: 8, height: 8)
                Text(text).lineLimit(2)
                Spacer(minLength: 4)
                Image(systemName: "chevron.right").foregroundStyle(.tertiary)
            }
            .font(.aeBody)
            .foregroundStyle(.secondary)
            .padding(.horizontal, 16).padding(.vertical, 12)
            .background(.background.secondary, in: RoundedRectangle(cornerRadius: Radius.card, style: .continuous))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .help("Open the node on this Mac")
    }

    private var text: String {
        switch work.phase {
        case .verifying: String(localized: "This Mac is verifying blocks")
        case .proving: String(localized: "This Mac is proving blocks · no reward yet")
        case .starting: work.height > 0 ? String(localized: "Node catching up · block #\(String(work.height))") : String(localized: "Node starting…")
        case .paused(let why): why
        }
    }
}

/// Under the node switch in the sidebar.
struct EarningsSidebarBadge: View {
    @EnvironmentObject var node: NodeController
    @EnvironmentObject var earnings: Earnings

    var body: some View {
        if node.enabled {
            EarningsBadge(summary: earnings.summary, work: earnings.work(node, canProve: true))
        }
    }
}

/// One line in the menu-bar panel: "+1.5 test DBLN today".
struct EarningsMenuLine: View {
    @EnvironmentObject var node: NodeController
    @EnvironmentObject var earnings: Earnings

    var body: some View {
        let s = earnings.summary
        if node.enabled, node.prove || s.count > 0 {
            HStack(spacing: 6) {
                Image(systemName: "sparkles")
                Text("+\(EarningsText.aeth(s.todayWei)) \(EarningsText.unit) today").fontWeight(.heavy)
                Spacer(minLength: 4)
                Text("\(EarningsText.aeth(s.totalWei)) total").foregroundStyle(.secondary)
            }
            .font(.aeBody)
            .foregroundStyle(EarnInk.brand)
            .contentTransition(.numericText())
        }
    }
}
#endif

// MARK: - DEBUG preview harness

#if DEBUG
/// Sample cards with a reward landing every few seconds (`-earningsPreview` on iOS debug builds).
struct EarningsPreviewHarness: View {
    @State private var summary = EarningsPreviewHarness.sample(count: 24)
    @State private var celebration: RewardCelebration?
    @State private var height: UInt64 = 184_207

    var body: some View {
        ScrollView {
            VStack(spacing: 22) {
                switch UserDefaults.standard.string(forKey: "earningsPreview") {
                case "working":
                    EarningsHero(summary: .empty, work: verifying, onProve: {})
                case "proving":
                    EarningsHero(summary: .empty, work: NodeWork(phase: .proving, height: height, proofs: 3))
                default:
                    EmptyView()
                }
                EarningsHero(summary: summary, work: proving, celebration: celebration)
                EarningsBadge(summary: summary, work: proving)
                EarningsHero(summary: .empty, work: verifying, onProve: {})
                EarningsBadge(summary: .empty, work: verifying)
                EarningsHero(summary: .empty, work: NodeWork(phase: .proving, height: height, proofs: 3))
            }
            .padding(16)
        }
        .background(.background)
        .task { await loop() }
    }

    private var proving: NodeWork {
        NodeWork(phase: .proving, height: height, blocksVerified: 1_284, runningSince: Date().addingTimeInterval(-11_520), proofs: 57)
    }

    private var verifying: NodeWork {
        NodeWork(phase: .verifying, height: height, blocksVerified: 1_284 + height - 184_207,
                 runningSince: Date().addingTimeInterval(-11_520), streakHours: 5)
    }

    /// A block every second; a reward every few seconds.
    private func loop() async {
        var n = 24
        while !Task.isCancelled {
            for _ in 0..<4 {
                try? await Task.sleep(for: .seconds(1))
                height += 1
            }
            n += 1
            summary = Self.sample(count: n)
            celebration = RewardCelebration(id: (celebration?.id ?? 0) + 1, amountWei: "500000000000000000")
        }
    }

    static func sampleEntries(count: Int) -> [RewardEntry] {
        let now = Date()
        return (0..<count).map { i in
            let time = now.addingTimeInterval(Double(i - count + 1) * 600)
            return RewardEntry(proven: UInt64(184_000 + i), amountWei: "500000000000000000", height: UInt64(184_001 + i),
                               time: time, timestampMs: UInt64(time.timeIntervalSince1970 * 1000))
        }
    }

    static func sample(count: Int) -> EarningsSummary {
        EarningsSummary.aggregate(sampleEntries(count: count), now: Date())
    }
}

#Preview("Earnings") { EarningsPreviewHarness() }
#endif
