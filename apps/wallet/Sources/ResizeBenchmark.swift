#if os(macOS) && DEBUG
import AppKit
import QuartzCore

// `-resizeBenchmark` (debug builds only, with -designPreview): drags the window
// through a resize storm — the width sweeping across both layout breakpoints
// while the earnings aurora animates — and reports what one resize step costs
// on the main thread, so a resize change can be measured, not guessed at:
//
//   Aether.app/Contents/MacOS/Aether -designPreview -resizeBenchmark \
//     -nodeEnabled 1 -proveBlocks 1 -proveAddress 0x00
//
// Each step times the work itself — `setFrame(…, display: false)`, then laying
// out the hosting view and committing the layer tree — because timing a
// displayed frame only measures the wait for the next vsync. Two passes are
// printed, `raw` and `paused`: the second posts the same NSWindow live-resize
// notifications a real drag sends, which is what pauses the continuous
// animations. CPU seconds per pass come from getrusage, which the vsync wait
// does not inflate. Nothing is written to persistent defaults: the launch
// arguments live in the argument domain only.

enum ResizeBenchmark {
    static var on: Bool { ProcessInfo.processInfo.arguments.contains("-resizeBenchmark") }

    /// Steps per pass: about 4 s at one resize per display frame.
    private static let ticks = 250

    /// Run once the window is up (design preview sizes it at 0.5 s; start after that).
    /// A warm-up pass first: the first resize at each size pays cold layout caches,
    /// which would otherwise make whichever pass runs first look worse.
    /// The app activates and holds a power assertion first: launched from a script
    /// it stays in the background, and App Nap would suspend its timers mid-storm.
    @MainActor
    static func run() {
        NSApp.activate(ignoringOtherApps: true)
        let awake = ProcessInfo.processInfo.beginActivity(options: .userInitiated, reason: "Aether resize benchmark")
        DispatchQueue.main.asyncAfter(deadline: .now() + 1.2) {
            guard let win = NSApp.windows.first(where: { $0.isVisible && $0.canBecomeMain }) else {
                print("resize-benchmark: no window")
                exit(1)
            }
            storm(win, paused: false, name: "warmup") {
                storm(win, paused: false, name: "raw") {
                    storm(win, paused: true, name: "paused") {
                        storm(win, paused: false, name: "raw2") { _ = awake; exit(0) }
                    }
                }
            }
        }
    }

    private static func storm(_ win: NSWindow, paused: Bool, name: String, done: @escaping () -> Void) {
        let origin = win.frame.origin
        let cpuAtStart = cpuSeconds()
        if paused { NotificationCenter.default.post(name: NSWindow.willStartLiveResizeNotification, object: win) }

        var steps: [Double] = []
        var i = 0
        let timer = DispatchSource.makeTimerSource(queue: .main)
        timer.schedule(deadline: .now() + 0.05, repeating: .milliseconds(16))
        timer.setEventHandler {
            i += 1
            // Three full sweeps between an iPhone-wide and a wide window: every
            // adaptive breakpoint (680, 560) is crossed six times.
            let f = Double(i) / Double(ticks)
            let rect = NSRect(x: origin.x, y: origin.y,
                              width: 800 + 240 * cos(2 * .pi * 3 * f),
                              height: 720 + 40 * sin(2 * .pi * f))
            let t0 = CACurrentMediaTime()
            win.setFrame(rect, display: false)
            win.contentView?.layoutSubtreeIfNeeded()
            CATransaction.flush()
            steps.append((CACurrentMediaTime() - t0) * 1000)
            if i >= ticks {
                timer.cancel()
                DispatchQueue.main.asyncAfter(deadline: .now() + 0.4) {
                    if paused { NotificationCenter.default.post(name: NSWindow.didEndLiveResizeNotification, object: win) }
                    report(name, steps, cpuSeconds() - cpuAtStart)
                    done()
                }
            }
        }
        timer.resume()
    }

    /// This process's CPU seconds so far (the vsync wait does not count as CPU).
    private static func cpuSeconds() -> Double {
        var ru = rusage()
        getrusage(RUSAGE_SELF, &ru)
        return Double(ru.ru_utime.tv_sec) + Double(ru.ru_utime.tv_usec) / 1e6
            + Double(ru.ru_stime.tv_sec) + Double(ru.ru_stime.tv_usec) / 1e6
    }

    private static func report(_ name: String, _ steps: [Double], _ cpu: Double) {
        guard steps.count > 10 else { print("resize-benchmark \(name): no steps"); return }
        let ms = steps.sorted()
        func p(_ q: Double) -> Double { ms[min(ms.count - 1, Int(Double(ms.count - 1) * q))] }
        // A step over one display period means a dropped frame in a real drag.
        let line = String(format: "resize-benchmark %@: steps=%d p50=%.2fms p95=%.2fms max=%.2fms over16.7ms=%d over34ms=%d cpu=%.2fs",
                          name, ms.count, p(0.5), p(0.95), ms.last!, ms.filter { $0 > 16.7 }.count, ms.filter { $0 > 34 }.count, cpu)
        print(line)
        // Launched through `open`, stdout is lost: `-benchmarkReport <path>` also
        // appends each line to a file, so scripted runs can collect the numbers.
        if let i = ProcessInfo.processInfo.arguments.firstIndex(of: "-benchmarkReport"),
           i + 1 < ProcessInfo.processInfo.arguments.count {
            let path = ProcessInfo.processInfo.arguments[i + 1]
            // FileHandle(forWritingAtPath:) needs the file to exist; create it on
            // first use so a fresh run does not silently drop every line.
            if !FileManager.default.fileExists(atPath: path) {
                FileManager.default.createFile(atPath: path, contents: nil)
            }
            if let h = FileHandle(forWritingAtPath: path) {
                _ = try? h.seekToEnd()
                try? h.write(contentsOf: Data((line + "\n").utf8))
                try? h.close()
            }
        }
    }
}
#endif
