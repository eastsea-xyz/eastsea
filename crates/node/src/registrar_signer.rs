//! Where the registrar's P-256 attestation key lives (docs/ops/registrar.md).
//!
//! The registrar signs a registration attestation that
//! `CommitteeRegistry.register` verifies on chain, and a per-period
//! re-attestation that validators verify against the same key in the registry
//! (docs/design/14-registration.md), and the binding of an in-memory encryption
//! key to that same on-chain identity. The helper stays signing-only; it never
//! receives DeviceCheck tokens or exports a private key. This module is the
//! seam over the key:
//!
//! - [`FileSigner`]: the key is a seed file, `<data>/registrar.key` written by
//!   `aether registrar-key` — local devnets and rehearsals, exactly as before.
//! - [`EnclaveSigner`]: the key is in the Secure Enclave of a signing-only Mac.
//!   That Mac runs `aether-registrar-signer serve` (apps/registrar-signer) and
//!   answers over a Unix socket. The private key never leaves the chip, cannot
//!   be copied off it, and is not written to the node's data directory.
//!
//! The node picks one with `--registrar-signer <socket>`; without it the file
//! key signs.
//!
//! Protocol of the helper (one JSON object per line, one request per connection):
//!
//! ```text
//! → {"op":"public"}
//! ← {"ok":true,"public":"<x hex><y hex>","secure_enclave":true}
//! → {"op":"sign","msg":"<message hex>"}
//! ← {"ok":true,"r":"<32-byte hex>","s":"<32-byte hex>"}
//! ← {"ok":false,"error":"…"}        (either request; the node reports it)
//! ```
//!
//! The message is signed as ECDSA P-256 over SHA-256(message), low-s, raw r‖s —
//! the convention of P256VERIFY (EIP-7951) and `aether_crypto::P256Signer`. The
//! Secure Enclave does not normalize s (about half its signatures are high-s,
//! which both the node and the chain reject), so the helper flips s to n−s
//! before it answers.

use std::io::{BufRead, BufReader, Read as _, Write as _};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use aether_crypto::PublicKey;
use aether_types::SignerScheme;

/// A registrar attestation: raw r‖s over sha256(message).
pub type Signature = ([u8; 32], [u8; 32]);

/// How long one helper round trip may take before the node gives up.
const TIMEOUT: Duration = Duration::from_secs(30);
/// Cap on a helper's answer line.
const MAX_LINE: u64 = 1 << 16;

/// The registrar's attestation key. Implementations must be usable from the
/// RPC service on several tasks at once.
pub trait RegistrarSigner: Send + Sync {
    /// The registrar public key as x‖y hex (lowercase): what
    /// `aether network --registrar` takes and what registry slots 0 and 1 hold.
    fn public_hex(&self) -> String;
    /// Sign an attestation: ECDSA P-256 over SHA-256(msg), low-s, raw r‖s.
    fn sign_bytes(&self, msg: &[u8]) -> Result<Signature, String>;
    /// Where the key is, for logs.
    fn describe(&self) -> String;
}

/// The key is a seed file (`aether registrar-key`): devnets and rehearsals.
pub struct FileSigner(crate::faucet::Faucet);

impl FileSigner {
    /// A key from a 32-byte seed (tests and devnets).
    pub fn from_seed(seed: &[u8; 32]) -> Result<Self, String> {
        Ok(FileSigner(crate::faucet::Faucet::from_seed(seed)?))
    }

    /// The key file `aether registrar-key` writes (a 32-byte hex seed).
    pub fn load(path: &Path) -> Result<Self, String> {
        Ok(FileSigner(crate::faucet::Faucet::load(path)?))
    }

    /// Create a random key file at `path` (owner-only); returns its address.
    pub fn generate(path: &Path) -> Result<aether_types::Address, String> {
        crate::faucet::Faucet::generate(path)
    }
}

impl RegistrarSigner for FileSigner {
    fn public_hex(&self) -> String {
        self.0.public_hex()
    }

    fn sign_bytes(&self, msg: &[u8]) -> Result<Signature, String> {
        self.0.sign_bytes(msg)
    }

    fn describe(&self) -> String {
        format!("file key (account {})", self.0.address)
    }
}

/// The key is in the Secure Enclave on the signing Mac; `apps/registrar-signer`
/// signs over a Unix socket (see the module docs for the protocol).
#[derive(Debug)]
pub struct EnclaveSigner {
    socket: PathBuf,
    public: String,
    /// SEC1 uncompressed form of `public`: every signature the helper returns is
    /// checked against it before the node uses it.
    sec1: Vec<u8>,
}

impl EnclaveSigner {
    /// Connect and learn the key: fails if the helper is not running, or answers
    /// with something that is not a P-256 public key.
    pub fn connect(socket: &Path) -> Result<Self, String> {
        let reply = call(socket, &serde_json::json!({ "op": "public" }))?;
        let public = reply["public"]
            .as_str()
            .ok_or("the signer helper gave no public key")?
            .to_ascii_lowercase();
        let bytes = hex::decode(&public).map_err(|_| "the signer helper's public key is not hex")?;
        if bytes.len() != 64 {
            return Err(format!("the signer helper's public key is {} bytes, not 64 (x‖y)", bytes.len()));
        }
        let sec1 = [&[4u8][..], bytes.as_slice()].concat();
        // Refuse a key that is not a point on P-256 before it is ever used.
        aether_crypto::p256_xy(&sec1).map_err(|e| format!("the signer helper's public key is not a P-256 point: {e:?}"))?;
        Ok(EnclaveSigner { socket: socket.to_path_buf(), public, sec1 })
    }

    /// The helper's socket (for logs).
    pub fn socket(&self) -> &Path {
        &self.socket
    }

    fn call(&self, request: serde_json::Value) -> Result<serde_json::Value, String> {
        call(&self.socket, &request)
    }
}

impl RegistrarSigner for EnclaveSigner {
    fn public_hex(&self) -> String {
        self.public.clone()
    }

    fn sign_bytes(&self, msg: &[u8]) -> Result<Signature, String> {
        let reply = self.call(serde_json::json!({ "op": "sign", "msg": hex::encode(msg) }))?;
        let part = |k: &str| -> Result<[u8; 32], String> {
            hex::decode(reply[k].as_str().unwrap_or_default())
                .ok()
                .and_then(|b| b.try_into().ok())
                .ok_or(format!("the signer helper gave no {k}"))
        };
        let (r, s) = (part("r")?, part("s")?);
        // The node does not trust the helper blindly: a mixed-up or swapped
        // helper signing with another key produces nothing usable.
        let pk = PublicKey { scheme: SignerScheme::P256, bytes: self.sec1.clone() };
        aether_crypto::verify(&pk, msg, &[r.as_slice(), s.as_slice()].concat())
            .map_err(|e| format!("the signer helper's signature does not verify: {e:?}"))?;
        Ok((r, s))
    }

    fn describe(&self) -> String {
        format!("Secure Enclave via {} (key {}…)", self.socket.display(), &self.public[..16])
    }
}

/// One request/response over `socket`: a JSON line out, a JSON line back.
fn call(socket: &Path, request: &serde_json::Value) -> Result<serde_json::Value, String> {
    let stream = UnixStream::connect(socket).map_err(|e| {
        format!(
            "{}: {e} (start `aether-registrar-signer serve --socket {}` on the signing Mac)",
            socket.display(),
            socket.display()
        )
    })?;
    stream.set_read_timeout(Some(TIMEOUT)).map_err(|e| e.to_string())?;
    stream.set_write_timeout(Some(TIMEOUT)).map_err(|e| e.to_string())?;
    let mut line = serde_json::to_vec(request).map_err(|e| e.to_string())?;
    line.push(b'\n');
    let mut writer = stream.try_clone().map_err(|e| e.to_string())?;
    writer.write_all(&line).map_err(|e| format!("{}: {e}", socket.display()))?;
    writer.flush().map_err(|e| e.to_string())?;
    let mut answer = String::new();
    BufReader::new(stream)
        .take(MAX_LINE)
        .read_line(&mut answer)
        .map_err(|e| format!("{}: {e}", socket.display()))?;
    let reply: serde_json::Value = serde_json::from_str(answer.trim_end()).map_err(|e| format!("{}: the signer helper answered {answer:?} ({e})", socket.display()))?;
    if reply["ok"].as_bool() != Some(true) {
        return Err(format!("the signer helper refused: {}", reply["error"].as_str().unwrap_or("no reason given")));
    }
    Ok(reply)
}

#[cfg(test)]
mod tests {
    use super::*;
    use aether_crypto::Signer as _;
    use std::os::unix::net::UnixListener;

    fn temp_dir(_tag: &str) -> PathBuf {
        // macOS limits a Unix socket's entire pathname to 103 bytes. Keep
        // fixtures short enough for a worktree's mandatory ./tmp directory,
        // including macOS's longer /System/Volumes/Data path alias.
        // Claim a fresh name rather than deleting a prior process's fixture.
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        loop {
            let next = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!("sg-{:x}-{next:x}", std::process::id()));
            match std::fs::create_dir(&dir) {
                Ok(()) => return dir,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("cannot create signer fixture: {error}"),
            }
        }
    }

    /// A stand-in for `aether-registrar-signer`: answers every connection with
    /// `reply`, for the rest of the test process.
    fn mock_helper(dir: &Path, reply: impl Fn(&serde_json::Value) -> serde_json::Value + Send + 'static) -> PathBuf {
        let path = dir.join("s");
        let listener = UnixListener::bind(&path).unwrap();
        std::thread::spawn(move || {
            for mut stream in listener.incoming().flatten() {
                let mut line = String::new();
                if BufReader::new(stream.try_clone().expect("clone")).read_line(&mut line).is_err() {
                    break;
                }
                let request: serde_json::Value = serde_json::from_str(line.trim_end()).unwrap();
                let mut out = serde_json::to_vec(&reply(&request)).unwrap();
                out.push(b'\n');
                let _ = stream.write_all(&out);
            }
        });
        path
    }

    #[test]
    fn a_file_signer_is_the_same_key_as_the_p256_seed() {
        let key = aether_crypto::P256Signer::from_seed(&[4u8; 32]).unwrap();
        let (x, y) = aether_crypto::p256_xy(&key.public_key().bytes).unwrap();
        let signer = FileSigner::from_seed(&[4u8; 32]).unwrap();
        assert_eq!(signer.public_hex(), format!("{}{}", hex::encode(x), hex::encode(y)));
        let (r, s) = signer.sign_bytes(b"attestation").unwrap();
        aether_crypto::verify(&key.public_key(), b"attestation", &[r.as_slice(), s.as_slice()].concat()).unwrap();
        assert!(signer.describe().contains("file key"));
    }

    #[test]
    fn the_enclave_signer_uses_the_helpers_key_and_checks_its_signatures() {
        let dir = temp_dir("ok");
        let key = aether_crypto::P256Signer::from_seed(&[7u8; 32]).unwrap();
        let (x, y) = aether_crypto::p256_xy(&key.public_key().bytes).unwrap();
        let path = mock_helper(&dir, move |request| {
            if request["op"] == "public" {
                return serde_json::json!({ "ok": true, "public": format!("{}{}", hex::encode(x), hex::encode(y)), "secure_enclave": true });
            }
            let msg = hex::decode(request["msg"].as_str().expect("msg")).unwrap();
            let sig = key.sign(&msg).unwrap();
            serde_json::json!({ "ok": true, "r": hex::encode(&sig[..32]), "s": hex::encode(&sig[32..]) })
        });
        let signer = EnclaveSigner::connect(&path).unwrap();
        assert_eq!(signer.public_hex(), format!("{}{}", hex::encode(x), hex::encode(y)));
        let (r, s) = signer.sign_bytes(b"attestation").unwrap();
        let pk = PublicKey { scheme: SignerScheme::P256, bytes: [&[4u8][..], &hex::decode(signer.public_hex()).unwrap()].concat() };
        aether_crypto::verify(&pk, b"attestation", &[r.as_slice(), s.as_slice()].concat()).unwrap();
        assert!(signer.describe().contains("Secure Enclave"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_helper_that_signs_with_another_key_is_refused() {
        let dir = temp_dir("liar");
        let good = aether_crypto::P256Signer::from_seed(&[7u8; 32]).unwrap();
        let liar = aether_crypto::P256Signer::from_seed(&[8u8; 32]).unwrap();
        let (x, y) = aether_crypto::p256_xy(&good.public_key().bytes).unwrap();
        let path = mock_helper(&dir, move |request| {
            if request["op"] == "public" {
                return serde_json::json!({ "ok": true, "public": format!("{}{}", hex::encode(x), hex::encode(y)) });
            }
            let msg = hex::decode(request["msg"].as_str().expect("msg")).unwrap();
            let sig = liar.sign(&msg).unwrap();
            serde_json::json!({ "ok": true, "r": hex::encode(&sig[..32]), "s": hex::encode(&sig[32..]) })
        });
        let signer = EnclaveSigner::connect(&path).unwrap();
        let err = signer.sign_bytes(b"attestation").unwrap_err();
        assert!(err.contains("does not verify"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_helper_that_refuses_or_is_absent_is_an_error_not_a_panic() {
        let dir = temp_dir("refuse");
        let path = mock_helper(&dir, |_| serde_json::json!({ "ok": false, "error": "no key: run `aether-registrar-signer init`" }));
        assert!(EnclaveSigner::connect(&path).unwrap_err().contains("run `aether-registrar-signer init`"));
        let absent = dir.join("n");
        let err = EnclaveSigner::connect(&absent).unwrap_err();
        assert!(err.contains("aether-registrar-signer serve"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The real helper on a signing Mac, through the real trait: build it
    /// (`scripts/build-registrar-signer.sh`), create the key
    /// (`aether-registrar-signer init`, AETHER_REGISTRAR_HOME=<dir>) and run:
    ///
    /// ```text
    /// cargo test -p aether-node --lib registrar_signer -- --ignored
    /// ```
    ///
    /// Ignored by default: it needs the Secure Enclave of the signing Mac, and
    /// the key cannot exist anywhere else. This is the one check that the chip's
    /// own signatures (low-s normalized by the helper) pass `aether_crypto`, the
    /// verifier the chain uses.
    #[test]
    #[ignore = "needs the signing Mac: scripts/build-registrar-signer.sh and aether-registrar-signer init"]
    fn the_secure_enclave_helper_signs_attestations_the_node_accepts() {
        let home = std::env::var("AETHER_REGISTRAR_HOME").expect("AETHER_REGISTRAR_HOME with a key on this Mac");
        let bin = std::env::var("AETHER_REGISTRAR_SIGNER_BIN")
            .unwrap_or_else(|_| format!("{}/../../target/registrar-signer/aether-registrar-signer", env!("CARGO_MANIFEST_DIR")));
        let dir = temp_dir("live");
        let socket = dir.join("s");
        let mut helper = std::process::Command::new(&bin)
            .args(["serve", "--socket"])
            .arg(&socket)
            .env("AETHER_REGISTRAR_HOME", &home)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap_or_else(|e| panic!("{bin}: {e} (run scripts/build-registrar-signer.sh)"));
        // The helper binds its socket a moment after it starts.
        let signer = {
            let mut why = String::new();
            let mut connected = None;
            for _ in 0..50 {
                match EnclaveSigner::connect(&socket) {
                    Ok(s) => {
                        connected = Some(s);
                        break;
                    }
                    Err(e) => {
                        why = e;
                        std::thread::sleep(Duration::from_millis(100));
                    }
                }
            }
            match connected {
                Some(s) => s,
                None => {
                    let _ = helper.kill();
                    let _ = helper.wait();
                    panic!("the helper never served {socket:?}: {why} (run scripts/build-registrar-signer.sh)");
                }
            }
        };
        let msg = b"aether registrar attestation (live)";
        let (r, s) = signer.sign_bytes(msg).expect("the Secure Enclave signed");
        let pk = PublicKey { scheme: SignerScheme::P256, bytes: [&[4u8][..], &hex::decode(signer.public_hex()).unwrap()].concat() };
        aether_crypto::verify(&pk, msg, &[r.as_slice(), s.as_slice()].concat()).expect("the node accepts the helper's signature");
        // Half of the chip's raw signatures are high-s; the helper must never
        // hand the node one, so sign a few and check every s.
        for i in 0..16u8 {
            let (_, s) = signer.sign_bytes(&[i]).unwrap();
            assert!(low_s(&s), "the helper returned a high-s signature, which the chain rejects");
        }
        let _ = helper.kill();
        let _ = helper.wait();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Whether `s` is the low half of the P-256 group (the non-malleable form):
    /// n/2 = 7fffffff800000007fffffffffffffffde737d56d38bcf4279dce5617e3192a8.
    fn low_s(s: &[u8; 32]) -> bool {
        const HALF: [u8; 32] = [
            0x7f, 0xff, 0xff, 0xff, 0x80, 0x00, 0x00, 0x00, 0x7f, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xde, 0x73, 0x7d, 0x56, 0xd3, 0x8b, 0xcf, 0x42, 0x79, 0xdc, 0xe5, 0x61, 0x7e, 0x31, 0x92, 0xa8,
        ];
        s.as_slice() <= HALF.as_slice()
    }

    #[test]
    fn a_public_key_that_is_not_a_point_is_refused() {
        let dir = temp_dir("point");
        let path = mock_helper(&dir, |_| serde_json::json!({ "ok": true, "public": "00".repeat(64) }));
        assert!(EnclaveSigner::connect(&path).unwrap_err().contains("not a P-256 point"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
