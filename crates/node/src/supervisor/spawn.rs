//! Writer spawning uses native file actions; all setup stays in the parent.

use super::{WRITER_LEASE_ENV, WRITER_LEASE_MIN_FD};
use std::collections::BTreeMap;
use std::ffi::{CString, OsStr, OsString};
use std::os::unix::{ffi::OsStrExt, fs::MetadataExt, io::AsRawFd, process::ExitStatusExt};
use std::path::{Path, PathBuf};
use std::process::ExitStatus;

/// Writers configure only arguments and a scoped environment. There is no
/// Rust child-setup fallback, or hidden Command option for spawn to ignore.
pub(super) struct WriterCommand {
    program: PathBuf,
    args: Vec<OsString>,
    pub(super) env: BTreeMap<OsString, OsString>,
}

impl WriterCommand {
    pub(super) fn new(program: &Path) -> Self {
        Self {
            program: program.to_owned(),
            args: Vec::new(),
            env: std::env::vars_os().collect(),
        }
    }

    pub(super) fn arg(&mut self, arg: impl AsRef<OsStr>) -> &mut Self {
        self.args.push(arg.as_ref().to_owned());
        self
    }

    pub(super) fn args(&mut self, args: impl IntoIterator<Item = impl AsRef<OsStr>>) -> &mut Self {
        for arg in args {
            self.arg(arg);
        }
        self
    }
}

/// Cached wait semantics match Child: never signal a PID after reaping it.
pub(super) struct WriterChild {
    pid: libc::pid_t,
    status: Option<ExitStatus>,
}

impl WriterChild {
    pub(super) fn id(&self) -> u32 {
        self.pid as u32
    }

    pub(super) fn kill(&mut self) -> std::io::Result<()> {
        if self.status.is_some() {
            return Ok(());
        }
        if unsafe { libc::kill(self.pid, libc::SIGKILL) } == 0 {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error())
        }
    }

    fn wait_inner(&mut self, options: i32) -> std::io::Result<Option<ExitStatus>> {
        if let Some(status) = self.status {
            return Ok(Some(status));
        }
        loop {
            let mut raw = 0;
            let result = unsafe { libc::waitpid(self.pid, &mut raw, options) };
            if result == self.pid {
                let status = ExitStatus::from_raw(raw);
                self.status = Some(status);
                return Ok(Some(status));
            }
            if result == 0 {
                return Ok(None);
            }
            let error = std::io::Error::last_os_error();
            if error.kind() != std::io::ErrorKind::Interrupted {
                return Err(error);
            }
        }
    }

    pub(super) fn try_wait(&mut self) -> std::io::Result<Option<ExitStatus>> {
        self.wait_inner(libc::WNOHANG)
    }
    pub(super) fn wait(&mut self) -> std::io::Result<ExitStatus> {
        Ok(self
            .wait_inner(0)?
            .expect("blocking wait returns a child status"))
    }
}

fn spawn_error(code: i32) -> Result<(), String> {
    if code == 0 {
        Ok(())
    } else {
        Err(format!(
            "spawn writer: {}",
            std::io::Error::from_raw_os_error(code)
        ))
    }
}

struct SpawnActions(libc::posix_spawn_file_actions_t);
impl Drop for SpawnActions {
    fn drop(&mut self) {
        unsafe {
            libc::posix_spawn_file_actions_destroy(&mut self.0);
        }
    }
}
struct SpawnAttributes(libc::posix_spawnattr_t);
impl Drop for SpawnAttributes {
    fn drop(&mut self) {
        unsafe {
            libc::posix_spawnattr_destroy(&mut self.0);
        }
    }
}

// libc omits this Darwin extension; the SDK signature is stable since 10.7.
#[cfg(target_os = "macos")]
extern "C" {
    fn posix_spawn_file_actions_addinherit_np(
        actions: *mut libc::posix_spawn_file_actions_t,
        fd: libc::c_int,
    ) -> libc::c_int;
}

/// The borrowed lock remains live until spawn duplicates its open-file
/// description into the child's fixed slot. The parent never clears CLOEXEC.
pub(super) fn spawn_with_writer_lease(
    mut command: WriterCommand,
    lock: &std::fs::File,
) -> Result<WriterChild, String> {
    let metadata = lock
        .metadata()
        .map_err(|e| format!("writer lease metadata: {e}"))?;
    if !metadata.is_file() {
        return Err("writer lease must be a regular file".into());
    }
    let source_fd = lock.as_raw_fd();
    command.env.insert(
        WRITER_LEASE_ENV.into(),
        format!(
            "1:{WRITER_LEASE_MIN_FD}:{}:{}:{}",
            metadata.dev(),
            metadata.ino(),
            std::process::id()
        )
        .into(),
    );

    let string = |bytes: &[u8]| {
        CString::new(bytes)
            .map_err(|_| "spawn writer: argument or environment contains NUL".to_string())
    };
    let program = string(command.program.as_os_str().as_bytes())?;
    let mut args = vec![program.clone()];
    for arg in &command.args {
        args.push(string(arg.as_bytes())?);
    }
    let env = command
        .env
        .iter()
        .map(|(key, value)| {
            let mut bytes = key.as_bytes().to_vec();
            bytes.push(b'=');
            bytes.extend_from_slice(value.as_bytes());
            string(&bytes)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let pointers = |strings: &[CString]| {
        strings
            .iter()
            .map(|s| s.as_ptr().cast_mut())
            .chain(std::iter::once(std::ptr::null_mut()))
            .collect::<Vec<_>>()
    };
    let argv = pointers(&args);
    let envp = pointers(&env);

    let mut actions = std::mem::MaybeUninit::uninit();
    spawn_error(unsafe { libc::posix_spawn_file_actions_init(actions.as_mut_ptr()) })?;
    let mut actions = SpawnActions(unsafe { actions.assume_init() });
    let mut attributes = std::mem::MaybeUninit::uninit();
    spawn_error(unsafe { libc::posix_spawnattr_init(attributes.as_mut_ptr()) })?;
    let mut attributes = SpawnAttributes(unsafe { attributes.assume_init() });
    // Match Rust's normal child SIGPIPE disposition (the node ignores it).
    let mut defaults = unsafe { std::mem::zeroed() };
    unsafe {
        libc::sigemptyset(&mut defaults);
        libc::sigaddset(&mut defaults, libc::SIGPIPE);
    }
    spawn_error(unsafe { libc::posix_spawnattr_setsigdefault(&mut attributes.0, &defaults) })?;
    let flags = libc::POSIX_SPAWN_SETSIGDEF;
    #[cfg(target_os = "macos")]
    let flags = flags | libc::POSIX_SPAWN_CLOEXEC_DEFAULT;
    spawn_error(unsafe {
        libc::posix_spawnattr_setflags(&mut attributes.0, flags as libc::c_short)
    })?;
    #[cfg(target_os = "macos")]
    {
        // All other descriptors are closed by default.
        for fd in [libc::STDIN_FILENO, libc::STDOUT_FILENO, libc::STDERR_FILENO] {
            if fd != source_fd && unsafe { libc::fcntl(fd, libc::F_GETFD) } >= 0 {
                spawn_error(unsafe { posix_spawn_file_actions_addinherit_np(&mut actions.0, fd) })?;
            }
        }
    }
    // The dup action changes only the child's descriptor table. Explicit
    // inheritance also clears CLOEXEC when source == target (dup is a no-op).
    spawn_error(unsafe {
        libc::posix_spawn_file_actions_adddup2(&mut actions.0, source_fd, WRITER_LEASE_MIN_FD)
    })?;
    #[cfg(target_os = "macos")]
    spawn_error(unsafe {
        posix_spawn_file_actions_addinherit_np(&mut actions.0, WRITER_LEASE_MIN_FD)
    })?;
    if source_fd != WRITER_LEASE_MIN_FD {
        spawn_error(unsafe { libc::posix_spawn_file_actions_addclose(&mut actions.0, source_fd) })?;
    }
    let mut pid = 0;
    spawn_error(unsafe {
        libc::posix_spawnp(
            &mut pid,
            program.as_ptr(),
            &actions.0,
            &attributes.0,
            argv.as_ptr(),
            envp.as_ptr(),
        )
    })?;
    Ok(WriterChild { pid, status: None })
}
