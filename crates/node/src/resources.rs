//! Resource limits: memory, CPU and disk budgets so a Mac running Aether
//! never runs away with it (docs/ops/resource-limits.md).
//!
//! 2026-09-29, on a 64 GB Mac: aether-prover at 14 GB and 350% CPU pushed the
//! system to 17.5/18.4 GB of swap and load average ~1000, and the testnet
//! validators on the same Mac slowed to ~0.45 blocks/s. macOS enforces no
//! per-process RSS limit (RLIMIT_RSS and RLIMIT_AS are not honored), so the
//! node watches its own child instead: the prover sidecar's physical footprint
//! is sampled every 2 s (`proc_pid_rusage`), and past the cap the sidecar is
//! killed and restarted with a growing back-off.
//!
//! The same monitor watches the system (memory pressure, swap, battery) and
//! the data volume's free space, and samples are shared with `aether_status`.

use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

pub const GB: u64 = 1024 * 1024 * 1024;

/// kern.memorystatus_vm_pressure_level values (kVMMemoryPressure*).
pub const PRESSURE_NORMAL: u8 = 1;
pub const PRESSURE_WARN: u8 = 2;
pub const PRESSURE_CRITICAL: u8 = 4;

/// Swap use above this fraction pauses proving (the 2026-09-29 incident sat at 95%).
pub const SWAP_PAUSE_PERMILLE: u16 = 500;

/// How long the system must be back to normal before paused proving resumes.
pub const RESUME_AFTER: Duration = Duration::from_secs(5 * 60);

/// Free space that must return before disk-guarded writes resume (min + this).
pub const DISK_RESUME: u64 = 2 * GB;
/// Warn wallets before the hard participation floor is reached.
pub const DISK_WARN_AHEAD: u64 = 3 * GB;

/// How often the watchdog samples the system.
const CHECK: Duration = Duration::from_secs(2);

/// How often the node's own footprint lands in the log.
const FOOTPRINT_EVERY: Duration = Duration::from_secs(10 * 60);

// ------------------------------------------------------------------ the machine

/// This Mac's physical RAM, in bytes.
#[cfg(target_os = "macos")]
pub fn physical_ram() -> u64 {
    let mut v: u64 = 0;
    let mut len = std::mem::size_of::<u64>();
    let ok = unsafe {
        libc::sysctlbyname(
            b"hw.memsize\0".as_ptr().cast(),
            &mut v as *mut u64 as *mut libc::c_void,
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    } == 0;
    ok.then_some(v).filter(|v| *v > 0).unwrap_or(16 * GB)
}

#[cfg(not(target_os = "macos"))]
pub fn physical_ram() -> u64 {
    16 * GB
}

pub fn cores() -> usize {
    std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1)
}

/// A process's physical footprint in bytes — the number Activity Monitor's
/// "Memory" column shows, so caps and displays agree. None when it is gone.
#[cfg(target_os = "macos")]
pub fn footprint(pid: u32) -> Option<u64> {
    #[repr(C)]
    struct Head {
        // rusage_info_v4 up to ri_phys_footprint, padded to the full struct.
        ri_uuid: [u8; 16],
        ri_user_time: u64,
        ri_system_time: u64,
        ri_pkg_idle_wkups: u64,
        ri_interrupt_wkups: u64,
        ri_pageins: u64,
        ri_wired_size: u64,
        ri_resident_size: u64,
        ri_phys_footprint: u64,
        _rest: [u8; 256],
    }
    // proc_pid_rusage lives in libSystem (which libproc re-exports into); an
    // explicit `-lproc` fails to link on current toolchains.
    extern "C" {
        fn proc_pid_rusage(pid: libc::pid_t, flavor: libc::c_int, buffer: *mut libc::c_void) -> libc::c_int;
    }
    const RUSAGE_INFO_V4: libc::c_int = 4;
    let mut buf: Head = unsafe { std::mem::zeroed() };
    let ok = unsafe { proc_pid_rusage(pid as libc::pid_t, RUSAGE_INFO_V4, &mut buf as *mut _ as *mut libc::c_void) } == 0;
    ok.then_some(buf.ri_phys_footprint)
}

#[cfg(not(target_os = "macos"))]
pub fn footprint(_pid: u32) -> Option<u64> {
    None
}

/// The kernel's memory-pressure level (`kern.memorystatus_vm_pressure_level`):
/// 1 normal, 2 warn, 4 critical. None when the sysctl is unavailable.
#[cfg(target_os = "macos")]
pub fn pressure_level() -> Option<u8> {
    let mut v: libc::c_int = 0;
    let mut len = std::mem::size_of::<libc::c_int>();
    let ok = unsafe {
        libc::sysctlbyname(
            b"kern.memorystatus_vm_pressure_level\0".as_ptr().cast(),
            &mut v as *mut libc::c_int as *mut libc::c_void,
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    } == 0;
    ok.then_some(v as u8)
}

#[cfg(not(target_os = "macos"))]
pub fn pressure_level() -> Option<u8> {
    None
}

/// The same levels from free pages plus the pages the kernel can take back
/// (`reclaimable_pages`), for the machines where the pressure sysctl does not
/// answer: under 10% of the RAM available is `warn`, under 5% is `critical`
/// (docs/ops/resource-limits.md).
pub fn pages_pressure(free_pages: u64, inactive_pages: u64, page_size: u64, ram: u64) -> Option<u8> {
    let available = (free_pages + inactive_pages).checked_mul(page_size)?;
    if ram == 0 || page_size == 0 {
        return None;
    }
    // Per-mille of the RAM, so no float rounding decides a kill.
    let available = available.min(ram) * 1000 / ram;
    Some(if available < 50 {
        PRESSURE_CRITICAL
    } else if available < 100 {
        PRESSURE_WARN
    } else {
        PRESSURE_NORMAL
    })
}

/// Reclaimable memory available to a snapshot build. The Mac path uses the
/// same page counters as the watchdog; absence is handled conservatively by
/// `snapshot_memory_budget` rather than interpreted as unlimited memory.
#[cfg(target_os = "macos")]
pub fn available_memory() -> Option<u64> {
    let pages = u64::from(sysctl_u32(b"vm.page_free_count\0")?).saturating_add(reclaimable_pages()?);
    let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    (page_size > 0).then(|| pages.saturating_mul(page_size as u64).min(physical_ram()))
}

#[cfg(not(target_os = "macos"))]
pub fn available_memory() -> Option<u64> {
    std::fs::read_to_string("/proc/meminfo").ok()?.lines().find_map(|line| {
        line.strip_prefix("MemAvailable:")?.split_whitespace().next()?.parse::<u64>().ok().map(|kib| kib.saturating_mul(1024))
    })
}

/// Test-only replacements for the host's pressure level and available memory,
/// so the snapshot gate's tests run the same on every machine (a busy Mac
/// sits at WARN most of the day). Process-wide scope: tests that pin readings
/// hold [`SEAM`] while they do, and integration tests pin one shared value.
///
/// Guard: compiled only under `cfg(test)` and under the `test-seam` feature,
/// which nothing enables but this crate's own `[dev-dependencies]`
/// self-reference. The shipped binary (`cargo build`, with or without
/// `--release`) compiles the whole seam out, so no environment variable,
/// config file or RPC can weaken a real node's gate.
#[cfg(any(test, feature = "test-seam"))]
static TEST_READINGS: RwLock<Option<(Option<u8>, Option<u64>)>> = RwLock::new(None);

/// Serializes the lib's tests that pin [`TEST_READINGS`]: they run in parallel
/// threads of one process and would otherwise read each other's overrides.
#[cfg(any(test, feature = "test-seam"))]
pub static SEAM: Mutex<()> = Mutex::new(());

/// Pin the readings the snapshot gate sees. `None` for a field falls back to
/// the host for that field; [`clear_test_readings`] restores the host.
#[cfg(any(test, feature = "test-seam"))]
pub fn set_test_readings(pressure: Option<u8>, available: Option<u64>) {
    *TEST_READINGS.write().expect("test readings") = Some((pressure, available));
}

#[cfg(any(test, feature = "test-seam"))]
pub fn clear_test_readings() {
    *TEST_READINGS.write().expect("test readings") = None;
}

#[cfg(any(test, feature = "test-seam"))]
static TEST_FREE_DISK: RwLock<Option<u64>> = RwLock::new(None);

/// Pin free disk space for isolated disk-floor regression tests. Shipped
/// binaries compile this seam out, like the memory readings above.
#[cfg(any(test, feature = "test-seam"))]
pub fn set_test_free_disk(bytes: Option<u64>) {
    *TEST_FREE_DISK.write().expect("test free disk") = bytes;
}

fn test_readings() -> Option<(Option<u8>, Option<u64>)> {
    #[cfg(any(test, feature = "test-seam"))]
    { *TEST_READINGS.read().expect("test readings") }
    #[cfg(not(any(test, feature = "test-seam")))]
    { None }
}

/// Extra allocation allowed for a snapshot: at most one quarter of currently
/// available memory and at most the configured node memory budget.
pub fn snapshot_memory_budget() -> Result<u64, String> {
    snapshot_budget_for(gate_pressure(), gate_available(), gate_configured())
}

pub fn snapshot_memory_budget_for(available: u64, configured: u64) -> u64 {
    (available / 4).min(configured)
}

/// The snapshot gate's pressure policy (docs/design/05-state.md):
///
/// * CRITICAL always refuses — the kernel is already reclaiming for survival,
///   and a build competing with it can push the Mac into swap or a jetsam kill.
/// * WARN — where an ordinary consumer Mac sits for much of a day under normal
///   use — is not a refusal by itself: the same budget check as at NORMAL
///   decides. A build that fits still runs, so a busy-but-healthy Mac keeps
///   serving checkpoint sync to new and recovering nodes; one that does not
///   fit is refused exactly as at NORMAL would refuse it.
///
/// The follower-side inbound guard (follow.rs, audit 3 A3-5) reads the same
/// budget through [`snapshot_memory_budget`], so this one policy governs both
/// serving and downloading.
fn snapshot_budget_for(pressure: Option<u8>, available: u64, configured: u64) -> Result<u64, String> {
    if pressure.is_some_and(|p| p >= PRESSURE_CRITICAL) {
        return Err("snapshot build refused: system memory pressure is critical".into());
    }
    Ok(snapshot_memory_budget_for(available, configured))
}

/// The serving-side gate (rpc.rs): may a build whose estimated extra memory is
/// `estimate` run on this host's live readings?
pub fn snapshot_build_gate(estimate: u64) -> Result<(), String> {
    snapshot_gate_for(gate_pressure(), estimate, gate_available(), gate_configured())
}

/// [`snapshot_build_gate`] on chosen numbers: the pressure policy first, then
/// the estimate against the budget.
pub fn snapshot_gate_for(pressure: Option<u8>, estimate: u64, available: u64, configured: u64) -> Result<(), String> {
    let budget = snapshot_budget_for(pressure, available, configured)?;
    if estimate > budget {
        return Err(format!("snapshot build refused: estimated extra memory {estimate} bytes exceeds budget {budget} bytes"));
    }
    Ok(())
}

/// Available memory with the page counters' absence handled conservatively: if
/// they are unavailable, permit at most 128 MiB of new allocation instead of
/// inferring headroom from a possibly guessed RAM.
fn gate_available() -> u64 {
    gate_available_memory().unwrap_or(512 * 1024 * 1024)
}

fn gate_configured() -> u64 {
    monitor().map(|m| m.limits.max_memory).unwrap_or_else(default_cache_budget)
}

/// The pressure level the snapshot gate sees: a test seam's override when one
/// is pinned, the kernel's own level (with the page-count fallback) otherwise.
/// The watchdog's own sampling stays on the raw readings.
fn gate_pressure() -> Option<u8> {
    match test_readings() {
        Some((pressure, _)) => pressure,
        None => pressure_level().or_else(pages_pressure_now),
    }
}

fn gate_available_memory() -> Option<u64> {
    match test_readings() {
        Some((_, available)) => available,
        None => available_memory(),
    }
}

/// `pages_pressure` for this Mac, read from `vm.page_*_count` and the page size.
#[cfg(target_os = "macos")]
fn pages_pressure_now() -> Option<u8> {
    let free = sysctl_u32(b"vm.page_free_count\0")?;
    let inactive = reclaimable_pages()?;
    let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    (page > 0).then(|| pages_pressure(free as u64, inactive as u64, page as u64, physical_ram())).flatten()
}

/// The pages the kernel can take back without paging anything in. The name of
/// this queue changed: `vm.page_inactive_count` on older macOS, and
/// `vm.page_pageable_internal_count` on the current ones (Darwin 24+).
#[cfg(target_os = "macos")]
fn reclaimable_pages() -> Option<u64> {
    [b"vm.page_inactive_count\0".as_slice(), b"vm.page_pageable_internal_count\0".as_slice()]
        .into_iter()
        .find_map(|name| sysctl_u32(name))
        .map(u64::from)
}

#[cfg(target_os = "macos")]
fn sysctl_u32(name: &[u8]) -> Option<u32> {
    let mut v: u32 = 0;
    let mut len = std::mem::size_of::<u32>();
    let ok = unsafe {
        libc::sysctlbyname(
            name.as_ptr().cast(),
            &mut v as *mut u32 as *mut libc::c_void,
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    } == 0;
    ok.then_some(v)
}

#[cfg(not(target_os = "macos"))]
fn pages_pressure_now() -> Option<u8> {
    None
}

/// (used, total) swap bytes, from `vm.swapusage`.
pub fn swap_usage() -> Option<(u64, u64)> {
    #[cfg(target_os = "macos")]
    let raw = sysctl_string(b"vm.swapusage\0");
    #[cfg(not(target_os = "macos"))]
    let raw: Option<&str> = None;
    raw.as_deref().and_then(parse_swapusage)
}

/// `vm.swapusage` output:
/// `total = 20480.00M  used = 18944.00M  free = 1536.00M (in bytes)`.
pub fn parse_swapusage(s: &str) -> Option<(u64, u64)> {
    let mut total = None;
    let mut used = None;
    let mut fields = s.split_whitespace();
    while let Some(f) = fields.next() {
        let slot = match f.trim_end_matches(':') {
            "total" => &mut total,
            "used" => &mut used,
            _ => continue,
        };
        // Skip the "=" between the label and the value.
        let v = match fields.next() {
            Some("=") => fields.next(),
            other => other,
        };
        *slot = v.and_then(parse_metric);
    }
    Some((used?, total?))
}

/// One size as macOS prints it: "18944.00M", "1.50G", "2048" (bytes).
fn parse_metric(s: &str) -> Option<u64> {
    let (num, unit) = s.split_at(s.find(|c: char| c.is_ascii_alphabetic()).unwrap_or(s.len()));
    let v: f64 = num.parse().ok()?;
    let m = match unit.trim_matches(|c| c == ',' || c == ')') {
        "" => 1.0,
        // Zero swap prints as "0B" — bytes, not the megabytes of the rest.
        "B" | "b" => 1.0,
        "K" => 1024.0,
        "M" => 1024.0 * 1024.0,
        "G" => 1024.0 * 1024.0 * 1024.0,
        "T" => 1024.0 * 1024.0 * 1024.0 * 1024.0,
        _ => return None,
    };
    (v.is_finite() && v >= 0.0).then(|| (v * m) as u64)
}

#[cfg(target_os = "macos")]
fn sysctl_string(name: &[u8]) -> Option<String> {
    let mut len = 0;
    let ok = unsafe {
        libc::sysctlbyname(
            name.as_ptr().cast(),
            std::ptr::null_mut(),
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    } == 0 && len > 1;
    if !ok {
        return None;
    }
    let mut buf = vec![0u8; len];
    let ok = unsafe {
        libc::sysctlbyname(
            name.as_ptr().cast(),
            buf.as_mut_ptr() as *mut libc::c_void,
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    } == 0;
    ok.then(|| String::from_utf8_lossy(&buf).trim_end_matches('\0').to_string())
}

/// Whether this Mac runs on battery power (`IOPSCopyPowerSourcesInfo`).
#[cfg(target_os = "macos")]
pub fn on_battery() -> bool {
    const KCFSTRING_ENCODING_UTF8: u32 = 0x0800_0100;
    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFStringGetCString(
            the_string: *const libc::c_void,
            buffer: *mut libc::c_char,
            buffer_size: isize,
            encoding: u32,
        ) -> libc::c_uchar;
        fn CFRelease(cf: *const libc::c_void);
    }
    #[link(name = "IOKit", kind = "framework")]
    extern "C" {
        fn IOPSCopyPowerSourcesInfo() -> *mut libc::c_void;
        // A constant string (kIOPSBatteryPowerValue…), not a new reference.
        fn IOPSGetProvidingPowerSourceType(info: *mut libc::c_void) -> *const libc::c_void;
    }
    unsafe {
        let info = IOPSCopyPowerSourcesInfo();
        if info.is_null() {
            return false;
        }
        let source = IOPSGetProvidingPowerSourceType(info);
        let mut buf = [0 as libc::c_char; 32];
        let battery = !source.is_null()
            && CFStringGetCString(source, buf.as_mut_ptr(), buf.len() as isize, KCFSTRING_ENCODING_UTF8) != 0
            && buf.split(|&b| b == 0).next().is_some_and(|s| s.iter().copied().eq(b"Battery Power".iter().map(|&c| c as libc::c_char)));
        CFRelease(info);
        battery
    }
}

#[cfg(not(target_os = "macos"))]
pub fn on_battery() -> bool {
    false
}

/// Free space (bytes) on the volume holding `dir`, as this process may write it.
#[cfg(unix)]
pub fn free_disk(dir: &Path) -> Option<u64> {
    #[cfg(any(test, feature = "test-seam"))]
    if let Some(bytes) = *TEST_FREE_DISK.read().expect("test free disk") {
        return Some(bytes);
    }
    use std::os::unix::ffi::OsStrExt;
    // The data directory may not exist yet when the initial sample runs.
    // Its nearest existing ancestor is on the volume the directory will use.
    let absolute = if dir.is_absolute() { dir.to_path_buf() } else { std::env::current_dir().ok()?.join(dir) };
    for ancestor in absolute.ancestors() {
        let path = std::ffi::CString::new(ancestor.as_os_str().as_bytes()).ok()?;
        let mut st: libc::statfs = unsafe { std::mem::zeroed() };
        if unsafe { libc::statfs(path.as_ptr(), &mut st) } == 0 {
            return Some(st.f_bavail as u64 * st.f_bsize as u64);
        }
    }
    None
}

#[cfg(not(unix))]
pub fn free_disk(_dir: &Path) -> Option<u64> {
    None
}

// ------------------------------------------------------------------ sizes on the command line

/// A size flag: a plain number is whole GB ("8"), a unit makes it exact
/// ("512M", "6G", "8589934592B"). 0 is allowed (it turns a limit off).
pub fn parse_size(s: &str) -> Result<u64, String> {
    let t = s.trim();
    let (num, unit) = t.split_at(t.find(|c: char| c.is_ascii_alphabetic()).unwrap_or(t.len()));
    let n: u64 = num.trim().parse().map_err(|_| format!("--size {t:?}: a number, then optionally K/M/G/B"))?;
    let mult = match unit.trim_end_matches('B').trim_end_matches('b') {
        "" => GB,
        "K" | "k" => 1024,
        "M" | "m" => 1024 * 1024,
        "G" | "g" => GB,
        other => return Err(format!("--size {t:?}: unknown unit {other:?} (K, M, G)")),
    };
    Ok(n.checked_mul(mult).filter(|v| *v < (1u64 << 50)).ok_or_else(|| format!("--size {t:?}: too large"))?)
}

/// The default prover cap: a quarter of the RAM, at least 4 GB.
pub fn default_prover_cap() -> u64 {
    prover_cap_for(physical_ram())
}

/// [`default_prover_cap`] for a machine with `ram` bytes of RAM.
pub fn prover_cap_for(ram: u64) -> u64 {
    (ram / 4).max(4 * GB)
}

/// Floor of the default history-cache budget.
pub const CACHE_BUDGET_MIN: u64 = GB;
/// Ceiling of the default history-cache budget.
pub const CACHE_BUDGET_MAX: u64 = 4 * GB;

/// The default budget for the node's own in-memory history caches.
pub fn default_cache_budget() -> u64 {
    cache_budget_for(physical_ram())
}

/// [`default_cache_budget`] for a machine with `ram` bytes of RAM: an eighth
/// of the RAM, between 1 GB and 4 GB. A consumer Mac runs the user's other
/// apps, and often several nodes at once (the wallet's follower plus
/// validators), and each node takes this whole budget for itself. The old
/// quarter of the RAM gave 16 GB per node on a 64 GB Mac. Trimming only drops
/// sealed eras whose files are on disk, so a small budget costs era-file reads
/// for old heights and never loses data. `--max-memory` still overrides it.
pub fn cache_budget_for(ram: u64) -> u64 {
    (ram / 8).clamp(CACHE_BUDGET_MIN, CACHE_BUDGET_MAX)
}

/// Everything the resource flags configure (docs/ops/resource-limits.md).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Limits {
    /// The prover sidecar's physical-footprint cap in bytes; 0 disables the prover.
    pub prover_max_memory: u64,
    /// Worker threads the sidecar's provers may use (RAYON_NUM_THREADS).
    pub prover_threads: usize,
    /// Whether proving may run on battery power (it pauses otherwise).
    pub prover_on_battery: bool,
    /// Budget for this node's own in-memory history caches (summaries, receipts).
    pub max_memory: u64,
    /// Free-space floor for the data volume: below it consensus participation,
    /// finalized commits, new era/shard files, and proving pause. 0 disables the guard.
    pub min_free_disk: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            prover_max_memory: default_prover_cap(),
            prover_threads: (cores() / 2).max(1),
            prover_on_battery: false,
            max_memory: default_cache_budget(),
            min_free_disk: 5 * GB,
        }
    }
}

impl Limits {
    /// From command-line values (None = not given): "auto" takes the default.
    pub fn resolve(
        prover_max_memory: Option<&str>,
        prover_threads: Option<usize>,
        prover_on_battery: bool,
        max_memory: Option<&str>,
        min_free_disk: Option<&str>,
    ) -> Result<Limits, String> {
        let auto = |v: Option<&str>, what: &str, default: u64| -> Result<u64, String> {
            match v {
                None | Some("auto") => Ok(default),
                Some(s) => parse_size(s).map_err(|e| format!("{what}: {}", e.replace("--size", what))),
            }
        };
        Ok(Limits {
            prover_max_memory: auto(prover_max_memory, "--prover-max-memory", default_prover_cap())?,
            prover_threads: prover_threads.unwrap_or((cores() / 2).max(1)).max(1),
            prover_on_battery,
            max_memory: auto(max_memory, "--max-memory", default_cache_budget())?,
            min_free_disk: auto(min_free_disk, "--min-free-disk", 5 * GB)?,
        })
    }
}

// ------------------------------------------------------------------ the monitor

/// One system sample (a probe that failed keeps the previous value).
#[derive(Clone, Copy, Default)]
pub struct Sample {
    pub free_disk: Option<u64>,
    pub pressure: Option<u8>,
    pub battery: bool,
    /// Swap in use, per-mille of the total.
    pub swap_permille: Option<u16>,
}

#[derive(Default)]
struct State {
    free_disk: Option<u64>,
    disk_low: bool,
    disk_warned: bool,
    pressure: Option<u8>,
    battery: bool,
    swap_permille: Option<u16>,
    /// Why proving is paused system-wide ("memory" is the sidecar gate's, in prover.rs).
    proving: Option<&'static str>,
    /// When the system was last seen all-normal (None while paused).
    normal_since: Option<Instant>,
    footprint_logged: Option<Instant>,
}

/// The process-wide resource state, sampled by a watchdog thread and read by
/// the disk guard (chain, shards), the prover service and `aether_status`.
pub struct Monitor {
    pub limits: Limits,
    dir: PathBuf,
    state: Mutex<State>,
}

impl Monitor {
    /// One sample applied: disk-guard and proving-pause transitions are
    /// logged here, once each. `now` is a parameter so tests can move time.
    pub fn apply(&self, s: Sample, now: Instant) {
        let mut st = self.state.lock().expect("resource state");
        if s.free_disk.is_none() && self.limits.min_free_disk > 0 && st.free_disk.is_some() {
            tracing::error!("data volume free space could not be measured; pausing writes");
        }
        st.free_disk = s.free_disk;
        if s.free_disk.is_none() && self.limits.min_free_disk > 0 {
            st.disk_low = true;
            st.disk_warned = false;
        }
        if let Some(free) = s.free_disk {
            let warning = self.limits.min_free_disk > 0
                && free < self.limits.min_free_disk.saturating_add(DISK_WARN_AHEAD);
            if warning && !st.disk_warned {
                tracing::warn!(free, floor = self.limits.min_free_disk, "disk almost full: free space is approaching the node's write floor");
            }
            st.disk_warned = warning;
            if self.limits.min_free_disk > 0 && free < self.limits.min_free_disk && !st.disk_low {
                st.disk_low = true;
                tracing::warn!(free, min = self.limits.min_free_disk, "disk almost full: consensus participation, finalized commits, history writes and proving paused");
            } else if free >= self.limits.min_free_disk.saturating_add(DISK_RESUME) && st.disk_low {
                st.disk_low = false;
                tracing::info!(free, "disk space recovered: consensus participation, finalized commits, history writes and proving resume");
            }
        }
        if s.pressure.is_some() {
            st.pressure = s.pressure;
        }
        st.battery = s.battery;
        if s.swap_permille.is_some() {
            st.swap_permille = s.swap_permille;
        }
        let reason = if st.disk_low {
            Some("disk")
        } else if st.pressure.is_some_and(|p| p >= PRESSURE_WARN) || st.swap_permille.is_some_and(|s| s > SWAP_PAUSE_PERMILLE) {
            Some("pressure")
        } else if st.battery && !self.limits.prover_on_battery {
            Some("battery")
        } else {
            None
        };
        match reason {
            Some(r) => {
                if st.proving.is_none() {
                    tracing::warn!(reason = r, pressure = ?st.pressure, swap_permille = ?st.swap_permille, battery = st.battery, "pausing proving (no new jobs; the running one is killed on critical pressure)");
                }
                st.proving = Some(r);
                st.normal_since = None;
            }
            // The disk guard's own 2 GB hysteresis is the debounce: once the
            // floor is cleared, proving resumes at once (the five-minute dwell
            // below is for the conditions that flap — pressure, battery).
            None if st.proving.is_some_and(|p| p == "disk") => {
                tracing::info!("the disk is above the floor again: proving resumes");
                st.proving = None;
                st.normal_since = None;
            }
            None if st.proving.is_some() => {
                let since = *st.normal_since.get_or_insert(now);
                if now.duration_since(since) >= RESUME_AFTER {
                    tracing::info!("the system has been back to normal for five minutes: proving resumes");
                    st.proving = None;
                    st.normal_since = None;
                }
            }
            None => {}
        }
    }

    /// Whether the data volume is below its configured free-space floor.
    pub fn disk_low(&self) -> bool {
        let st = self.state.lock().expect("resource state");
        self.limits.min_free_disk > 0 && (st.disk_low || st.free_disk.is_none())
    }

    pub fn disk_warned(&self) -> bool {
        self.state.lock().expect("resource state").disk_warned
    }

    /// Why proving is paused system-wide (None: it may run).
    pub fn proving_pause(&self) -> Option<&'static str> {
        self.state.lock().expect("resource state").proving
    }

    /// Memory pressure at "critical": a running proof is killed, not just finished.
    pub fn critical(&self) -> bool {
        self.state.lock().expect("resource state").pressure == Some(PRESSURE_CRITICAL)
    }

    /// The `aether_status` view of this node's resources.
    pub fn status_value(&self) -> Value {
        let st = self.state.lock().expect("resource state");
        let paused = self.limits.min_free_disk > 0 && (st.disk_low || st.free_disk.is_none());
        json!({
            "disk_free": st.free_disk,
            // The shipping wallet reads disk_low to show its disk warning.
            // Raise it at the early warning threshold, before writes pause.
            "disk_low": paused || st.disk_warned,
            "disk_paused": paused,
            "disk_status": if st.free_disk.is_none() { "unknown" } else if st.disk_low { "paused" } else if st.disk_warned { "almost_full" } else { "ok" },
            "disk_almost_full": st.disk_warned,
            "min_free_disk": self.limits.min_free_disk,
            "resume_free_disk": self.limits.min_free_disk.saturating_add(DISK_RESUME),
            "proving_paused": st.proving,
        })
    }

    /// The watchdog body: sample, apply, log the node's own footprint.
    fn watch(self: Arc<Self>) -> ! {
        loop {
            self.apply(Sample {
                free_disk: free_disk(&self.dir),
                // The kernel's own level, and only where it stays silent the
                // free+inactive reading (macOS caches aggressively, so the
                // page count alone would warn on a healthy machine).
                pressure: pressure_level().or_else(pages_pressure_now),
                battery: on_battery(),
                swap_permille: swap_usage().map(|(used, total)| if total > 0 { (used * 1000 / total) as u16 } else { 0 }),
            }, Instant::now());
            if self.disk_low() {
                tracing::error!("disk write floor reached: stopping node child before journal and archive writes; supervisor will resume when space returns");
                std::process::exit(crate::supervisor::EXIT_DISK_LOW);
            }
            {
                let mut st = self.state.lock().expect("resource state");
                if st.footprint_logged.is_none_or(|t| t.elapsed() >= FOOTPRINT_EVERY) {
                    st.footprint_logged = Some(Instant::now());
                    tracing::info!(
                        footprint_bytes = footprint(std::process::id()).unwrap_or_default(),
                        "this node's memory (physical footprint, every 10 min)"
                    );
                }
            }
            std::thread::sleep(CHECK);
        }
    }
}

static MONITOR: RwLock<Option<Arc<Monitor>>> = RwLock::new(None);

/// Install the process-wide monitor and start its watchdog. The node and
/// follow commands each call it once, before the chain opens.
pub fn install(limits: Limits, dir: impl Into<PathBuf>) -> Arc<Monitor> {
    let m = Arc::new(Monitor { limits, dir: dir.into(), state: Mutex::new(State::default()) });
    // Chain and marshal open immediately after install. Sample synchronously so
    // they cannot begin writing during the watchdog's first scheduling delay.
    m.apply(Sample { free_disk: free_disk(&m.dir), ..Default::default() }, Instant::now());
    if m.disk_low() {
        tracing::error!("disk write floor reached before startup: stopping child until supervisor sees free space");
        std::process::exit(crate::supervisor::EXIT_DISK_LOW);
    }
    *MONITOR.write().expect("resource monitor") = Some(m.clone());
    let watchdog = m.clone();
    std::thread::Builder::new()
        .name("resources".into())
        .spawn(move || watchdog.watch())
        .expect("start the resource watchdog");
    m
}

/// The running monitor, if this process installed one.
pub fn monitor() -> Option<Arc<Monitor>> {
    MONITOR.read().expect("resource monitor").clone()
}

/// Exercise pause/resume decisions in an isolated test process without the
/// watchdog terminating that process at the disk floor.
#[cfg(any(test, feature = "test-seam"))]
pub fn install_test_monitor(limits: Limits, dir: impl Into<PathBuf>) {
    let m = Arc::new(Monitor { limits, dir: dir.into(), state: Mutex::new(State::default()) });
    *MONITOR.write().expect("resource monitor") = Some(m);
}

/// Whether new writes on the data volume may begin (no monitor: always).
pub fn disk_ok() -> bool {
    monitor().is_none_or(|m| {
        // A decision to vote or commit must see fresh space, even if another
        // process filled the volume between the watchdog's two-second samples.
        let free = free_disk(&m.dir);
        if free.is_none() && m.limits.min_free_disk > 0 { return false; }
        let battery = m.state.lock().expect("resource state").battery;
        m.apply(Sample { free_disk: free, battery, ..Default::default() }, Instant::now());
        !m.disk_low()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn a_new_relative_data_dir_still_has_a_measurable_volume() {
        assert!(free_disk(Path::new("new-node-that-does-not-exist")).is_some_and(|bytes| bytes > 0));
    }

    #[test]
    fn snapshots_use_a_quarter_of_free_memory_and_the_configured_cap() {
        assert_eq!(snapshot_memory_budget_for(2 * GB, 2 * GB), GB / 2);
        assert_eq!(snapshot_memory_budget_for(16 * GB, GB), GB);
        assert_eq!(snapshot_memory_budget_for(0, GB), 0);
    }

    /// The snapshot gate's pressure policy (docs/design/05-state.md): critical
    /// always refuses; warn is the budget check, not a refusal of its own, so
    /// a busy-but-healthy Mac still serves checkpoint sync; normal is
    /// unchanged. A kernel that stays silent behaves as normal.
    #[test]
    fn warn_defers_to_the_budget_and_critical_refuses_outright() {
        // A quarter of 8 GB available is 2 GB, and the cap is 2 GB: budget 2 GB.
        let (available, configured) = (8 * GB, 2 * GB);
        for pressure in [None, Some(PRESSURE_NORMAL), Some(PRESSURE_WARN)] {
            assert!(snapshot_gate_for(pressure, 2 * GB, available, configured).is_ok(), "{pressure:?}: a fitting build runs");
            assert_eq!(snapshot_budget_for(pressure, available, configured).unwrap(), 2 * GB, "{pressure:?}: the budget itself is unchanged");
        }
        // One byte over the budget: refused at warn exactly as at normal.
        for pressure in [Some(PRESSURE_NORMAL), Some(PRESSURE_WARN)] {
            let err = snapshot_gate_for(pressure, 2 * GB + 1, available, configured).unwrap_err();
            assert!(err.contains("exceeds budget"), "{pressure:?}: {err}");
        }
        // Critical refuses whatever the headroom, with no budget arithmetic.
        let err = snapshot_gate_for(Some(PRESSURE_CRITICAL), 1, 64 * GB, 64 * GB).unwrap_err();
        assert!(err.contains("critical"), "{err}");
    }

    /// The seam replaces the host's readings for the gate — whatever this
    /// machine is really under — and clearing restores the host.
    #[test]
    fn the_test_seam_pins_the_readings_the_gate_sees() {
        let _seam = SEAM.lock().unwrap_or_else(|e| e.into_inner());
        set_test_readings(Some(PRESSURE_CRITICAL), None);
        assert!(snapshot_memory_budget().unwrap_err().contains("critical"));
        // A quarter of 4 GB is 1 GB, the default budget's floor, so the
        // result does not depend on this host's RAM.
        set_test_readings(Some(PRESSURE_WARN), Some(4 * GB));
        assert_eq!(snapshot_memory_budget().unwrap(), GB, "warn with pinned headroom budgets normally");
        assert!(snapshot_build_gate(GB).is_ok(), "a fitting estimate builds under pinned warn");
        assert!(snapshot_build_gate(GB + 1).is_err(), "a non-fitting estimate refuses under pinned warn");
        clear_test_readings();
        set_test_readings(None, Some(4 * GB));
        assert_eq!(snapshot_memory_budget().unwrap(), GB, "a pinned available memory alone also pins the budget");
        clear_test_readings();
    }

    fn monitor_with(limits: Limits) -> Monitor {
        Monitor { limits, dir: PathBuf::from("."), state: Mutex::new(State::default()) }
    }

    #[test]
    fn sizes_parse_gb_plain_and_unit_exact() {
        assert_eq!(parse_size("8").unwrap(), 8 * GB);
        assert_eq!(parse_size("0").unwrap(), 0);
        assert_eq!(parse_size("512M").unwrap(), 512 * 1024 * 1024);
        assert_eq!(parse_size("4GB").unwrap(), 4 * GB);
        assert_eq!(parse_size("6g").unwrap(), 6 * GB);
        assert_eq!(parse_size("1048576K").unwrap(), GB);
        assert!(parse_size("eight").is_err());
        assert!(parse_size("8X").is_err());
        assert!(parse_size("-1").is_err());
        assert!(parse_size("999999999").is_err(), "absurd sizes are rejected, not wrapped");
    }

    #[test]
    fn swapusage_parses_used_and_total() {
        assert_eq!(
            parse_swapusage("total = 20480.00M  used = 18944.00M  free = 1536.00M (in bytes)").unwrap(),
            (18944 * 1024 * 1024, 20480 * 1024 * 1024)
        );
        assert_eq!(parse_swapusage("total = 0B  used = 0B  free = 0B").unwrap(), (0, 0));
        assert!(parse_swapusage("total = 1.00M").is_none(), "no used value");
    }

    /// The default cache budget per machine size: an eighth of the RAM,
    /// between 1 GB and 4 GB, so several nodes on one consumer Mac fit.
    #[test]
    fn the_cache_budget_is_an_eighth_of_the_ram_between_1_and_4_gb() {
        assert_eq!(cache_budget_for(8 * GB), GB);
        assert_eq!(cache_budget_for(16 * GB), 2 * GB);
        assert_eq!(cache_budget_for(24 * GB), 3 * GB);
        assert_eq!(cache_budget_for(64 * GB), 4 * GB);
        assert_eq!(cache_budget_for(128 * GB), 4 * GB);
        assert_eq!(cache_budget_for(4 * GB), GB, "the 1 GB floor holds on small machines");
        assert_eq!(cache_budget_for(0), GB, "an unreadable RAM size still gets the floor");
        assert_eq!(default_cache_budget(), cache_budget_for(physical_ram()));
        assert_eq!(Limits::default().max_memory, default_cache_budget());
    }

    /// The prover cap is unchanged: a quarter of the RAM, at least 4 GB.
    #[test]
    fn the_prover_cap_is_a_quarter_of_the_ram_at_least_4_gb() {
        assert_eq!(prover_cap_for(8 * GB), 4 * GB);
        assert_eq!(prover_cap_for(16 * GB), 4 * GB);
        assert_eq!(prover_cap_for(24 * GB), 6 * GB);
        assert_eq!(prover_cap_for(64 * GB), 16 * GB);
        assert_eq!(default_prover_cap(), prover_cap_for(physical_ram()));
    }

    /// `--max-memory` overrides the default in both directions.
    #[test]
    fn max_memory_overrides_the_default_budget() {
        assert_eq!(Limits::resolve(None, None, false, Some("16"), None).unwrap().max_memory, 16 * GB);
        assert_eq!(Limits::resolve(None, None, false, Some("256M"), None).unwrap().max_memory, 256 * 1024 * 1024);
        assert_eq!(Limits::resolve(None, None, false, Some("auto"), None).unwrap().max_memory, default_cache_budget());
    }

    #[test]
    fn limits_resolve_defaults_and_values() {
        let d = Limits::resolve(None, None, false, None, None).unwrap();
        assert_eq!(d, Limits::default());
        assert!(d.prover_max_memory >= 4 * GB);
        assert_eq!(d.prover_threads, (cores() / 2).max(1));
        assert_eq!(d.min_free_disk, 5 * GB);
        let l = Limits::resolve(Some("0"), Some(cores()), true, Some("512M"), Some("0")).unwrap();
        assert_eq!(l.prover_max_memory, 0, "0 means the prover never runs");
        assert_eq!(l.prover_threads, cores(), "the user may raise the thread cap");
        assert!(l.prover_on_battery);
        assert_eq!(l.max_memory, 512 * 1024 * 1024);
        assert_eq!(l.min_free_disk, 0, "0 turns the disk guard off");
        assert_eq!(Limits::resolve(Some("auto"), None, false, None, None).unwrap(), Limits::default());
        assert!(Limits::resolve(Some("junk"), None, false, None, None).is_err());
    }

    #[test]
    fn the_disk_guard_trips_and_resumes_with_hysteresis() {
        let m = monitor_with(Limits { min_free_disk: 5 * GB, ..Limits::default() });
        let now = Instant::now();
        assert!(m.disk_low(), "unknown initial disk space fails closed");
        m.apply(Sample { free_disk: Some(6 * GB), ..Default::default() }, now);
        assert!(m.disk_warned(), "wallet gets an early warning before the write floor");
        assert_eq!(m.status_value()["disk_low"], true);
        assert_eq!(m.status_value()["disk_paused"], false);
        assert!(!m.disk_low());
        m.apply(Sample { free_disk: Some(5 * GB - 1), ..Default::default() }, now);
        assert!(m.disk_low(), "below the floor: no new history files");
        assert_eq!(m.status_value()["disk_status"], "paused");
        assert_eq!(m.status_value()["disk_paused"], true);
        assert_eq!(m.status_value()["resume_free_disk"], 7 * GB);
        assert_eq!(m.proving_pause(), Some("disk"));
        m.apply(Sample { free_disk: Some(6 * GB), ..Default::default() }, now);
        assert!(m.disk_low(), "above the floor but below floor+2 GB: still holding");
        assert_eq!(m.proving_pause(), Some("disk"));
        m.apply(Sample { free_disk: Some(7 * GB), ..Default::default() }, now);
        assert!(!m.disk_low(), "floor + 2 GB: writes resume");
        assert_eq!(m.status_value()["disk_status"], "almost_full", "warning remains until the extra margin returns");
        m.apply(Sample { free_disk: Some(8 * GB), ..Default::default() }, now);
        assert_eq!(m.status_value()["disk_status"], "ok");
        assert_eq!(m.proving_pause(), None, "and proving is no longer paused for the disk");
        m.apply(Sample::default(), now);
        assert!(m.disk_low(), "a failed volume measurement pauses writes");
        assert_eq!(m.status_value()["disk_status"], "unknown");
        // A guard turned off (0) never trips.
        let off = monitor_with(Limits { min_free_disk: 0, ..Limits::default() });
        off.apply(Sample { free_disk: Some(0), ..Default::default() }, now);
        assert!(!off.disk_low());
    }

    #[test]
    fn proving_pauses_immediately_and_resumes_after_five_normal_minutes() {
        // This test isolates memory/battery behavior; unknown disk readings
        // now fail closed when the disk guard is enabled.
        let m = monitor_with(Limits { min_free_disk: 0, ..Limits::default() });
        let t0 = Instant::now();
        m.apply(Sample { pressure: Some(PRESSURE_WARN), ..Default::default() }, t0);
        assert_eq!(m.proving_pause(), Some("pressure"));
        // Swap past half also pauses, and beats battery in what gets reported.
        m.apply(Sample { pressure: Some(PRESSURE_NORMAL), swap_permille: Some(600), battery: true, ..Default::default() }, t0);
        assert_eq!(m.proving_pause(), Some("pressure"));
        // Battery alone pauses unless allowed.
        m.apply(Sample { pressure: Some(PRESSURE_NORMAL), swap_permille: Some(0), battery: true, ..Default::default() }, t0);
        assert_eq!(m.proving_pause(), Some("battery"));
        let allowed = monitor_with(Limits { prover_on_battery: true, min_free_disk: 0, ..Limits::default() });
        allowed.apply(Sample { battery: true, ..Default::default() }, t0);
        assert_eq!(allowed.proving_pause(), None);
        // Normal again: only after five uninterrupted minutes does proving
        // resume (the clock starts at the first normal sample, t0 + 60).
        m.apply(Sample { pressure: Some(PRESSURE_NORMAL), swap_permille: Some(0), ..Default::default() }, t0 + Duration::from_secs(60));
        assert_eq!(m.proving_pause(), Some("battery"), "normal for no time yet");
        m.apply(Sample { pressure: Some(PRESSURE_NORMAL), swap_permille: Some(0), ..Default::default() }, t0 + Duration::from_secs(60 + 4 * 60));
        assert_eq!(m.proving_pause(), Some("battery"), "four minutes are not five");
        m.apply(Sample { pressure: Some(PRESSURE_NORMAL), swap_permille: Some(0), ..Default::default() }, t0 + Duration::from_secs(60 + 5 * 60));
        assert_eq!(m.proving_pause(), None, "five normal minutes: resumed");
        // A blip back to warn restarts the clock, it does not pause forever.
        m.apply(Sample { pressure: Some(PRESSURE_WARN), ..Default::default() }, t0 + Duration::from_secs(60 + 6 * 60));
        assert_eq!(m.proving_pause(), Some("pressure"));
        m.apply(Sample { pressure: Some(PRESSURE_NORMAL), ..Default::default() }, t0 + Duration::from_secs(60 + 7 * 60));
        m.apply(Sample { pressure: Some(PRESSURE_NORMAL), ..Default::default() }, t0 + Duration::from_secs(60 + 13 * 60));
        assert_eq!(m.proving_pause(), None);
    }

    #[test]
    fn the_page_count_fallback_maps_free_and_inactive_to_a_level() {
        let (ram, page) = (16 * GB, 16 * 1024);
        let pages = |n: u64| n / page;
        // Plenty free: normal.
        assert_eq!(pages_pressure(pages(8 * GB), pages(4 * GB), page, ram), Some(PRESSURE_NORMAL));
        // Under 10%: warn. Under 5%: critical.
        assert_eq!(pages_pressure(pages(GB), pages(GB / 2), page, ram), Some(PRESSURE_WARN), "9% of 16 GB warns");
        assert_eq!(pages_pressure(pages(400 * 1024 * 1024), 0, page, ram), Some(PRESSURE_CRITICAL), "2.4% is critical");
        assert_eq!(pages_pressure(pages(2 * GB), 0, page, ram), Some(PRESSURE_NORMAL), "12% is normal");
        // Nonsense in never kills the prover.
        assert_eq!(pages_pressure(1, 1, 0, ram), None);
        assert_eq!(pages_pressure(1, 1, page, 0), None);
    }

    #[test]
    fn critical_pressure_is_distinguishable() {
        let m = monitor_with(Limits::default());
        let now = Instant::now();
        assert!(!m.critical());
        m.apply(Sample { pressure: Some(PRESSURE_WARN), ..Default::default() }, now);
        assert!(!m.critical());
        m.apply(Sample { pressure: Some(PRESSURE_CRITICAL), ..Default::default() }, now);
        assert!(m.critical());
    }

    #[test]
    fn the_machine_reads_back_something() {
        assert!(physical_ram() > 0);
        assert!(cores() >= 1);
        assert!(footprint(std::process::id()).is_some_and(|f| f > 0), "the kernel answers for our own pid");
        assert!(free_disk(Path::new("/")).is_some_and(|f| f > 0));
        assert!(matches!(pressure_level(), None | Some(PRESSURE_NORMAL | PRESSURE_WARN | PRESSURE_CRITICAL)));
        // The fallback is a real reading too, not a sysctl name that stopped
        // existing (vm.page_inactive_count did, on Darwin 24).
        #[cfg(target_os = "macos")]
        assert!(matches!(pages_pressure_now(), Some(PRESSURE_NORMAL | PRESSURE_WARN | PRESSURE_CRITICAL)));
    }
}
