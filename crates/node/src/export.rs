//! Era export to a static, torrent-ready file set (roadmap B6).
//!
//! Every completed era (8192 blocks, root committed in the certified history
//! MMR) a node seals is also written, verbatim, to an export directory as a
//! set a dumb mirror can serve:
//!
//! ```text
//! era-00000000.aera      the era file itself (byte for byte what was sealed)
//! era-00000000.json      its Manifest (aether_types): blake3, size, range,
//!                        Https and Torrent mirrors, Ed25519 signature
//! era-00000000.torrent   BitTorrent v1 metainfo: piece hashes, webseeds
//! index.json             the whole set: chain id, signer, per-era roots
//! ```
//!
//! Nothing here is trusted by readers: an era file is re-hashed block by block
//! and checked against a certified history root (`era_net`), the torrent's
//! pieces hash the same bytes, and the manifest's blake3 pins the file. The
//! Ed25519 signature (over the manifest with `signature` emptied, under
//! [`SIGN_NAMESPACE`]) only names the exporter; the history root is the proof.
//!
//! The magnet link identifies the file by its info hash (SHA-1 of the bencoded
//! `info` dict — the BitTorrent v1 rule), so a torrent client anywhere fetches
//! the same bytes over the DHT (peer discovery the network already uses) or
//! the webseeds. Seeding is out of scope here: the files are correct and
//! verifiable, serving them is the mirror's job (`rpc::serve` also exposes
//! plain `GET /era/<file>` so this node is its own first webseed).

use crate::chain::Chain;
use aether_state::mmr::ERA_LEN;
use aether_types::{Manifest, ManifestKind, Mirror};
use commonware_codec::{DecodeExt, Encode as _};
use commonware_cryptography::{ed25519, Signer as _};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// How often the exporter looks for newly sealed eras.
pub const INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);
/// Torrent piece length: an era file is a few MB, so 256 KiB pieces hash it
/// in well under a hundred pieces.
pub const PIECE_LEN: usize = 1 << 18;
/// Namespace of the manifest Ed25519 signature.
pub const SIGN_NAMESPACE: &[u8] = b"aether-era-manifest-v1";
/// Manifest format version.
const MANIFEST_VERSION: u32 = 1;

#[derive(Clone, Debug)]
pub struct ExportArgs {
    /// Directory the file set is written to (a mirror serves it as-is).
    pub dir: PathBuf,
    /// Extra webseed URLs (a `~name~` placeholder takes the file name; a bare
    /// URL gets the file appended as `<url>/<name>`).
    pub webseeds: Vec<String>,
    /// This node's public base URL (`http://host:port`); the era files it
    /// serves at `/era/<file>` become the first Https mirror and webseed.
    pub https_base: Option<String>,
    /// Ed25519 seed file (hex) signing every manifest; created when missing.
    pub sign_key: PathBuf,
}

/// A bencode value (BEP-003), encoding only what a torrent needs.
#[derive(Clone, Debug, PartialEq)]
pub enum Ben {
    Int(i64),
    Bytes(Vec<u8>),
    List(Vec<Ben>),
    Dict(Vec<(Vec<u8>, Ben)>),
}

/// Encode `b` as bencode. Dictionary keys are sorted (the rule every client
/// checks the info hash under).
pub fn ben_encode(b: Ben) -> Vec<u8> {
    let mut out = Vec::new();
    encode_into(&mut out, b);
    out
}

fn encode_into(out: &mut Vec<u8>, b: Ben) {
    match b {
        Ben::Int(i) => {
            out.extend_from_slice(format!("i{i}e").as_bytes());
        }
        Ben::Bytes(s) => {
            out.extend_from_slice(format!("{}:", s.len()).as_bytes());
            out.extend_from_slice(&s);
        }
        Ben::List(l) => {
            out.push(b'l');
            for e in l {
                encode_into(out, e);
            }
            out.push(b'e');
        }
        Ben::Dict(mut d) => {
            d.sort_by(|a, b| a.0.cmp(&b.0));
            out.push(b'd');
            for (k, v) in d {
                encode_into(out, Ben::Bytes(k));
                encode_into(out, v);
            }
            out.push(b'e');
        }
    }
}

/// Decode one bencode value from the front of `b` (tests and consumers check
/// what was written).
pub fn ben_decode(b: &[u8]) -> Result<(Ben, usize), String> {
    let (v, rest) = ben_decode_at(b)?;
    if !rest.is_empty() {
        return Err("trailing bytes after the bencode value".into());
    }
    let n = b.len() - rest.len();
    Ok((v, n))
}

fn ben_decode_at(b: &[u8]) -> Result<(Ben, &[u8]), String> {
    let (t, rest) = b.split_first().ok_or("empty")?;
    match t {
        b'i' => {
            let end = rest.iter().position(|c| *c == b'e').ok_or("unterminated integer")?;
            let i: i64 = std::str::from_utf8(&rest[..end]).map_err(|_| "integer")?.parse().map_err(|_| "integer")?;
            Ok((Ben::Int(i), &rest[end + 1..]))
        }
        b'l' => {
            let (mut rest, mut items) = (rest, Vec::new());
            while rest.first() != Some(&b'e') {
                let (v, r) = ben_decode_at(rest)?;
                items.push(v);
                rest = r;
            }
            Ok((Ben::List(items), &rest[1..]))
        }
        b'd' => {
            let (mut rest, mut items) = (rest, Vec::new());
            while rest.first() != Some(&b'e') {
                let (k, r) = ben_decode_at(rest)?;
                let Ben::Bytes(k) = k else { return Err("dictionary key is not a string".into()) };
                let (v, r) = ben_decode_at(r)?;
                items.push((k, v));
                rest = r;
            }
            Ok((Ben::Dict(items), &rest[1..]))
        }
        b'0'..=b'9' => {
            let colon = b.iter().position(|c| *c == b':').ok_or("string without a colon")?;
            let len: usize = std::str::from_utf8(&b[..colon]).map_err(|_| "length")?.parse().map_err(|_| "length")?;
            let rest = &b[colon + 1..];
            if rest.len() < len {
                return Err("string shorter than its length".into());
            }
            Ok((Ben::Bytes(rest[..len].to_vec()), &rest[len..]))
        }
        _ => Err("not bencode".into()),
    }
}

fn sha1(b: &[u8]) -> [u8; 20] {
    use ::sha1::{Digest as _, Sha1};
    let mut h = Sha1::new();
    h.update(b);
    h.finalize()[..].try_into().expect("sha1 is 20 bytes")
}

/// Build the BitTorrent v1 metainfo of `file` named `name`: SHA-1 piece
/// hashes, no tracker (peers come from the Mainline DHT, as the node's own
/// discovery already does) and `url-list` webseeds. Returns the metainfo
/// bytes and the info hash (SHA-1 of the bencoded `info` dict).
pub fn torrent(name: &str, file: &[u8], webseeds: &[String], piece_len: usize) -> (Vec<u8>, [u8; 20]) {
    let piece_len = piece_len.max(16 * 1024);
    let mut pieces = Vec::new();
    for chunk in file.chunks(piece_len) {
        pieces.extend_from_slice(&sha1(chunk));
    }
    let info = Ben::Dict(vec![
        (b"length".to_vec(), Ben::Int(file.len() as i64)),
        (b"name".to_vec(), Ben::Bytes(name.as_bytes().to_vec())),
        (b"piece length".to_vec(), Ben::Int(piece_len as i64)),
        (b"pieces".to_vec(), Ben::Bytes(pieces)),
    ]);
    let info_bytes = ben_encode(info.clone());
    let info_hash = sha1(&info_bytes);
    let mut top = vec![(b"info".to_vec(), info)];
    if !webseeds.is_empty() {
        top.push((b"url-list".to_vec(), Ben::List(webseeds.iter().map(|w| Ben::Bytes(w.as_bytes().to_vec())).collect())));
    }
    (ben_encode(Ben::Dict(top)), info_hash)
}

/// Percent-encode for a magnet's `dn` and `ws` parameters (RFC 3986
/// unreserved characters pass through).
fn urlencode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// The magnet link of a torrent: its info hash, display name and webseeds.
pub fn magnet(info_hash: &[u8; 20], name: &str, webseeds: &[String]) -> String {
    let mut m = format!("magnet:?xt=urn:btih:{}&dn={}", hex::encode(info_hash), urlencode(name));
    for w in webseeds {
        m.push_str("&ws=");
        m.push_str(&urlencode(w));
    }
    m
}

/// The manifest bytes a signature covers: the manifest itself with its
/// `signature` emptied, as the type documents.
fn manifest_bytes(m: &Manifest) -> Vec<u8> {
    let mut unsigned = m.clone();
    unsigned.signature = String::new();
    serde_json::to_vec(&unsigned).expect("a manifest serializes")
}

/// The Ed25519 key for a 32-byte seed. `PrivateKey::from_seed` takes a u64
/// (a test convenience); byte-built keys come through the codec's `Read`.
fn key_from_seed(seed: &[u8; 32]) -> ed25519::PrivateKey {
    use commonware_codec::ReadExt as _;
    ed25519::PrivateKey::read(&mut seed.as_slice()).expect("a 32-byte seed is a valid ed25519 key")
}

/// Sign `m` in place with the exporter's key.
fn sign_manifest(m: &mut Manifest, seed: &[u8; 32]) {
    let key = key_from_seed(seed);
    let sig = key.sign(SIGN_NAMESPACE, &manifest_bytes(m));
    m.signature = hex::encode(sig.encode());
}

/// Check `m`'s signature against the public key `signer` (hex). An empty
/// signature fails: a missing signature must look missing, not verified.
pub fn verify_manifest(m: &Manifest, signer: &str) -> bool {
    let key = hex::decode(signer).ok().and_then(|k| ed25519::PublicKey::decode(k.as_slice()).ok());
    let sig = hex::decode(&m.signature).ok().and_then(|s| ed25519::Signature::decode(s.as_slice()).ok());
    match (key, sig) {
        (Some(key), Some(sig)) => commonware_cryptography::Verifier::verify(&key, SIGN_NAMESPACE, &manifest_bytes(m), &sig),
        _ => false,
    }
}

/// Load the exporter's Ed25519 seed (hex, 32 bytes), creating it when the
/// file does not exist yet. Returns the seed and the public key hex.
fn load_or_create_key(path: &Path) -> Result<([u8; 32], String), String> {
    if let Ok(text) = std::fs::read_to_string(path) {
        let bytes = hex::decode(text.trim()).map_err(|e| format!("{}: {e}", path.display()))?;
        let seed: [u8; 32] = bytes.try_into().map_err(|_| format!("{}: expected 32 bytes of hex seed", path.display()))?;
        let pk = key_from_seed(&seed).public_key();
        return Ok((seed, hex::encode(pk.as_ref())));
    }
    let seed: [u8; 32] = rand::random();
    let pk = key_from_seed(&seed).public_key();
    write_atomically(path, hex::encode(seed).as_bytes())?;
    Ok((seed, hex::encode(pk.as_ref())))
}

/// Write `bytes` to `path` under a temporary name, then rename (a mirror
/// never serves a half-written file).
fn write_atomically(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let tmp = path.with_extension(format!("{}.tmp", path.extension().and_then(|e| e.to_str()).unwrap_or("part")));
    let mut f = std::fs::File::create(&tmp).map_err(|e| format!("{}: {e}", tmp.display()))?;
    f.write_all(bytes).and_then(|_| f.sync_all()).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
}

/// A webseed URL for file `name`: `~name~` takes the file name, anything else
/// is a prefix the name is appended to.
fn webseed_url(base: &str, name: &str) -> String {
    match base.split_once("~name~") {
        Some((a, b)) => format!("{a}{name}{b}"),
        None => format!("{}/{}", base.trim_end_matches('/'), name),
    }
}

/// The (size, blake3) of a file, or `None` when it does not read.
fn file_info(path: &Path) -> Option<(u64, String)> {
    let bytes = std::fs::read(path).ok()?;
    Some((bytes.len() as u64, crate::rpc::blake3_hex(&bytes)))
}

/// Cursor shape version: bump when the record changes (an old file is then
/// discarded, which only costs one full pass).
const STATE_VERSION: u32 = 1;
/// At most this many eras are (re)processed in one pass (pre-audit 7
/// PA7-08): the pass runs every 30 seconds beside following and serving, so
/// a backlog larger than this — a fresh archive node, changed arguments —
/// continues in the next pass instead of monopolizing the disk now.
const MAX_ERAS_PER_PASS: usize = 8;

/// A file's cheap fingerprint: metadata only, nothing read.
fn fingerprint(path: &Path) -> Option<(u64, u64)> {
    let m = std::fs::metadata(path).ok()?;
    let d = m.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?;
    Some((m.len(), d.as_nanos() as u64))
}

/// One era a previous pass fully verified and exported (pre-audit 7
/// PA7-08): enough to prove "unchanged" by metadata alone and to rebuild
/// the index entry without reading the file. The magnet is recorded because
/// it is the SHA-1 of the torrent, not derivable from the era's blake3.
#[derive(Serialize, Deserialize)]
struct Exported {
    /// (size, mtime_ns) of the canonical source when it was verified.
    src: (u64, u64),
    /// (size, mtime_ns) of the exported copy.
    dst: (u64, u64),
    /// (size, mtime_ns) of the .torrent and .json sidecars.
    torrent: (u64, u64),
    manifest: (u64, u64),
    blake3: String,
    size: u64,
    magnet: String,
}

/// The exporter's cursor: which eras were fully processed under which
/// arguments. The arguments are fingerprinted too — the derived files name
/// the signer and every mirror, so a change of any of them must void the
/// whole cursor rather than quietly reuse stale sidecars.
#[derive(Serialize, Deserialize)]
struct ExportState {
    version: u32,
    args: String,
    eras: BTreeMap<u64, Exported>,
}

/// Export every sealed era this node keeps to `args.dir`. Each era is fully
/// re-verified (`era::read`) against the root this node's history index holds
/// before anything is written. A pass reads only what is new or changed
/// since the last one (pre-audit 7 PA7-08): verified eras are recorded in a
/// cursor (`.export-state.json`) by metadata fingerprint and skipped without
/// a single read, and at most [`MAX_ERAS_PER_PASS`] eras are processed per
/// pass. Returns how many era file sets were written.
pub fn once(chain: &Chain, args: &ExportArgs) -> Result<usize, String> {
    let store = chain.store().ok_or("no store")?;
    let chain_id = chain.cfg().chain_id;
    let roots = chain.lock().history_index.as_ref().map(|i| i.eras.clone()).ok_or("no history index (history v2 only)")?;
    std::fs::create_dir_all(&args.dir).map_err(|e| format!("{}: {e}", args.dir.display()))?;
    let era_dir = store.era_dir();
    let mut files: Vec<(u64, PathBuf)> = std::fs::read_dir(&era_dir)
        .map_err(|e| format!("{}: {e}", era_dir.display()))?
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            let era: u64 = name.strip_prefix("era-")?.strip_suffix(".aera")?.parse().ok()?;
            Some((era, e.path()))
        })
        .collect();
    files.sort();
    if files.is_empty() {
        return Ok(0);
    }
    let (seed, signer) = load_or_create_key(&args.sign_key)?;
    let args_fp = format!("{signer}|{}|{}", args.https_base.as_deref().unwrap_or(""), args.webseeds.join("\n"));
    let state_path = args.dir.join(".export-state.json");
    let mut state = std::fs::read(&state_path)
        .ok()
        .and_then(|b| serde_json::from_slice::<ExportState>(&b).ok())
        .filter(|s| s.version == STATE_VERSION && s.args == args_fp)
        .unwrap_or(ExportState { version: STATE_VERSION, args: args_fp, eras: BTreeMap::new() });
    let mut written = 0usize;
    let mut index = Vec::with_capacity(files.len());
    let mut processed = 0usize;
    let mut kept = BTreeSet::new();
    for (era, path) in files {
        let Some(root) = roots.get(era as usize).copied() else {
            tracing::warn!(era, "sealed era with no root in the history index; skipped");
            continue;
        };
        let name = crate::era::file_name(era);
        let dst = args.dir.join(&name);
        let torrent_path = args.dir.join(format!("{name}.torrent"));
        let manifest_path = args.dir.join(format!("{name}.json"));
        let first = era * ERA_LEN;
        // Unchanged since a previous pass verified and exported it (pre-audit
        // 7 PA7-08): source, copy and both sidecars all match their recorded
        // fingerprints, so this pass reads nothing. The index entry is rebuilt
        // from the cursor — the root from the history index, the magnet from
        // the record.
        if let Some(done) = state.eras.get(&era).filter(|d| {
            fingerprint(&path) == Some(d.src)
                && fingerprint(&dst) == Some(d.dst)
                && fingerprint(&torrent_path) == Some(d.torrent)
                && fingerprint(&manifest_path) == Some(d.manifest)
        }) {
            kept.insert(era);
            index.push(json!({
                "era": era,
                "first": first,
                "last": first + ERA_LEN - 1,
                "root": hex::encode(root),
                "blake3": done.blake3,
                "size": done.size,
                "file": name,
                "manifest": format!("{name}.json"),
                "torrent": format!("{name}.torrent"),
                "magnet": done.magnet,
            }));
            continue;
        }
        if processed >= MAX_ERAS_PER_PASS {
            tracing::warn!(era, processed, "export pass is full; this era continues in the next pass");
            continue;
        }
        processed += 1;
        // The canonical source is the only origin of an export (pre-audit 7
        // PA7-09): the old code hashed the destination, reread it and
        // compared the two — the destination verifying itself — so a
        // corrupted copy passed as "unchanged" and was fed to era::read,
        // failing the whole pass. Now the bytes always come from the store,
        // and the copy is rewritten whenever it differs from them.
        let Some((src_size, _)) = fingerprint(&path) else {
            tracing::warn!(era, "canonical era unreadable; skipped this pass");
            continue;
        };
        if src_size >= crate::era_net::MAX_ERA_FILE as u64 {
            tracing::warn!(era, src_size, "canonical era over MAX_ERA_FILE; skipped this pass");
            continue;
        }
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!(era, %e, "canonical era unreadable; skipped this pass");
                continue;
            }
        };
        let blake3 = crate::rpc::blake3_hex(&bytes);
        // The set never publishes anything the node itself could not verify —
        // and one era that cannot be verified no longer stops the pass
        // (pre-audit 7 PA7-09): it is skipped with a warning and retried
        // next pass, while the other eras still export.
        let decoded = match crate::era::read(&bytes, Some(&root)) {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!(era, %e, "era does not verify against its history root; skipped this pass");
                continue;
            }
        };
        if decoded.index != era {
            tracing::warn!(era, decoded = decoded.index, "era file number disagrees with its name; skipped this pass");
            continue;
        }
        let copy_ok = fingerprint(&dst).is_some_and(|(s, _)| s == bytes.len() as u64)
            && std::fs::read(&dst).map(|b| crate::rpc::blake3_hex(&b) == blake3).unwrap_or(false);
        if !copy_ok {
            write_atomically(&dst, &bytes)?;
            written += 1;
        }
        // Webseeds and the Https mirror: this node first, the configured ones after.
        let mut urls = args.https_base.iter().map(|b| webseed_url(&format!("{b}/era"), &name)).collect::<Vec<_>>();
        urls.extend(args.webseeds.iter().map(|w| webseed_url(w, &name)));
        let (torrent_bytes, info_hash) = torrent(&name, &bytes, &urls, PIECE_LEN);
        let magnet = magnet(&info_hash, &name, &urls);
        if file_info(&torrent_path).map(|(_, b)| b != blake3_of(&torrent_bytes)).unwrap_or(true) {
            write_atomically(&torrent_path, &torrent_bytes)?;
            written += 1;
        }
        let mut mirrors = Vec::new();
        if let Some(base) = &args.https_base {
            mirrors.push(Mirror::Https { url: webseed_url(&format!("{base}/era"), &name) });
        }
        mirrors.push(Mirror::Torrent { magnet: magnet.clone(), webseeds: urls.clone() });
        let mut m = Manifest {
            version: MANIFEST_VERSION,
            chain_id,
            kind: ManifestKind::HistoryChunk,
            range: Some((first, first + ERA_LEN - 1)),
            blake3: blake3.clone(),
            size: bytes.len() as u64,
            mirrors,
            signature: String::new(),
        };
        sign_manifest(&mut m, &seed);
        let manifest_bytes = serde_json::to_vec_pretty(&m).map_err(|e| e.to_string())?;
        if std::fs::read(&manifest_path).ok().as_deref() != Some(manifest_bytes.as_slice()) {
            write_atomically(&manifest_path, &manifest_bytes)?;
            written += 1;
        }
        // Record what this pass verified, so the next one can skip by
        // metadata (an unreadable fingerprint records (0,0), which never
        // matches — the era is simply reprocessed next pass).
        state.eras.insert(era, Exported {
            src: fingerprint(&path).unwrap_or_default(),
            dst: fingerprint(&dst).unwrap_or_default(),
            torrent: fingerprint(&torrent_path).unwrap_or_default(),
            manifest: fingerprint(&manifest_path).unwrap_or_default(),
            blake3: blake3.clone(),
            size: bytes.len() as u64,
            magnet: magnet.clone(),
        });
        kept.insert(era);
        index.push(json!({
            "era": era,
            "first": first,
            "last": first + ERA_LEN - 1,
            "root": hex::encode(root),
            "blake3": blake3,
            "size": bytes.len(),
            "file": name,
            "manifest": format!("{name}.json"),
            "torrent": format!("{name}.torrent"),
            "magnet": magnet,
        }));
    }
    // Eras whose sources are gone (pruned, moved) leave the cursor with
    // them: the file set and its record stay in step.
    state.eras.retain(|e, _| kept.contains(e));
    let index_doc = json!({
        "version": 1,
        "chain_id": chain_id,
        "era_len": ERA_LEN,
        "signer": signer,
        "eras": index,
    });
    let index_bytes = serde_json::to_vec_pretty(&index_doc).map_err(|e| e.to_string())?;
    let index_path = args.dir.join("index.json");
    if std::fs::read(&index_path).ok().as_deref() != Some(index_bytes.as_slice()) {
        write_atomically(&index_path, &index_bytes)?;
        written += 1;
    }
    write_atomically(&state_path, &serde_json::to_vec_pretty(&state).map_err(|e| e.to_string())?)?;
    Ok(written)
}

fn blake3_of(b: &[u8]) -> String {
    crate::rpc::blake3_hex(b)
}

/// Export newly sealed eras every [`INTERVAL`] (the archive node's background
/// task; `once` is the testable half).
pub async fn run(chain: Chain, args: ExportArgs) {
    loop {
        let (c, a) = (chain.clone(), args.clone());
        match tokio::task::spawn_blocking(move || once(&c, &a)).await {
            Ok(Ok(0)) => {}
            Ok(Ok(n)) => tracing::info!(n, "era export set written"),
            Ok(Err(e)) => tracing::warn!(%e, "era export failed; will retry"),
            Err(e) => tracing::warn!(%e, "era export task failed"),
        }
        tokio::time::sleep(INTERVAL).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bencode_round_trips_the_torrent_shapes() {
        // The BEP-003 examples.
        assert_eq!(ben_encode(Ben::Int(52)), b"i52e");
        assert_eq!(ben_encode(Ben::Bytes(b"spam".to_vec())), b"4:spam");
        assert_eq!(ben_encode(Ben::List(vec![Ben::Bytes(b"spam".to_vec()), Ben::Int(52)])), b"l4:spami52ee");
        // Keys sort, whatever order they were given in.
        let d = Ben::Dict(vec![(b"z".to_vec(), Ben::Int(1)), (b"a".to_vec(), Ben::Int(2))]);
        assert_eq!(ben_encode(d), b"d1:ai2e1:zi1ee");
        for v in [
            Ben::Int(-7),
            Ben::Bytes(vec![]),
            Ben::List(vec![Ben::Bytes(vec![0, 1, 2]), Ben::Int(0), Ben::List(vec![])]),
            Ben::Dict(vec![(b"info".to_vec(), Ben::Bytes(b"x".to_vec()))]),
        ] {
            let enc = ben_encode(v.clone());
            assert_eq!(ben_decode(&enc).unwrap().0, v);
        }
        assert!(ben_decode(b"i52").is_err(), "unterminated");
        assert!(ben_decode(b"4:spa").is_err(), "short string");
        assert!(ben_decode(b"i52ee").is_err(), "trailing");
        assert!(ben_decode(b"x").is_err(), "not bencode");
    }

    #[test]
    fn torrent_hashes_the_pieces_and_the_info_dict() {
        let file: Vec<u8> = (0..300_000u32).map(|i| i as u8).collect();
        // Callers pass finished URLs (once() runs webseed_url first); a bare
        // URL would be served as-is and a webseed client could not use it.
        let webseeds = vec![webseed_url("http://nas/eras", "era-00000000.aera")];
        let (bytes, info_hash) = torrent("era-00000000.aera", &file, &webseeds, PIECE_LEN);
        let (top, _) = ben_decode(&bytes).unwrap();
        let Ben::Dict(top) = top else { panic!("top") };
        let get = |k: &str| top.iter().find(|(key, _)| key == k.as_bytes()).expect(k);
        let Ben::Dict(info) = get("info").1.clone() else { panic!("info") };
        let pieces = match info.iter().find(|(k, _)| k == b"pieces").unwrap().1 {
            Ben::Bytes(ref p) => p,
            _ => panic!("pieces"),
        };
        assert_eq!(pieces.len(), file.len().div_ceil(PIECE_LEN) * 20);
        for (i, chunk) in file.chunks(PIECE_LEN).enumerate() {
            assert_eq!(&pieces[i * 20..(i + 1) * 20], &sha1(chunk));
        }
        // The info hash is over exactly the info dict, re-encoded.
        let info_bytes = ben_encode(get("info").1.clone());
        assert_eq!(&info_hash[..], &sha1(&info_bytes)[..]);
        let Ben::List(urls) = get("url-list").1.clone() else { panic!("url-list") };
        assert_eq!(urls, vec![Ben::Bytes(b"http://nas/eras/era-00000000.aera".to_vec())]);
        // A custom piece length is honored (clamped up to 16 KiB minimum).
        let (b2, ih2) = torrent("n", &file, &[], 4 * 1024);
        let (_t2, _) = ben_decode(&b2).unwrap();
        assert_ne!(ih2, info_hash, "different pieces, different hash");
        assert!(b2.windows(8).find(|w| w == b"url-lis").is_none(), "no webseeds, no url-list");
    }

    #[test]
    fn magnet_urlencodes_only_what_needs_it() {
        let m = magnet(&[1u8; 20], "era-00000000.aera", &["http://100.100.59.78:8545/era/era-00000000.aera".to_string()]);
        assert_eq!(
            m,
            "magnet:?xt=urn:btih:0101010101010101010101010101010101010101&dn=era-00000000.aera\
             &ws=http%3A%2F%2F100.100.59.78%3A8545%2Fera%2Fera-00000000.aera"
        );
    }

    #[test]
    fn webseed_urls_take_a_placeholder_or_a_prefix() {
        assert_eq!(webseed_url("https://gh/releases/download/eras", "era-1.aera"), "https://gh/releases/download/eras/era-1.aera");
        assert_eq!(webseed_url("https://gh/releases/download/eras/~name~?token=x", "era-1.aera"), "https://gh/releases/download/eras/era-1.aera?token=x");
        assert_eq!(webseed_url("http://x/era/", "f"), "http://x/era/f");
    }

    #[test]
    fn manifests_sign_and_verify_under_the_namespace() {
        let (seed, signer) = load_or_create_key(&std::env::temp_dir().join(format!("aether-export-key-{}", std::process::id()))).unwrap();
        let mut m = Manifest {
            version: 1,
            chain_id: 7_780,
            kind: ManifestKind::HistoryChunk,
            range: Some((0, ERA_LEN - 1)),
            blake3: "ab".into(),
            size: 2,
            mirrors: vec![Mirror::Torrent { magnet: "magnet:?xt=urn:btih:00".into(), webseeds: vec![] }],
            signature: String::new(),
        };
        assert!(!verify_manifest(&m, &signer), "no signature is not verified");
        sign_manifest(&mut m, &seed);
        assert!(verify_manifest(&m, &signer));
        // Any change to the covered bytes breaks it, and so does another key.
        let mut tampered = m.clone();
        tampered.size += 1;
        assert!(!verify_manifest(&tampered, &signer));
        tampered = m.clone();
        tampered.blake3 = "cd".into();
        assert!(!verify_manifest(&tampered, &signer));
        // The signature is what gets checked: any other value — even a
        // well-formed 64-byte one — fails (emptying it is the rule the signer
        // follows, so the covered bytes never contain it).
        tampered = m.clone();
        tampered.signature = "00".repeat(64);
        assert!(!verify_manifest(&tampered, &signer));
        let (_, other) = load_or_create_key(&std::env::temp_dir().join(format!("aether-export-key2-{}", std::process::id()))).unwrap();
        assert!(!verify_manifest(&m, &other));
    }

    /// Two sealed eras in a store, roots in the chain's history index — the
    /// exporter's whole input. The same synthetic chain era's own tests
    /// build, trimmed to what `once` reads.
    fn export_fixture(tag: &str) -> (Chain, ExportArgs) {
        use crate::block::{Block, Context, Payload, PublicKey};
        use aether_hash::ChainHasher;
        use aether_state::mmr::{EraIndex, Mmr};
        use aether_types::{B256, GasVector};
        use commonware_consensus::types::{Epoch, Height, Round, View};
        use commonware_cryptography::{ed25519, sha256::Digest, Digestible, Hasher as _};

        fn digest_of(d: &Digest) -> aether_hash::Digest {
            d.as_ref().try_into().expect("sha256 digest is 32 bytes")
        }

        let dir = std::env::temp_dir().join(format!("aether-export-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = crate::store::Store::open(&dir.join("db.redb")).unwrap();
        std::fs::create_dir_all(store.era_dir()).unwrap();
        let h = ChainHasher::new();
        let leaders: Vec<PublicKey> = (0..4).map(|i| ed25519::PrivateKey::from_seed(i).public_key()).collect();
        let genesis = Block::genesis(7781, B256::repeat_byte(1));
        let (mut blocks, mut mmrs) = (vec![genesis.clone()], vec![Mmr::default()]);
        let mut mmr = Mmr::default().append(&h, 0, &digest_of(&genesis.digest()));
        for height in 1..2 * ERA_LEN {
            mmrs.push(mmr.clone());
            let prev = blocks.last().unwrap();
            let state_root = B256::from(digest_of(&commonware_cryptography::Sha256::hash(&[height.to_be_bytes().as_slice()])));
            let payload = Payload {
                version: 2,
                parent_state_root: state_root,
                history_root: B256::from(mmr.root(&h)),
                receipts_root: None,
                parent_meta: B256::repeat_byte((height / 5000) as u8),
                gas: GasVector::default(),
                txs: vec![],
                ..Default::default()
            };
            let view = height + height / 1000;
            let context = Context {
                round: Round::new(Epoch::zero(), View::new(view)),
                leader: leaders[(view % 4) as usize].clone(),
                parent: (View::new(prev.context.round.view().get()), prev.digest()),
            };
            let b = Block::new(context, prev.digest(), Height::new(height), 1_790_000_000_000 + height * 1000, payload.to_bytes());
            mmr = mmr.append(&h, height, &digest_of(&b.digest()));
            blocks.push(b);
        }
        mmrs.push(mmr);
        let mut roots = Vec::new();
        for era in 0..2u64 {
            let (a, b) = ((era * ERA_LEN) as usize, ((era + 1) * ERA_LEN) as usize);
            let bytes = crate::era::write(&mmrs[a], &blocks[a..b]).unwrap();
            std::fs::write(store.era_dir().join(crate::era::file_name(era)), &bytes).unwrap();
            roots.push(crate::era::read(&bytes, None).unwrap().root);
        }
        let (chain, _) = Chain::open(crate::chain::ChainConfig {
            chain_id: 7781,
            limits: GasVector { exec: 30_000_000, state: u64::MAX, prove: 200_000_000 },
            alloc: vec![], fees: false, registrar: None, epoch_blocks: 0,
            min_streak: None, draw_epochs: None, history_v2: false, protocol: 1,
            node_rewards: false, committee: vec![], reserve: None, group: 0,
            max_committee: crate::rotation::GROW_UNTIL,
        }, store).unwrap();
        chain.lock().history_index = Some(std::sync::Arc::new(EraIndex { eras: roots, open: vec![] }));
        let args = ExportArgs {
            dir: dir.join("out"),
            webseeds: vec!["http://mirror.example/eras".into()],
            https_base: None,
            sign_key: dir.join("sign.key"),
        };
        (chain, args)
    }

    /// An unchanged pass reads nothing (pre-audit 7 PA7-08): once both eras
    /// are exported, making every canonical source and exported copy
    /// unreadable changes nothing — the pass still answers Ok and rewrites
    /// no file, because it never opens what it already verified. The old
    /// code reread every file each pass (each copy three times over) and
    /// failed here.
    #[test]
    fn unchanged_eras_are_never_reread_between_passes() {
        let (chain, args) = export_fixture("cursor");
        assert!(once(&chain, &args).unwrap() > 0, "the first pass exports both eras");
        use std::os::unix::fs::PermissionsExt;
        let mut touched = Vec::new();
        let store = chain.store().unwrap();
        for era in 0..2u64 {
            touched.push(store.era_dir().join(crate::era::file_name(era)));
            touched.push(args.dir.join(crate::era::file_name(era)));
        }
        for p in &touched {
            let mut perm = std::fs::metadata(p).unwrap().permissions();
            perm.set_mode(0o000);
            std::fs::set_permissions(p, perm).unwrap();
        }
        // Metadata is still readable (only open() is refused): a pass that
        // trusts its cursor never opens these; the old one did and errored.
        let second = once(&chain, &args);
        for p in &touched {
            let mut perm = std::fs::metadata(p).unwrap().permissions();
            perm.set_mode(0o644);
            std::fs::set_permissions(p, perm).unwrap();
        }
        assert_eq!(second.unwrap(), 0, "an unchanged pass writes nothing");
        let _ = std::fs::remove_dir_all(args.dir.parent().unwrap());
    }

    /// A damaged export copy is rewritten from the canonical source
    /// (pre-audit 7 PA7-09): the old verification compared the destination
    /// with itself (hash the file, reread it, compare), so a corrupted copy
    /// verified as "unchanged", was fed to era::read and killed the pass.
    #[test]
    fn a_damaged_destination_is_rewritten_from_the_canonical_source() {
        let (chain, args) = export_fixture("damage");
        once(&chain, &args).unwrap();
        let dst = args.dir.join(crate::era::file_name(0));
        let good = std::fs::read(&dst).unwrap();
        std::fs::write(&dst, &good[..good.len() - 8]).unwrap();
        let n = once(&chain, &args).unwrap();
        assert!(n >= 1, "the damaged copy is rewritten: {n}");
        assert_eq!(std::fs::read(&dst).unwrap(), good, "the copy is the canonical bytes again");
        let index: serde_json::Value = serde_json::from_slice(&std::fs::read(args.dir.join("index.json")).unwrap()).unwrap();
        let e0 = index["eras"].as_array().unwrap().iter().find(|e| e["era"] == 0).unwrap();
        assert_eq!(e0["blake3"], crate::rpc::blake3_hex(&good));
        let _ = std::fs::remove_dir_all(args.dir.parent().unwrap());
    }

    /// One era's failure never stops the pass (pre-audit 7 PA7-09): a
    /// corrupt canonical era is skipped with a warning and retried next
    /// pass — the other eras still export and the index still lists them.
    /// The old code aborted the whole pass with `?` on the first bad era,
    /// and did so again every 30 seconds.
    #[test]
    fn one_unreadable_era_does_not_stop_the_pass() {
        let (chain, args) = export_fixture("isolate");
        once(&chain, &args).unwrap();
        let store = chain.store().unwrap();
        // Break era 1's magic: era::read refuses the file outright, and the
        // export copy is damaged too, so neither copy can answer this pass.
        let src = store.era_dir().join(crate::era::file_name(1));
        let mut bytes = std::fs::read(&src).unwrap();
        bytes[0] ^= 0xff;
        std::fs::write(&src, &bytes).unwrap();
        std::fs::write(args.dir.join(crate::era::file_name(1)), b"corrupt").unwrap();
        once(&chain, &args).unwrap();
        let index: serde_json::Value = serde_json::from_slice(&std::fs::read(args.dir.join("index.json")).unwrap()).unwrap();
        let eras: Vec<u64> = index["eras"].as_array().unwrap().iter().map(|e| e["era"].as_u64().unwrap()).collect();
        assert_eq!(eras, vec![0], "era 1 is skipped with a warning; era 0 stays exported and listed");
        // And the damage stays contained: era 0's copy is still the canonical bytes.
        let src0 = std::fs::read(store.era_dir().join(crate::era::file_name(0))).unwrap();
        assert_eq!(std::fs::read(args.dir.join(crate::era::file_name(0))).unwrap(), src0);
        let _ = std::fs::remove_dir_all(args.dir.parent().unwrap());
    }
}
