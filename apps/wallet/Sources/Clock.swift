import Dispatch
import Foundation

/// Time for measuring durations (red team #6): a wall clock jumps when a
/// person sets the date, when NTP steps it, or across a DST change — a
/// "60 s stall" could then appear in an instant (restarting a healthy
/// validator costs the network a signature) or stay hidden for hours. Every
/// duration the app decides by is read from here instead; wall-clock `Date`
/// stays only where a person reads it or where it must survive a restart.
///
/// Why `DispatchTime` (mach_absolute_time) and not the alternatives:
/// - Darwin's `CLOCK_MONOTONIC` *does* count time asleep, so an overnight
///   sleep would still look like an 8-hour stall the moment the Mac wakes
///   and the wake notification was lost — the exact failure #6 is about;
/// - `ProcessInfo.systemUptime` is awake-only like this one, but returns
///   rounded `Double` seconds, while `DispatchTime.uptimeNanoseconds` is
///   integer nanoseconds on every Apple platform, macOS and iOS alike;
/// - this clock never moves backward, so a step in either direction can
///   neither fabricate nor hide elapsed time, and it does not advance while
///   the Mac sleeps, so sleep itself can never look like a stall.
/// The existing sleep/wake invalidation (red team #9) keeps working: it
/// still resets every measurement on a real wake, and this clock makes even
/// a *lost* wake notification harmless.
protocol Clock {
    /// A reading that only ever moves forward while the Mac is awake.
    var now: MonotonicInstant { get }
}

/// A moment on the never-jumping clock. Only the difference between two
/// instants means anything (a duration); the value itself is meaningless —
/// and a wall-clock `Date` cannot be passed where this is expected, which is
/// the point of the type (red team #6).
struct MonotonicInstant: Equatable, Comparable {
    /// Monotonic seconds from the clock's own (arbitrary) zero.
    let seconds: TimeInterval
    /// A reading any real clock is later than (for "due since" fields).
    static let distantPast = MonotonicInstant(-.infinity)

    init(_ seconds: TimeInterval) { self.seconds = seconds }

    static func < (lhs: MonotonicInstant, rhs: MonotonicInstant) -> Bool { lhs.seconds < rhs.seconds }

    func advanced(by interval: TimeInterval) -> MonotonicInstant { MonotonicInstant(seconds + interval) }

    /// How long from `earlier` to this instant.
    func elapsed(since earlier: MonotonicInstant) -> TimeInterval { seconds - earlier.seconds }
}

/// The production clock; the app injects it, tests inject their own.
struct UptimeClock: Clock {
    var now: MonotonicInstant {
        MonotonicInstant(TimeInterval(DispatchTime.now().uptimeNanoseconds) / 1_000_000_000)
    }
}
