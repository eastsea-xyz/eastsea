//! Shared support for TCP integration tests. No production crate depends on this.

use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::net::{SocketAddr, TcpListener, UdpSocket};
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const FIRST_PORT: u16 = 40_000;
const PORT_COUNT: usize = 49_152 - FIRST_PORT as usize;
static NEXT_PORT: AtomicUsize = AtomicUsize::new(0);

/// A TCP port reserved for this test until the last clone is dropped.
///
/// The kernel owns the reservation: an exclusive UDP socket on the same
/// loopback port coordinates every process, even in worktrees with different
/// TMPDIRs. TCP stays available for a node that must open its own listener.
/// These ports are below macOS's default ephemeral range, so clients do not
/// normally consume them between the availability probe and node startup.
/// This guard is for TCP servers; it must not be used for UDP/iroh listeners.
#[derive(Clone, Debug)]
pub struct Port {
    port: u16,
    _reservation: Arc<UdpSocket>,
}

impl Port {
    fn reserve_at(port: u16) -> io::Result<Self> {
        let reservation = UdpSocket::bind(("127.0.0.1", port))?;
        // Keep the UDP reservation while probing TCP. A cooperating test
        // cannot take this number in the gap before its node binds.
        drop(TcpListener::bind(("127.0.0.1", port))?);
        Ok(Self {
            port,
            _reservation: Arc::new(reservation),
        })
    }

    pub fn reserve() -> io::Result<Self> {
        let start = NEXT_PORT
            .fetch_add(1, Ordering::Relaxed)
            .wrapping_add(std::process::id() as usize)
            % PORT_COUNT;
        for offset in 0..PORT_COUNT {
            let port = FIRST_PORT + ((start + offset) % PORT_COUNT) as u16;
            match Self::reserve_at(port) {
                Ok(lease) => {
                    NEXT_PORT.fetch_add(offset, Ordering::Relaxed);
                    return Ok(lease);
                }
                Err(e) if e.kind() == io::ErrorKind::AddrInUse => continue,
                Err(e) => return Err(e),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AddrInUse,
            "all aether TCP test ports are reserved or in use",
        ))
    }

    /// Reserve consecutive numbers for a server with a built-in port fallback.
    pub fn reserve_block(count: usize) -> io::Result<Vec<Self>> {
        if count == 0 || count > PORT_COUNT {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid test port block size",
            ));
        }
        let start = NEXT_PORT
            .fetch_add(count, Ordering::Relaxed)
            .wrapping_add(std::process::id() as usize)
            % PORT_COUNT;
        for offset in 0..PORT_COUNT {
            let index = (start + offset) % PORT_COUNT;
            if index + count > PORT_COUNT {
                continue;
            }
            let mut ports = Vec::with_capacity(count);
            for i in 0..count {
                match Self::reserve_at(FIRST_PORT + (index + i) as u16) {
                    Ok(port) => ports.push(port),
                    Err(e) if e.kind() == io::ErrorKind::AddrInUse => break,
                    Err(e) => return Err(e),
                }
            }
            if ports.len() == count {
                NEXT_PORT.fetch_add(offset, Ordering::Relaxed);
                return Ok(ports);
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AddrInUse,
            "no contiguous aether test port block is available",
        ))
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn addr(&self) -> SocketAddr {
        SocketAddr::from(([127, 0, 0, 1], self.port))
    }

    /// Bind a mock TCP server, retrying an external bind race once.
    pub fn bind_tcp(&self) -> io::Result<TcpListener> {
        match TcpListener::bind(self.addr()) {
            Err(e) if e.kind() == io::ErrorKind::AddrInUse => {
                eprintln!(
                    "PORT RACE: {}: {e}; retrying TCP bind once (1/1)",
                    self.addr()
                );
                std::thread::sleep(Duration::from_millis(100));
                TcpListener::bind(self.addr())
            }
            result => result,
        }
    }
}

impl fmt::Display for Port {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.port, f)
    }
}

/// A test child with captured diagnostics and one bind-only startup retry.
/// Call `try_wait` while checking readiness, then `mark_started` as soon as
/// the service works. After that, crashes are returned without retrying.
pub struct TestChild {
    state: Mutex<ChildState>,
}

struct ChildState {
    command: Command,
    child: Child,
    log: PathBuf,
    offset: u64,
    retried: bool,
    started: bool,
    stopped: bool,
}

fn spawn_logged(command: &mut Command, log: &Path) -> io::Result<(Child, u64)> {
    if let Some(parent) = log.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file = OpenOptions::new().create(true).append(true).open(log)?;
    let offset = file.metadata()?.len();
    // aether run starts descendants. Give each harness child its own group
    // so a retry/cleanup cannot leave an old listener holding the leased TCP
    // port while the next attempt starts.
    #[cfg(unix)]
    command.process_group(0);
    let child = command
        .stdout(Stdio::from(file.try_clone()?))
        .stderr(Stdio::from(file))
        .spawn()?;
    Ok((child, offset))
}

fn stop_child(state: &mut ChildState) -> io::Result<()> {
    if state.stopped {
        return Ok(());
    }
    let child = &mut state.child;
    #[cfg(unix)]
    {
        let group = format!("-{}", child.id());
        // Use the system kill utility, keeping this dev crate dependency-free.
        // The negative ID targets only the group created by spawn_logged.
        let kill_group = |signal: &str| -> io::Result<bool> {
            Ok(Command::new("/bin/kill")
                .args([signal, &group])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()?
                .success())
        };
        kill_group("-KILL")?;
        child.wait()?;
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while kill_group("-0")? {
            if std::time::Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!("test process group {group} did not exit"),
                ));
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        state.stopped = true;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        if child.try_wait()?.is_none() {
            child.kill()?;
        }
        child.wait()?;
        state.stopped = true;
        Ok(())
    }
}

fn bind_failure(log: &str) -> bool {
    log.lines().any(|line| {
        // Commonware turns the OS error into BindFailed before its listener
        // panics. Match that exact startup panic, not arbitrary panic text.
        if line.contains("failed to bind listener: BindFailed")
            || line.contains("네트워크 포트 바인딩에 실패했습니다")
        {
            return true;
        }
        let line = line.to_ascii_lowercase();
        (line.contains("bind") || line.contains("rpc server stopped"))
            && (line.contains("address already in use") || line.contains("addrinuse"))
    })
}

impl TestChild {
    pub fn spawn(mut command: Command, log: impl AsRef<Path>) -> io::Result<Self> {
        let log = log.as_ref().to_path_buf();
        let (child, offset) = spawn_logged(&mut command, &log)?;
        Ok(Self {
            state: Mutex::new(ChildState {
                command,
                child,
                log,
                offset,
                retried: false,
                started: false,
                stopped: false,
            }),
        })
    }

    pub fn mark_started(&self) {
        self.state.lock().expect("test child lock").started = true;
    }

    /// The current owned process; a startup retry may replace it before readiness.
    pub fn id(&self) -> u32 {
        self.state.lock().expect("test child lock").child.id()
    }

    pub fn try_wait(&self) -> io::Result<Option<ExitStatus>> {
        let mut state = self.state.lock().expect("test child lock");
        let status = state.child.try_wait()?;
        if !state.started {
            let mut log = String::new();
            let mut file = File::open(&state.log)?;
            file.seek(SeekFrom::Start(state.offset))?;
            file.read_to_string(&mut log)?;
            if bind_failure(&log) {
                // A listener actor may panic before the whole node exits.
                // Reap it before retrying the same leased addresses: peers
                // have already been configured with these port numbers.
                stop_child(&mut state)?;
                if state.retried {
                    return Err(io::Error::new(
                        io::ErrorKind::AddrInUse,
                        format!("second startup bind failure; no more retries:\n{log}"),
                    ));
                }
                let message = format!(
                    "PORT RACE: {:?}: startup bind failed; retrying once (1/1)\n{log}",
                    state.command
                );
                eprintln!("{message}");
                writeln!(
                    OpenOptions::new().append(true).open(&state.log)?,
                    "{message}"
                )?;
                state.retried = true;
                std::thread::sleep(Duration::from_millis(100));
                let path = state.log.clone();
                let (child, offset) = spawn_logged(&mut state.command, &path)?;
                state.child = child;
                state.stopped = false;
                state.offset = offset;
                return Ok(None);
            }
        }
        Ok(status)
    }

    pub fn kill(&self) -> io::Result<()> {
        let mut state = self.state.lock().expect("test child lock");
        state.started = true; // cleanup must never restart the child
        stop_child(&mut state)
    }

    pub fn wait(&self) -> io::Result<ExitStatus> {
        loop {
            if let Some(status) = self.try_wait()? {
                return Ok(status);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    pub fn wait_with_output(self) -> io::Result<Output> {
        let status = self.wait()?;
        // Older executions and failed attempts stay in the diagnostic log,
        // but their apparent success output must not win a caller's parsing.
        let state = self.state.lock().expect("test child lock");
        let mut file = File::open(&state.log)?;
        file.seek(SeekFrom::Start(state.offset))?;
        let mut log = Vec::new();
        file.read_to_end(&mut log)?;
        Ok(Output {
            status,
            stdout: log.clone(),
            stderr: log,
        })
    }
}

impl Drop for TestChild {
    fn drop(&mut self) {
        if let Ok(state) = self.state.get_mut() {
            if let Err(error) = stop_child(state) {
                eprintln!(
                    "test child cleanup failed ({}): {error}",
                    state.log.display()
                );
            }
        }
    }
}
