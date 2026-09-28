import SwiftUI

// Loading indicators in the aether colors (violet into pink).
//
// Both are drawn with Canvas inside a TimelineView(.animation), so there are no timers
// and no per-frame view diffing: one draw call per frame, and nothing at all while
// the view is off screen. With Reduce Motion on, they draw a single still frame.

/// Brand palette as raw components, so the trail can blend smoothly between the two.
private enum AetherInk {
    static let violet = RGB(r: 0.49, g: 0.40, b: 0.95)   // Color.aether
    static let pink = RGB(r: 1.00, g: 0.26, b: 0.58)
    static let sky = RGB(r: 0.36, g: 0.62, b: 1.00)

    struct RGB {
        let r, g, b: Double
        func mix(_ o: RGB, _ f: Double) -> RGB {
            RGB(r: r + (o.r - r) * f, g: g + (o.g - g) * f, b: b + (o.b - b) * f)
        }
        func color(_ opacity: Double = 1) -> Color { Color(red: r, green: g, blue: b).opacity(opacity) }
    }
}

// MARK: - Orbit spinner

/// A comet with a tapered violet-to-pink tail orbiting a faint ring. Its speed and tail
/// length breathe, so it never looks mechanical. From about 28 pt up it also gains a
/// pulsing core and two satellites on tilted orbits; at 12-13 pt it stays a crisp comet.
struct OrbitSpinner: View {
    /// Seconds per revolution of the comet.
    var period: Double = 1.3
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.colorScheme) private var scheme
    /// Holds its last frame while the window is dragged, so resizing does not pay
    /// for the blur layers on top of every layout pass.
    @Environment(\.liveResize) private var resizing
    /// Time is measured from here: small numbers keep angles precise on the GPU (Float).
    @State private var epoch = Date()

    /// How far the glow may spill past the frame, as a fraction of the frame's size.
    private static let bleed: CGFloat = 0.35

    var body: some View {
        GeometryReader { g in
            let d = min(g.size.width, g.size.height)
            TimelineView(.animation(minimumInterval: 1.0 / 60, paused: reduceMotion || resizing)) { tl in
                Canvas { ctx, size in
                    let t = reduceMotion ? 0.42 * period : tl.date.timeIntervalSince(epoch)
                    OrbitSpinner.draw(in: &ctx, size: size, diameter: d, time: t, period: period,
                                      glow: scheme == .dark ? 1 : 0.6)
                }
            }
            // The canvas is larger than the frame so the comet's glow is never cut off square.
            .padding(-d * Self.bleed)
        }
        .accessibilityHidden(true)
    }

    /// Draws one frame. Pure in `time`, so previews and snapshots can pin a moment.
    /// `glow` scales the blurred light; light backgrounds want less of it.
    static func draw(in ctx: inout GraphicsContext, size: CGSize, diameter: CGFloat? = nil, time t: Double,
                     period: Double = 1.3, glow: Double = 1) {
        let s = diameter ?? min(size.width, size.height)
        guard s > 2 else { return }
        let c = CGPoint(x: size.width / 2, y: size.height / 2)
        let rich = s >= 28
        let lw = max(1.4, s * (rich ? 0.055 : 0.13))
        let r = s / 2 - lw * (rich ? 2.2 : 0.9)

        // Eased motion: the comet surges, then glides; the tail stretches and contracts.
        let p = (t / period).truncatingRemainder(dividingBy: 1) * 2 * .pi
        let head = p + 0.42 * sin(p) - .pi / 2
        let tail = Double.pi * (0.62 + 0.38 * (0.5 + 0.5 * sin(t / (period * 2.1) * 2 * .pi)))
        let pulse = 0.5 + 0.5 * sin(t * 2 * .pi / 2.4)

        if rich { drawCore(in: &ctx, c: c, r: r, pulse: pulse) }

        // Track.
        ctx.stroke(Path(ellipseIn: CGRect(x: c.x - r, y: c.y - r, width: 2 * r, height: 2 * r)),
                   with: .color(AetherInk.violet.color(rich ? 0.14 : 0.18)), lineWidth: max(1, lw * 0.55))

        if rich { drawSatellites(in: &ctx, c: c, r: r, t: t, period: period, glow: glow) }

        // Bloom under the tail, then the crisp tail over it.
        ctx.drawLayer { g in
            g.addFilter(.blur(radius: lw * (rich ? 2.2 : 1.1)))
            drawTail(in: &g, c: c, r: r, head: head, length: tail * 0.75, width: lw * 1.6, alpha: (rich ? 0.75 : 0.55) * glow, segments: 24)
        }
        drawTail(in: &ctx, c: c, r: r, head: head, length: tail, width: lw, alpha: 1, segments: rich ? 64 : 32)

        // Head: a pink glow with a hot white centre.
        let hp = CGPoint(x: c.x + r * cos(head), y: c.y + r * sin(head))
        ctx.drawLayer { g in
            g.addFilter(.blur(radius: lw * (rich ? 1.8 : 1.0)))
            g.fill(dot(hp, lw * (rich ? 2.4 : 1.5)), with: .color(AetherInk.pink.color(0.9 * glow)))
        }
        ctx.fill(dot(hp, lw * 0.78), with: .color(AetherInk.pink.mix(RGB.white, 0.55 * glow).color()))
        if rich { ctx.fill(dot(hp, lw * 0.42), with: .color(.white)) }
    }

    private typealias RGB = AetherInk.RGB

    private static func dot(_ p: CGPoint, _ radius: CGFloat) -> Path {
        Path(ellipseIn: CGRect(x: p.x - radius, y: p.y - radius, width: 2 * radius, height: 2 * radius))
    }

    /// The tail as one tapered crescent, shaded by a conic gradient so there are no seams:
    /// thin, clear and violet at the end, full width and pink at the head.
    private static func drawTail(in ctx: inout GraphicsContext, c: CGPoint, r: CGFloat, head: Double, length: Double,
                                 width: CGFloat, alpha: Double, segments n: Int) {
        let start = head - length
        func point(_ f: Double, _ side: CGFloat) -> CGPoint {
            let a = start + length * f
            let rr = r + side * width * (0.12 + 0.88 * CGFloat(pow(f, 0.8))) / 2
            return CGPoint(x: c.x + rr * cos(a), y: c.y + rr * sin(a))
        }
        var shape = Path()
        shape.move(to: point(0, 1))
        for i in 1...n { shape.addLine(to: point(Double(i) / Double(n), 1)) }
        // Round the head end so it tucks under the glowing dot.
        shape.addArc(center: point(1, 0), radius: width / 2, startAngle: .radians(head), endAngle: .radians(head + .pi), clockwise: false)
        for i in stride(from: n, through: 0, by: -1) { shape.addLine(to: point(Double(i) / Double(n), -1)) }
        shape.closeSubpath()
        let span = length / (2 * .pi)
        let stops: [Gradient.Stop] = (0...6).map { k in
            let f = Double(k) / 6
            return .init(color: AetherInk.violet.mix(AetherInk.pink, f * f).color(alpha * pow(f, 1.4)), location: span * f)
        } + [.init(color: AetherInk.pink.color(alpha), location: min(1, span + 0.02)),
             .init(color: AetherInk.violet.color(0), location: min(1, span + 0.021))]
        ctx.fill(shape, with: .conicGradient(Gradient(stops: stops), center: c, angle: .radians(start)))
    }

    /// A soft pulsing glow at the centre: the "aether" the comets circle.
    private static func drawCore(in ctx: inout GraphicsContext, c: CGPoint, r: CGFloat, pulse: Double) {
        let halo = r * (0.62 + 0.10 * pulse)
        ctx.fill(dot(c, halo), with: .radialGradient(
            Gradient(colors: [AetherInk.violet.color(0.34 + 0.12 * pulse), AetherInk.pink.color(0.10), .clear]),
            center: c, startRadius: 0, endRadius: halo))
        let core = r * (0.15 + 0.035 * pulse)
        ctx.drawLayer { g in
            g.addFilter(.blur(radius: core * 0.6))
            g.fill(dot(c, core * 1.25), with: .color(AetherInk.violet.mix(AetherInk.pink, 0.5).color(0.85)))
        }
        ctx.fill(dot(c, core * 0.62), with: .radialGradient(
            Gradient(colors: [.white, AetherInk.violet.mix(.init(r: 1, g: 1, b: 1), 0.3).color()]),
            center: c, startRadius: 0, endRadius: core * 0.62))
    }

    /// Two small particles on tilted elliptical orbits, dimmer when "behind" the core.
    private static func drawSatellites(in ctx: inout GraphicsContext, c: CGPoint, r: CGFloat, t: Double, period: Double, glow: Double) {
        let orbits: [(tilt: Double, speed: Double, phase: Double, ink: AetherInk.RGB)] = [
            (.pi / 3, 0.55, 0.0, AetherInk.sky),
            (-.pi / 3, -0.8, 2.1, AetherInk.pink),
        ]
        let rx = r * 0.70, ry = r * 0.25
        for o in orbits {
            let rot = CGAffineTransform(translationX: c.x, y: c.y).rotated(by: o.tilt)
            let ring = Path(ellipseIn: CGRect(x: -rx, y: -ry, width: 2 * rx, height: 2 * ry)).applying(rot)
            ctx.stroke(ring, with: .color(o.ink.color(0.16)), lineWidth: max(0.6, r * 0.018))
            let a = (t / period * o.speed).truncatingRemainder(dividingBy: 1) * 2 * .pi + o.phase
            for k in stride(from: 6, through: 0, by: -1) {
                let ak = a - Double(k) * 0.11 * (o.speed > 0 ? 1 : -1)
                let depth = 0.55 + 0.45 * sin(ak)              // 0.1 behind ... 1 in front
                let p = CGPoint(x: rx * cos(ak), y: ry * sin(ak)).applying(rot)
                let fade = 1 - Double(k) / 7
                let rad = r * 0.045 * (0.6 + 0.4 * depth) * (k == 0 ? 1.25 : 0.35 + 0.5 * fade)
                if k == 0 {
                    ctx.drawLayer { g in
                        g.addFilter(.blur(radius: rad * 1.6))
                        g.fill(dot(p, rad * 2.2), with: .color(o.ink.color(0.8 * depth * glow)))
                    }
                    ctx.fill(dot(p, rad), with: .color(o.ink.mix(.init(r: 1, g: 1, b: 1), 0.5).color(0.6 + 0.4 * depth)))
                } else {
                    ctx.fill(dot(p, rad), with: .color(o.ink.color(0.45 * fade * depth)))
                }
            }
        }
    }
}

private extension AetherInk.RGB {
    static let white = AetherInk.RGB(r: 1, g: 1, b: 1)
}

// MARK: - Aurora shimmer

/// A loading placeholder where soft violet, pink and blue light drifts like an aurora,
/// crossed now and then by a glassy sheen. Same footprint as the content it stands in for.
struct ShimmerBar: View {
    var cornerRadius: CGFloat = 14
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.liveResize) private var resizing
    @State private var epoch = Date()

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: cornerRadius, style: .continuous)
        TimelineView(.animation(minimumInterval: 1.0 / 60, paused: reduceMotion || resizing)) { tl in
            Canvas { ctx, size in
                let t = reduceMotion ? 1.0 : tl.date.timeIntervalSince(epoch)
                ShimmerBar.draw(in: &ctx, size: size, time: t)
            }
        }
        .background(.quaternary.opacity(0.45))
        .clipShape(shape)
        .overlay(shape.strokeBorder(LinearGradient(colors: [Color.aether.opacity(0.35), .pink.opacity(0.25), Color.aether.opacity(0.10)],
                                                   startPoint: .leading, endPoint: .trailing), lineWidth: 1))
        .accessibilityElement()
        .accessibilityLabel("Loading")
    }

    static func draw(in ctx: inout GraphicsContext, size: CGSize, time t: Double) {
        let w = size.width, h = size.height
        guard w > 2, h > 2 else { return }
        // Three drifting blobs of light, heavily blurred into one another.
        let blobs: [(ink: AetherInk.RGB, alpha: Double, speed: Double, phase: Double, width: Double)] = [
            (AetherInk.violet, 0.70, 0.50, 0.0, 0.62),
            (AetherInk.pink, 0.55, 0.37, 2.2, 0.50),
            (AetherInk.sky, 0.40, 0.29, 4.1, 0.45),
        ]
        ctx.drawLayer { g in
            g.addFilter(.blur(radius: h * 0.42))
            for b in blobs {
                let x = w * (0.5 + 0.42 * sin(t * b.speed + b.phase))
                let y = h * (0.5 + 0.35 * sin(t * b.speed * 1.7 + b.phase * 0.6))
                let bw = w * b.width * (0.9 + 0.2 * sin(t * 0.9 + b.phase))
                let bh = h * 1.1
                g.fill(Path(ellipseIn: CGRect(x: x - bw / 2, y: y - bh / 2, width: bw, height: bh)),
                       with: .color(b.ink.color(b.alpha)))
            }
        }
        // A slanted sheen that sweeps across every few seconds.
        let cycle = 2.6
        let u = (t.truncatingRemainder(dividingBy: cycle)) / cycle          // 0...1
        let sx = -w * 0.4 + (w * 1.8) * easeInOut(u)
        let band = max(h * 0.9, w * 0.22)
        var sheen = Path()
        sheen.move(to: CGPoint(x: sx, y: 0))
        sheen.addLine(to: CGPoint(x: sx + band, y: 0))
        sheen.addLine(to: CGPoint(x: sx + band - h * 0.6, y: h))
        sheen.addLine(to: CGPoint(x: sx - h * 0.6, y: h))
        sheen.closeSubpath()
        ctx.fill(sheen, with: .linearGradient(
            Gradient(colors: [.white.opacity(0), .white.opacity(0.38), .white.opacity(0)]),
            startPoint: CGPoint(x: sx - h * 0.3, y: 0), endPoint: CGPoint(x: sx + band - h * 0.3, y: 0)))
    }

    private static func easeInOut(_ x: Double) -> Double { x * x * (3 - 2 * x) }
}

#if DEBUG
/// Every spinner size and the shimmer on one screen, for eyeballing in previews.
struct SpinnerGallery: View {
    var body: some View {
        VStack(spacing: 28) {
            OrbitSpinner().frame(width: 140, height: 140)
            HStack(spacing: 28) {
                OrbitSpinner().frame(width: 56, height: 56)
                OrbitSpinner().frame(width: 32, height: 32)
                OrbitSpinner().frame(width: 20, height: 20)
                OrbitSpinner().frame(width: 13, height: 13)
            }
            HStack(spacing: 7) {
                OrbitSpinner().frame(width: 13, height: 13)
                Text("Verifying")
            }
            .font(.caption.weight(.semibold)).foregroundStyle(Color.aether)
            .padding(.horizontal, 11).padding(.vertical, 5)
            .background(Color.aether.opacity(0.10), in: Capsule())
            ShimmerBar().frame(width: 220, height: 52)
        }
        .padding(32)
    }
}

#Preview("Spinners") { SpinnerGallery() }
#endif
