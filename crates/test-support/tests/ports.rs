use aether_test_support::{Port, TestChild};
use std::collections::HashSet;
use std::fs::OpenOptions;
use std::io::Write;
use std::net::{TcpListener, UdpSocket};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

fn temp(tag: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("aether-ports-{}-{tag}", std::process::id()));
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn wait_until(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !condition() {
        assert!(Instant::now() < deadline, "test worker timed out");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn worker(name: &str, dir: &Path) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", name, "--ignored", "--nocapture"])
        .env("AETHER_PORT_WORKER", dir);
    command
}

// Invoked only by the parent regression in another process. All leases stay
// alive until the parent has compared the two independently allocated sets.
#[test]
#[ignore = "subprocess fixture"]
fn port_worker() {
    let dir = PathBuf::from(std::env::var_os("AETHER_PORT_WORKER").unwrap());
    wait_until(|| dir.join("start").exists());
    let mut leases = Vec::new();
    for _ in 0..2048 {
        leases.push(Port::reserve().unwrap());
        // Exercise reuse/wraparound without keeping tens of thousands of
        // descriptors open. The old bind-0/drop allocator reused live test
        // ports once the OS ephemeral sequence wrapped on this Mac.
        for _ in 0..7 {
            drop(Port::reserve().unwrap());
        }
    }
    let ports = leases
        .iter()
        .map(|port| port.port().to_string())
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(dir.join("ports.pending"), ports).unwrap();
    std::fs::rename(dir.join("ports.pending"), dir.join("ports")).unwrap();
    wait_until(|| dir.join("release").exists());
    drop(leases);
}

#[test]
fn reservations_are_unique_across_processes_and_tmpdirs() {
    let root = temp("concurrent");
    let dirs = [root.join("lane-a"), root.join("lane-b")];
    let children: Vec<TestChild> = dirs
        .iter()
        .map(|dir| {
            std::fs::create_dir_all(dir).unwrap();
            let mut command = worker("port_worker", dir);
            command.env("TMPDIR", dir);
            TestChild::spawn(command, dir.join("worker.log")).unwrap()
        })
        .collect();
    for dir in &dirs {
        std::fs::write(dir.join("start"), []).unwrap();
    }
    wait_until(|| {
        for child in &children {
            assert!(
                child.try_wait().unwrap().is_none(),
                "allocation worker exited early"
            );
        }
        dirs.iter().all(|dir| dir.join("ports").exists())
    });
    let ports: Vec<Vec<u16>> = dirs
        .iter()
        .map(|dir| {
            std::fs::read_to_string(dir.join("ports"))
                .unwrap()
                .lines()
                .map(|line| line.parse().unwrap())
                .collect()
        })
        .collect();
    let sets: Vec<HashSet<u16>> = ports
        .iter()
        .map(|ports| ports.iter().copied().collect())
        .collect();
    let shared = sets[0].intersection(&sets[1]).count();
    assert_eq!(
        shared, 0,
        "port collision across two harness processes: {shared} shared ports"
    );
    for set in &sets {
        assert_eq!(set.len(), 2048, "port collision within a harness process");
    }
    // Reservations survive even when no TCP server has been started yet.
    for port in ports.iter().flatten() {
        assert!(
            UdpSocket::bind(("127.0.0.1", *port)).is_err(),
            "lease {port} was dropped early"
        );
        assert!(
            TcpListener::bind(("127.0.0.1", *port)).is_ok(),
            "lease must leave TCP available"
        );
    }
    for dir in &dirs {
        std::fs::write(dir.join("release"), []).unwrap();
    }
    for child in children {
        assert!(child.wait().unwrap().success());
    }
    // Released ports belong to the shared pool again. Another lane may
    // reserve them immediately; probing every released number here would
    // mistake that legitimate reuse for a leaked lease.
    eprintln!(
        "2 processes x 2048 held ports, 32768 allocation calls, 0 collisions; distinct TMPDIRs"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
#[ignore = "subprocess fixture"]
fn bind_worker() {
    let dir = PathBuf::from(std::env::var_os("AETHER_PORT_WORKER").unwrap());
    let mode = std::env::var("AETHER_BIND_MODE").unwrap();
    writeln!(
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join("attempts"))
            .unwrap(),
        "attempt"
    )
    .unwrap();
    if mode == "real-error" {
        eprintln!("failed to load network: invalid genesis");
        std::process::exit(42);
    }
    let addr = std::env::var("AETHER_BIND_ADDR").unwrap();
    let _listener = match TcpListener::bind(&addr) {
        Ok(listener) => listener,
        Err(e) => {
            println!("first-attempt-only: output before the bind failure");
            eprintln!("failed to bind listener: {e}");
            std::fs::write(dir.join("failed"), []).unwrap();
            if mode == "live-bind-error" {
                // Commonware's listener actor can die without exiting the
                // process; both startup attempts must still be observed.
                std::thread::sleep(Duration::from_secs(60));
            }
            std::process::exit(41);
        }
    };
    if mode == "late-error" {
        std::fs::write(dir.join("ready"), []).unwrap();
        wait_until(|| dir.join("stop").exists());
        eprintln!("failed to bind listener: BindFailed");
        std::process::exit(41);
    }
}

fn start_bind_worker(dir: &Path, port: &Port, mode: &str) -> TestChild {
    std::fs::create_dir_all(dir).unwrap();
    let mut command = worker("bind_worker", dir);
    command
        .env("AETHER_BIND_ADDR", port.addr().to_string())
        .env("AETHER_BIND_MODE", mode);
    TestChild::spawn(command, dir.join("node.log")).unwrap()
}

fn attempts(dir: &Path) -> usize {
    std::fs::read_to_string(dir.join("attempts"))
        .unwrap()
        .lines()
        .count()
}

#[test]
fn startup_retry_is_once_bind_only_and_observable() {
    let root = temp("retry");
    let port = Port::reserve().unwrap();
    let blocker = port.bind_tcp().unwrap();
    let dir = root.join("transient");
    let child = start_bind_worker(&dir, &port, "transient");
    wait_until(|| dir.join("failed").exists());
    drop(blocker);
    let output = child.wait_with_output().unwrap();
    let status = output.status;
    assert!(
        status.success(),
        "a transient bind failure must get one startup retry: {status}"
    );
    assert_eq!(attempts(&dir), 2);
    assert!(
        !String::from_utf8_lossy(&output.stdout).contains("first-attempt-only"),
        "failed-attempt output must not be returned as the final attempt's output"
    );
    assert!(std::fs::read_to_string(dir.join("node.log"))
        .unwrap()
        .contains("retrying once (1/1)"));

    let _blocker = port.bind_tcp().unwrap();
    let dir = root.join("persistent");
    let child = start_bind_worker(&dir, &port, "persistent");
    let err = child
        .wait()
        .expect_err("a second bind failure must remain a failure");
    assert_eq!(err.kind(), std::io::ErrorKind::AddrInUse);
    assert_eq!(attempts(&dir), 2, "no third attempt");

    let dir = root.join("live-listener-failure");
    let child = start_bind_worker(&dir, &port, "live-bind-error");
    let err = child
        .wait()
        .expect_err("a failed listener actor must not leave the harness waiting forever");
    assert_eq!(err.kind(), std::io::ErrorKind::AddrInUse);
    assert_eq!(
        attempts(&dir),
        2,
        "no third attempt even while the process stays alive"
    );

    let dir = root.join("genuine");
    let child = start_bind_worker(&dir, &port, "real-error");
    assert_eq!(
        child.wait().unwrap().code(),
        Some(42),
        "a genuine startup failure propagates"
    );
    assert_eq!(attempts(&dir), 1, "do not retry non-bind errors");
    drop(_blocker);

    let dir = root.join("late");
    let child = start_bind_worker(&dir, &port, "late-error");
    wait_until(|| dir.join("ready").exists());
    child.mark_started();
    std::fs::write(dir.join("stop"), []).unwrap();
    assert_eq!(
        child.wait().unwrap().code(),
        Some(41),
        "a running node's crash propagates"
    );
    assert_eq!(attempts(&dir), 1, "do not retry after startup");
    eprintln!("transient bind: 2 attempts; persistent bind: 2; genuine/late failure: 1 each");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn contiguous_blocks_and_clones_hold_every_reservation() {
    let ports = Port::reserve_block(10).unwrap();
    for pair in ports.windows(2) {
        assert_eq!(
            pair[1].port(),
            pair[0].port() + 1,
            "legacy fallback ports must be consecutive"
        );
    }
    for port in &ports {
        assert!(
            UdpSocket::bind(port.addr()).is_err(),
            "contiguous block dropped reservation {port}"
        );
        assert!(
            TcpListener::bind(port.addr()).is_ok(),
            "contiguous block must leave TCP available"
        );
    }
    let clone = ports[0].clone();
    let addr = clone.addr();
    drop(ports);
    assert!(
        UdpSocket::bind(addr).is_err(),
        "cloned lease was released too early"
    );
    drop(clone); // now available to other lanes; do not race them with a probe
}

#[test]
#[ignore = "subprocess fixture"]
fn group_parent_worker() {
    let dir = PathBuf::from(std::env::var_os("AETHER_PORT_WORKER").unwrap());
    std::fs::write(dir.join("parent.pid"), std::process::id().to_string()).unwrap();
    let mut command = worker("group_listener_worker", &dir);
    command.env(
        "AETHER_BIND_ADDR",
        std::env::var("AETHER_BIND_ADDR").unwrap(),
    );
    let mut descendant = command.spawn().unwrap();
    // Inherits the parent's process group, just like aether run's child.
    descendant.wait().unwrap();
}

#[test]
#[ignore = "subprocess fixture"]
fn group_listener_worker() {
    let dir = PathBuf::from(std::env::var_os("AETHER_PORT_WORKER").unwrap());
    let _listener = TcpListener::bind(std::env::var("AETHER_BIND_ADDR").unwrap()).unwrap();
    std::fs::write(dir.join("ready"), []).unwrap();
    wait_until(|| dir.join("stop").exists());
}

#[test]
fn cleanup_releases_supervisor_descendant_ports() {
    let root = temp("descendants");
    let port = Port::reserve().unwrap();
    let mut command = worker("group_parent_worker", &root);
    command.env("AETHER_BIND_ADDR", port.addr().to_string());
    let child = TestChild::spawn(command, root.join("node.log")).unwrap();
    wait_until(|| root.join("ready").exists());
    assert_eq!(
        child.id(),
        std::fs::read_to_string(root.join("parent.pid")).unwrap().parse::<u32>().unwrap(),
        "signals target the owned parent, rather than its listener descendant"
    );
    child.mark_started();
    child.kill().unwrap();
    child.wait().unwrap();
    let released = TcpListener::bind(port.addr());
    // Also let the old, ungrouped fixture terminate when the red check fails.
    std::fs::write(root.join("stop"), []).unwrap();
    assert!(
        released.is_ok(),
        "test descendant still holds TCP port after parent cleanup: {released:?}"
    );
    std::fs::remove_dir_all(root).unwrap();
}
