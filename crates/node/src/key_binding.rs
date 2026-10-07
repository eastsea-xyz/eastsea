//! Hardware binding for this Mac's node keys (design 36, N1).

use std::path::Path;
use commonware_cryptography::Signer as _;

pub mod signing;

pub const BINDING_FILE: &str = "key-binding.json";
pub const EXIT_KEY_ELSEWHERE: i32 = 15;
const REFUSAL: &str = "key binding refused:";

#[derive(Debug)]
enum BindingError {
    Mismatch(String),
    Invalid(String),
    Unavailable(String),
    Storage(String),
}

impl std::fmt::Display for BindingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Mismatch(reason) => write!(f, "{REFUSAL} This node's keys came from another Mac. Voting and signing are stopped. {reason}"),
            Self::Invalid(reason) => write!(f, "key binding invalid: restore this node's original keys and binding. {reason}"),
            Self::Unavailable(reason) => write!(f, "waiting to confirm this Mac: signing is paused. {reason}"),
            Self::Storage(reason) => write!(f, "key binding storage error: {reason}"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Guard {
    dir: std::path::PathBuf,
    public: [u8; 32],
}

impl Guard {
    pub fn for_validator(dir: &Path, public: &crate::block::PublicKey) -> Self {
        Self {
            dir: dir.to_path_buf(),
            public: public.as_ref().try_into().expect("Ed25519 public key"),
        }
    }

    pub fn check_or_exit(&self) {
        if let Err(error) = wait_for_confirmation(|| platform_uuid()
            .and_then(|uuid| check_existing_with_uuid(&self.dir, &self.public, &uuid))) {
            eprintln!("error: {error}");
            std::process::exit(match error {
                BindingError::Mismatch(_) => EXIT_KEY_ELSEWHERE,
                BindingError::Storage(_) => crate::store::EXIT_STORAGE,
                _ => crate::candidate::EXIT_IDENTITY,
            });
        }
    }
}

static PROCESS_BINDING: std::sync::OnceLock<Guard> = std::sync::OnceLock::new();

/// The CLI has one validator identity. Keep raw DKG/transport handoffs under
/// the same guard without changing Commonware's key codecs.
pub fn install_process_guard(guard: Option<Guard>) {
    if let Some(guard) = guard {
        guard.check_or_exit();
        let _ = PROCESS_BINDING.set(guard);
    }
}

pub fn check_process() {
    if let Some(guard) = PROCESS_BINDING.get() {
        guard.check_or_exit();
    }
}

pub fn is_refusal(error: &str) -> bool {
    error.starts_with(REFUSAL)
}

pub fn exit_if_refusal(error: &str) {
    if is_refusal(error) {
        refuse(error);
    }
}

fn refuse(error: &str) -> ! {
    eprintln!("error: {error}");
    std::process::exit(EXIT_KEY_ELSEWHERE);
}

fn error(reason: impl std::fmt::Display) -> BindingError {
    BindingError::Invalid(reason.to_string())
}

fn unavailable(reason: impl std::fmt::Display) -> BindingError {
    BindingError::Unavailable(reason.to_string())
}

fn storage(reason: impl std::fmt::Display) -> BindingError {
    BindingError::Storage(reason.to_string())
}

fn wait_for_confirmation<T>(read: impl FnMut() -> Result<T, BindingError>) -> Result<T, BindingError> {
    retry_confirmation(read, std::thread::sleep)
}

fn retry_confirmation<T>(mut read: impl FnMut() -> Result<T, BindingError>, mut sleep: impl FnMut(std::time::Duration)) -> Result<T, BindingError> {
    let mut backoff = std::time::Duration::from_millis(250);
    let mut waited = false;
    loop {
        match read() {
            Err(e @ BindingError::Unavailable(_)) => {
                eprintln!("warning: {e}; retrying in {} ms", backoff.as_millis());
                waited = true;
                sleep(backoff);
                backoff = (backoff * 2).min(std::time::Duration::from_secs(30));
            }
            result => {
                if waited && result.is_ok() { eprintln!("Mac key binding confirmed; signing can resume"); }
                return result;
            }
        }
    }
}

pub fn check(dir: &Path, public: &crate::block::PublicKey) -> Result<Checked, String> {
    let public = public.as_ref().try_into().expect("Ed25519 public key");
    let checked = wait_for_confirmation(|| check_with_uuid(dir, &public, &platform_uuid()?))
        .map_err(|e| e.to_string())?;
    // Startup/legacy-load marker also clears a stale wait in an appended log.
    eprintln!("Mac key binding confirmed");
    Ok(checked)
}

/// Owner recovery is deliberately separate from verification: normal starts
/// never change a committed binding. CLI and wallet must present a terminal
/// and require the owner to type this validator's public address.
pub fn rebind_interactive(dir: &Path) -> Result<(), String> {
    use std::io::{IsTerminal as _, Write as _};
    use std::os::unix::fs::MetadataExt as _;
    let owner = unsafe { libc::geteuid() };
    for path in [dir.to_path_buf(), dir.join(crate::roster::KEY_FILE)] {
        let meta = std::fs::symlink_metadata(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        if meta.file_type().is_symlink() || meta.uid() != owner || (path != dir && !meta.is_file()) {
            return Err(format!("{} must belong to the interactive node owner", path.display()));
        }
        if path != dir && meta.mode() & 0o077 != 0 {
            return Err(format!("{} must be owner-only (chmod 600)", path.display()));
        }
    }
    // Hold both locks until publication and audit completion. The lock check
    // happens before prompting and also closes the race with a new run.
    let _running = crate::supervisor::lock_data_dir(dir)?;
    let _creation = crate::roster::lock_key_creation(dir)?;
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        return Err("keys rebind requires an interactive terminal; type the validator address to confirm".into());
    }
    if dir.join(crate::roster::CREATION_FILE).exists() {
        return Err("finish or restore the staged key creation before rebinding".into());
    }
    let keys = crate::roster::LocalKeys::load_unchecked(dir)?;
    let public: [u8; 32] = keys.signer.public_key().as_ref().try_into().expect("Ed25519 public key");
    let path = dir.join(BINDING_FILE);
    let before = read_binding(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let record: Record = serde_json::from_slice(&before).map_err(|e| format!("{}: {e}; restore the original binding", path.display()))?;
    if hex::decode(&record.validator_pub).ok().as_deref() != Some(public.as_slice())
        || hex::decode(&record.platform_uuid_hash).ok().is_none_or(|h| h.len() != 32)
        || record.created_at == 0 {
        return Err("the stored binding is invalid; restore the original key and binding before rebinding".into());
    }
    let uuid = platform_uuid().map_err(|e| e.to_string())?;
    let address = hex::encode(public);
    println!("Only do this if you moved this Mac's node on purpose; running the same keys on two Macs gets the validator slashed.");
    println!("Validator address: {address}");
    print!("Type the validator address to bind these keys to this Mac: ");
    std::io::stdout().flush().map_err(|e| e.to_string())?;
    let mut typed = String::new();
    std::io::stdin().read_line(&mut typed).map_err(|e| e.to_string())?;
    let typed = typed.trim();
    let typed = typed.strip_prefix("0x").or_else(|| typed.strip_prefix("0X")).unwrap_or(typed);
    if hex::decode(typed).ok().as_deref() != Some(public.as_slice()) {
        return Err("validator address confirmation did not match; binding unchanged".into());
    }
    // Read the ID again after the potentially long owner prompt. A failed
    // read cannot authorize a new binding, even after typed confirmation.
    let latest = platform_uuid().map_err(|e| e.to_string())?;
    if latest != uuid { return Err("this Mac's hardware ID changed during confirmation; binding unchanged".into()); }
    let new_hash = hex::encode(uuid_hash(&uuid));
    let audit = format!("validator={address} owner_uid={owner} old={} -> new={new_hash}", record.platform_uuid_hash);
    append_rebind_audit(dir, &format!("authorized {audit}\n"))?;
    crate::atomic::replace(&path, &record_bytes(&public, &uuid).map_err(|e| e.to_string())?, 0o600)?;
    append_rebind_audit(dir, &format!("committed {audit}\n"))
        .map_err(|e| format!("binding changed durably, but audit completion failed: {e}"))?;
    println!("Mac key binding confirmed; rebound {audit}");
    Ok(())
}

fn append_rebind_audit(dir: &Path, entry: &str) -> Result<(), String> {
    use std::io::Write as _;
    use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _};
    let path = dir.join("key-rebind.log");
    let mut log = std::fs::OpenOptions::new().append(true).create(true).mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let meta = log.metadata().map_err(|e| format!("{}: {e}", path.display()))?;
    if !meta.is_file() || meta.uid() != unsafe { libc::geteuid() } || meta.mode() & 0o077 != 0 {
        return Err(format!("{} must be a regular owner-only audit file", path.display()));
    }
    log.write_all(entry.as_bytes()).map_err(|e| format!("{}: {e}", path.display()))?;
    log.sync_all().map_err(|e| format!("{}: {e}", path.display()))?;
    crate::atomic::sync_parent(&path)
}

fn platform_uuid() -> Result<String, BindingError> {
    #[cfg(all(feature = "test-seam", debug_assertions))]
    if let Ok(uuid) = std::env::var("AETHER_TEST_PLATFORM_UUID") {
        return if uuid.is_empty() {
            Err(unavailable("empty test platform UUID"))
        } else {
            Ok(uuid)
        };
    }
    hardware_uuid().map_err(unavailable)
}

#[cfg(target_os = "macos")]
fn hardware_uuid() -> Result<String, String> {
    use std::ffi::{c_char, c_void, CStr};
    #[link(name = "IOKit", kind = "framework")]
    unsafe extern "C" {
        fn IOServiceMatching(name: *const c_char) -> *const c_void;
        fn IOServiceGetMatchingService(port: u32, matching: *const c_void) -> u32;
        fn IORegistryEntryCreateCFProperty(
            entry: u32,
            key: *const c_void,
            allocator: *const c_void,
            options: u32,
        ) -> *const c_void;
        fn IOObjectRelease(object: u32) -> i32;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFStringCreateWithCString(
            allocator: *const c_void,
            string: *const c_char,
            encoding: u32,
        ) -> *const c_void;
        fn CFStringGetCString(
            string: *const c_void,
            buffer: *mut c_char,
            size: isize,
            encoding: u32,
        ) -> u8;
        fn CFGetTypeID(value: *const c_void) -> usize;
        fn CFStringGetTypeID() -> usize;
        fn CFRelease(value: *const c_void);
    }
    const UTF8: u32 = 0x0800_0100;
    // Signatures/ownership follow the public IOKit/CoreFoundation SDK headers.
    // Matching consumes the dictionary; the service and CF objects are ours.
    unsafe {
        let matching = IOServiceMatching(c"IOPlatformExpertDevice".as_ptr());
        if matching.is_null() {
            return Err("IOPlatformExpertDevice matching unavailable".into());
        }
        let service = IOServiceGetMatchingService(0, matching);
        if service == 0 {
            return Err("IOPlatformExpertDevice unavailable".into());
        }
        let key = CFStringCreateWithCString(std::ptr::null(), c"IOPlatformUUID".as_ptr(), UTF8);
        if key.is_null() {
            IOObjectRelease(service);
            return Err("IOPlatformUUID property name unavailable".into());
        }
        let value = IORegistryEntryCreateCFProperty(service, key, std::ptr::null(), 0);
        CFRelease(key);
        IOObjectRelease(service);
        if value.is_null() {
            return Err("IOPlatformUUID unavailable".into());
        }
        let mut buffer = [0 as c_char; 128];
        let valid = CFGetTypeID(value) == CFStringGetTypeID()
            && CFStringGetCString(value, buffer.as_mut_ptr(), buffer.len() as isize, UTF8) != 0;
        CFRelease(value);
        if !valid {
            return Err("IOPlatformUUID is not a readable string".into());
        }
        let uuid = CStr::from_ptr(buffer.as_ptr())
            .to_str()
            .map_err(|_| "IOPlatformUUID is not UTF-8")?;
        if uuid.len() != 36
            || !uuid.bytes().enumerate().all(|(i, b)| {
                if [8, 13, 18, 23].contains(&i) {
                    b == b'-'
                } else {
                    b.is_ascii_hexdigit()
                }
            })
        {
            return Err("IOPlatformUUID is malformed".into());
        }
        Ok(uuid.to_ascii_uppercase())
    }
}

#[cfg(not(target_os = "macos"))]
fn hardware_uuid() -> Result<String, String> {
    Err("IOPlatformUUID requires macOS".into())
}

#[derive(Debug, PartialEq, Eq)]
pub enum Checked {
    Created,
    Existing,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    validator_pub: String,
    platform_uuid_hash: String,
    created_at: u64,
}

fn uuid_hash(uuid: &str) -> [u8; 32] {
    use sha2::{Digest as _, Sha256};
    let mut hash = Sha256::new();
    hash.update(b"eastsea.bind");
    hash.update(uuid.as_bytes());
    hash.finalize().into()
}

fn record_bytes(public: &[u8; 32], uuid: &str) -> Result<Vec<u8>, BindingError> {
    let record = Record {
        validator_pub: hex::encode(public),
        platform_uuid_hash: hex::encode(uuid_hash(uuid)),
        created_at: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
            .map_err(storage)?.as_secs(),
    };
    serde_json::to_vec_pretty(&record).map_err(storage)
}

pub(crate) fn prepare_creation(public: &crate::block::PublicKey) -> Result<Vec<u8>, String> {
    let public = public.as_ref().try_into().expect("Ed25519 public key");
    wait_for_confirmation(|| record_bytes(&public, &platform_uuid()?)).map_err(|e| e.to_string())
}

pub(crate) fn publish_prepared(dir: &Path, public: &crate::block::PublicKey, bytes: &[u8]) -> Result<(), String> {
    let public = public.as_ref().try_into().expect("Ed25519 public key");
    let path = dir.join(BINDING_FILE);
    wait_for_confirmation(|| {
        let uuid = platform_uuid()?;
        validate(&path, bytes, &public, &uuid)?;
        match crate::atomic::create_once(&path, bytes, 0o600) {
            Ok(()) => Ok(()),
            Err(crate::atomic::CreateError::AlreadyExists) => {
                check_existing_with_uuid(dir, &public, &uuid)?;
                crate::atomic::sync_parent(&path).map_err(storage)
            }
            Err(e) => Err(storage(e)),
        }
    }).map_err(|e| e.to_string())
}

fn read_binding(path: &Path) -> Result<Vec<u8>, std::io::Error> {
    use std::io::Read as _;
    use std::os::unix::fs::OpenOptionsExt as _;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)?;
    if !file.metadata()?.is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "not a regular binding file",
        ));
    }
    let mut bytes = Vec::new();
    file.take(4_097).read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn validate(path: &Path, bytes: &[u8], public: &[u8; 32], uuid: &str) -> Result<(), BindingError> {
    if bytes.len() > 4_096 {
        return Err(error(format!("{} is too large", path.display())));
    }
    let record: Record = serde_json::from_slice(bytes)
        .map_err(|e| error(format!("{} is corrupt: {e}", path.display())))?;
    let saved_public = hex::decode(&record.validator_pub)
        .map_err(|_| error(format!("{} has a corrupt public key", path.display())))?;
    let saved_hash = hex::decode(&record.platform_uuid_hash)
        .map_err(|_| error(format!("{} has a corrupt hardware hash", path.display())))?;
    if saved_public.len() != 32 || saved_hash.len() != 32 || record.created_at == 0 {
        return Err(error(format!(
            "{} has invalid binding fields",
            path.display()
        )));
    }
    if saved_public.as_slice() != public.as_slice() {
        return Err(error(format!(
            "{} does not match the validator key",
            path.display()
        )));
    }
    if saved_hash.as_slice() != uuid_hash(uuid).as_slice() {
        return Err(BindingError::Mismatch(format!("{} does not match this Mac", path.display())));
    }
    Ok(())
}

fn check_existing_with_uuid(dir: &Path, public: &[u8; 32], uuid: &str) -> Result<(), BindingError> {
    let path = dir.join(BINDING_FILE);
    let bytes = read_binding(&path).map_err(|e| {
        let reason = format!("{}: {e}", path.display());
        if matches!(e.kind(), std::io::ErrorKind::NotFound | std::io::ErrorKind::InvalidData) {
            error(reason)
        } else { unavailable(reason) }
    })?;
    validate(&path, &bytes, public, uuid)
}

fn check_with_uuid(dir: &Path, public: &[u8; 32], uuid: &str) -> Result<Checked, BindingError> {
    if uuid.is_empty() {
        return Err(unavailable("platform UUID is empty"));
    }
    let path = dir.join(BINDING_FILE);
    match read_binding(&path) {
        Ok(bytes) => {
            validate(&path, &bytes, public, uuid)?;
            crate::atomic::sync_parent(&path).map_err(storage)?;
            Ok(Checked::Existing)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let bytes = record_bytes(public, uuid)?;
            match crate::atomic::create_once(&path, &bytes, 0o600) {
                Ok(()) => Ok(Checked::Created),
                // Another creator may have won. Accept only its matching
                // binding, never overwrite it or ignore a mismatching winner.
                Err(crate::atomic::CreateError::AlreadyExists) => {
                    let saved = read_binding(&path).map_err(unavailable)?;
                    validate(&path, &saved, public, uuid)?;
                    crate::atomic::sync_parent(&path).map_err(storage)?;
                    Ok(Checked::Existing)
                }
                Err(write) => Err(storage(write)),
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::InvalidData => Err(error(format!("{}: {e}", path.display()))),
        Err(e) => Err(unavailable(format!("{}: {e}", path.display()))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    struct Dir(std::path::PathBuf);
    impl Dir {
        fn new(tag: &str) -> Self {
            let root = Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .parent()
                .unwrap();
            let path = root
                .join("tmp")
                .join(format!("aether-key-binding-{tag}-{}", std::process::id()));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn binding(dir: &Path, public: &[u8; 32], uuid: &str) {
        let hash = Sha256::digest([b"eastsea.bind".as_slice(), uuid.as_bytes()].concat());
        std::fs::write(
            dir.join(BINDING_FILE),
            serde_json::to_vec(&serde_json::json!({
                "validator_pub": hex::encode(public),
                "platform_uuid_hash": hex::encode(hash),
                "created_at": 1,
            }))
            .unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn binding_publication_does_not_hide_directory_sync_failure() {
        let dir = Dir::new("sync-failure");
        crate::atomic::fail_sync_for_test(Some(&dir.0));
        let result = check_with_uuid(&dir.0, &[1; 32], "MAC-A");
        crate::atomic::fail_sync_for_test(None);
        assert!(dir.0.join(BINDING_FILE).exists(), "the fault is after publication");
        assert!(result.is_err(), "a visible record must not mask failed durable publication");
    }

    #[test]
    fn an_existing_visible_binding_still_requires_durable_publication() {
        let dir = Dir::new("visible-sync-failure");
        binding(&dir.0, &[1; 32], "MAC-A");
        crate::atomic::fail_sync_for_test(Some(&dir.0));
        let result = check_with_uuid(&dir.0, &[1; 32], "MAC-A");
        crate::atomic::fail_sync_for_test(None);
        assert!(result.is_err(), "a reader racing publication must confirm directory durability");
    }

    #[test]
    fn unavailable_confirmation_retries_with_bounded_backoff_without_authorizing_signing() {
        let mut attempts = 0;
        let mut delays = Vec::new();
        let result = retry_confirmation(|| {
            attempts += 1;
            if attempts <= 12 { Err(unavailable("IOKit denied this read")) } else { Ok("confirmed") }
        }, |delay| delays.push(delay));
        assert_eq!(result.unwrap(), "confirmed");
        assert_eq!(attempts, 13);
        assert_eq!(delays[0], std::time::Duration::from_millis(250));
        assert!(delays.iter().all(|d| *d <= std::time::Duration::from_secs(30)));
        assert_eq!(delays.last(), Some(&std::time::Duration::from_secs(30)));
    }

    #[test]
    fn only_a_successfully_read_different_hardware_hash_is_exit_15() {
        let dir = Dir::new("typed-outcomes");
        binding(&dir.0, &[1; 32], "MAC-A");
        assert!(matches!(check_with_uuid(&dir.0, &[1; 32], ""), Err(BindingError::Unavailable(_))));
        assert!(matches!(check_with_uuid(&dir.0, &[2; 32], "MAC-A"), Err(BindingError::Invalid(_))));
        assert!(matches!(check_with_uuid(&dir.0, &[1; 32], "MAC-B"), Err(BindingError::Mismatch(_))));
    }

    #[test]
    fn mismatch_refuses() {
        let dir = Dir::new("mismatch");
        binding(&dir.0, &[1; 32], "MAC-A");
        assert!(
            check_with_uuid(&dir.0, &[1; 32], "MAC-B").is_err(),
            "a copied key must not sign on another Mac"
        );
    }

    #[test]
    fn missing_binding_is_created_once() {
        let dir = Dir::new("missing");
        assert_eq!(
            check_with_uuid(&dir.0, &[1; 32], "MAC-A").unwrap(),
            Checked::Created
        );
        let first = std::fs::read(dir.0.join(BINDING_FILE)).unwrap();
        assert_eq!(
            check_with_uuid(&dir.0, &[1; 32], "MAC-A").unwrap(),
            Checked::Existing
        );
        assert_eq!(
            std::fs::read(dir.0.join(BINDING_FILE)).unwrap(),
            first,
            "an existing binding is never replaced"
        );
    }

    #[test]
    fn corrupt_binding_refuses() {
        let dir = Dir::new("corrupt");
        for bytes in [
            b"{truncated".as_slice(),
            b"{}",
            b"null",
            br#"{"validator_pub":"00","platform_uuid_hash":"00","created_at":1}"#,
        ] {
            std::fs::write(dir.0.join(BINDING_FILE), bytes).unwrap();
            assert!(
                check_with_uuid(&dir.0, &[1; 32], "MAC-A").is_err(),
                "corruption must refuse, not silently rebind"
            );
            assert_eq!(std::fs::read(dir.0.join(BINDING_FILE)).unwrap(), bytes);
        }
    }

    #[test]
    fn another_public_key_refuses() {
        let dir = Dir::new("public");
        binding(&dir.0, &[1; 32], "MAC-A");
        assert!(check_with_uuid(&dir.0, &[2; 32], "MAC-A").is_err());
    }

    #[test]
    fn a_fifo_binding_refuses_without_blocking() {
        use std::os::unix::ffi::OsStrExt as _;
        let dir = Dir::new("fifo");
        let path = std::ffi::CString::new(dir.0.join(BINDING_FILE).as_os_str().as_bytes()).unwrap();
        // SAFETY: a NUL-terminated path inside this test's temporary directory.
        assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
        let data = dir.0.clone();
        let (send, receive) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = send.send(check_with_uuid(&data, &[1; 32], "MAC-A"));
        });
        assert!(receive
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("a malformed binding path must refuse rather than hang")
            .is_err());
    }
}
