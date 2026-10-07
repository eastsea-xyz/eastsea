//! The Mac app sends SIGUSR1 to `aether run` (the supervisor) on every
//! power-source change, sleep and wake, to wake the beacon loop. Before
//! 2026-10-07 only the children handled it: the default disposition killed
//! the supervisor silently, and the node stopped with no line in node.log.
//! The supervisor now survives the signal and forwards it to its child.

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn wait_exit(child: &mut std::process::Child, secs: u64) -> Option<std::process::ExitStatus> {
    let deadline = Instant::now() + Duration::from_secs(secs);
    loop {
        if let Some(s) = child.try_wait().unwrap() { return Some(s); }
        if Instant::now() > deadline { return None; }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// The real binary, parked before any child exists (a disk floor no volume
/// meets keeps it waiting): SIGUSR1 must not end it.
#[test]
fn the_supervisor_survives_the_apps_wake_signal() {
    let dir = std::env::temp_dir().join(format!("aether-wake-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_aether"))
        .args(["run", "--data"]).arg(&dir)
        .args(["--rpc-port", "1", "--port", "2", "--min-free-disk=900000G"])
        .stdout(Stdio::null()).stderr(Stdio::null())
        .spawn().expect("spawn aether");
    // Let it reach the disk wait (the handler is installed first thing).
    std::thread::sleep(Duration::from_secs(2));
    assert!(child.try_wait().unwrap().is_none(), "parked in the disk wait");
    for _ in 0..3 {
        unsafe { libc::kill(child.id() as i32, libc::SIGUSR1) };
        std::thread::sleep(Duration::from_millis(300));
    }
    let survived = child.try_wait().unwrap().is_none();
    let _ = child.kill();
    let status = wait_exit(&mut child, 10);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(survived, "SIGUSR1 killed the supervisor: {status:?}");
}

/// The forwarded wake reaches the current child (here a stand-in that dies
/// of SIGUSR1's default action, which makes the delivery observable).
#[test]
fn the_wake_signal_is_forwarded_to_the_child() {
    aether_node::supervisor::install_wake_forwarding();
    let mut stand_in = Command::new("/bin/sleep").arg("30").spawn().unwrap();
    aether_node::supervisor::set_wake_target(Some(stand_in.id()));
    unsafe { libc::raise(libc::SIGUSR1) };
    let status = wait_exit(&mut stand_in, 10);
    aether_node::supervisor::set_wake_target(None);
    if status.is_none() { let _ = stand_in.kill(); let _ = stand_in.wait(); }
    use std::os::unix::process::ExitStatusExt as _;
    assert_eq!(status.and_then(|s| s.signal()), Some(libc::SIGUSR1), "the child got the wake");
}
