//! One data directory, one node (docs/design/29-unattended-restart.md): the
//! `run.lock` the supervisor takes is what keeps an app-started node and a
//! daemon-started node from ever running together — the late one exits 7
//! (`EXIT_LOCKED`) at once, "already running", not a crash to restart. This
//! exercises the real binary end to end: a second `aether run` against a
//! held directory is refused, and the lock frees when the holder dies, so
//! the next run goes through (and then says what a bare directory misses).

use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

/// A port nothing is listening on (the node opens its own; the test only
/// needs two that will not collide with anything on this Mac).
fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

/// Temp directory removed however the test ends.
struct TmpDir(PathBuf);
impl TmpDir {
    fn new(tag: &str) -> Self {
        let p = std::env::temp_dir().join(format!("aether-run-lock-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        TmpDir(p)
    }
}
impl Drop for TmpDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Run the real `aether run` against `data`, wait up to `secs` for it to end,
/// and return its status (None = still running, killed by the test) with
/// everything it printed. Output goes to a file, not a pipe: a refused run
/// must be observable even under load, without a 64 KB pipe ever holding it.
fn run_once(data: &std::path::Path, secs: u64) -> (Option<ExitStatus>, String) {
    let log_path = data.join("second-run.log");
    let log = std::fs::OpenOptions::new().create(true).write(true).truncate(true)
        .open(&log_path).expect("open the run's log");
    let mut child = Command::new(env!("CARGO_BIN_EXE_aether"))
        .arg("run")
        .arg("--data").arg(data)
        .arg("--rpc-port").arg(free_port().to_string())
        .arg("--port").arg(free_port().to_string())
        .stdout(Stdio::from(log.try_clone().expect("clone the log handle")))
        .stderr(Stdio::from(log))
        .spawn()
        .expect("spawn the aether binary");
    let deadline = Instant::now() + Duration::from_secs(secs);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return (Some(status), std::fs::read_to_string(&log_path).unwrap_or_default());
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            return (None, std::fs::read_to_string(&log_path).unwrap_or_default());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn a_second_run_on_a_held_data_directory_exits_locked() {
    let dir = TmpDir::new("held");
    let _lock = aether_node::supervisor::lock_data_dir(&dir.0).unwrap();
    // Generous deadline: this binary also runs under whatever load the rest
    // of the test suite puts on the Mac; the refusal itself, not its speed,
    // is the contract.
    let (status, out) = run_once(&dir.0, 120);
    let status = status.expect("a refused run ends instead of running");
    assert_eq!(status.code(), Some(aether_node::supervisor::EXIT_LOCKED),
        "exit 7 is \"already running\", not a crash: {out}");
    assert!(out.contains("run.lock"), "the refusal names the holder's lock: {out}");
}

#[test]
fn the_lock_frees_when_the_holder_dies() {
    let dir = TmpDir::new("freed");
    {
        let _lock = aether_node::supervisor::lock_data_dir(&dir.0).unwrap();
    }
    let (status, out) = run_once(&dir.0, 120);
    match status {
        // Still running at the deadline: it took the freed lock and went to
        // work — exactly what the daemon's node does after a restart.
        None => {}
        // This bare directory has no network.json, so the run stops there with
        // the "first run" sentence — past the lock, which is the point.
        Some(status) => {
            assert_ne!(status.code(), Some(aether_node::supervisor::EXIT_LOCKED),
                "a freed lock must let a run through: {out}");
            assert!(out.contains("first run"), "a bare directory says what it misses next: {out}");
        }
    }
}
