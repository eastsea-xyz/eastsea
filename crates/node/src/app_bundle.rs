//! Hash-addressed static apps (design 31 §4). The bundle identity is SHA-256
//! of the canonical index, never of its deterministic ustar transport.

use aether_net::{Endpoint, EndpointAddr};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use futures::StreamExt as _;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const MAX_BUNDLE: usize = 20_000_000;
pub const MAX_FILE: usize = 10 * 1024 * 1024;
pub const MAX_FILES: usize = 2_000;
pub const MAX_INDEX: usize = 1024 * 1024;
pub const CACHE_BYTES: u64 = 500 * 1024 * 1024;
const MAX_PEERS: usize = 32;
const MAX_CACHE_BUNDLES: usize = 256;
const FORMAT: &str = "eastsea-bundle/1";

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FileEntry {
    pub path: String,
    pub sha256: String,
    pub size: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Index {
    pub format: String,
    pub files: Vec<FileEntry>,
}

pub fn sha256(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

pub fn normalize_hash(hash: &str) -> Result<String, String> {
    let hash = hash.strip_prefix("0x").unwrap_or(hash);
    if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("bundle hash must be 32 bytes of SHA-256 hex".into());
    }
    Ok(hash.to_ascii_lowercase())
}

/// Paths are relative, ASCII and interpreted once, without URL decoding.
pub fn validate_path(path: &str) -> Result<(), String> {
    if path.is_empty()
        || path.len() > 200
        || !path
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._/-".contains(&b))
        || path
            .split('/')
            .any(|p| p.is_empty() || p == "." || p == "..")
    {
        return Err("app path must be a canonical relative static-file path".into());
    }
    let ext = path.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase());
    if !ext.as_deref().is_some_and(|e| {
        matches!(
            e,
            "html"
                | "js"
                | "mjs"
                | "css"
                | "json"
                | "svg"
                | "png"
                | "jpg"
                | "webp"
                | "ico"
                | "woff2"
                | "txt"
                | "wasm"
        )
    }) {
        return Err(format!("unsupported app file extension: {path}"));
    }
    Ok(())
}

fn validate_index(bytes: &[u8], expected: Option<&str>) -> Result<Index, String> {
    if bytes.len() > MAX_INDEX {
        return Err("app index exceeds 1 MiB".into());
    }
    if let Some(hash) = expected {
        if sha256(bytes) != normalize_hash(hash)? {
            return Err("app bundle hash mismatch".into());
        }
    }
    let index: Index = serde_json::from_slice(bytes).map_err(|e| format!("app index: {e}"))?;
    if index.format != FORMAT || index.files.is_empty() || index.files.len() > MAX_FILES {
        return Err("app index format or file count is invalid".into());
    }
    if serde_json::to_vec(&index).map_err(|e| e.to_string())? != bytes {
        return Err("app index is not canonical JSON".into());
    }
    let mut previous: Option<&str> = None;
    let mut folded = BTreeSet::new();
    let mut total = 0u64;
    for file in &index.files {
        validate_path(&file.path)?;
        if file.path.eq_ignore_ascii_case("bundle.json")
            || previous.is_some_and(|p| p >= file.path.as_str())
            || !folded.insert(file.path.to_ascii_lowercase())
        {
            return Err("app index has a duplicate, reserved, or unsorted path".into());
        }
        if normalize_hash(&file.sha256)? != file.sha256 || file.size > MAX_FILE as u64 {
            return Err("app file hash or size is invalid".into());
        }
        total = total
            .checked_add(file.size)
            .ok_or("app file sizes overflow")?;
        if total > MAX_BUNDLE as u64 {
            return Err("app files exceed 20 MB".into());
        }
        previous = Some(&file.path);
    }
    Ok(index)
}

fn tar_header(path: &str, size: usize) -> Result<[u8; 512], String> {
    let mut header = [0u8; 512];
    let (prefix, name) = if path.len() <= 100 {
        ("", path)
    } else {
        path.rmatch_indices('/')
            .find_map(|(i, _)| {
                (i <= 155 && path.len() - i - 1 <= 100).then_some((&path[..i], &path[i + 1..]))
            })
            .ok_or_else(|| format!("app path cannot be encoded as ustar: {path}"))?
    };
    header[..name.len()].copy_from_slice(name.as_bytes());
    header[100..108].copy_from_slice(b"0000644\0");
    header[108..116].copy_from_slice(b"0000000\0");
    header[116..124].copy_from_slice(b"0000000\0");
    header[124..136].copy_from_slice(format!("{size:011o}\0").as_bytes());
    header[136..148].copy_from_slice(b"00000000000\0");
    header[148..156].fill(b' ');
    header[156] = b'0';
    header[257..263].copy_from_slice(b"ustar\0");
    header[263..265].copy_from_slice(b"00");
    header[345..345 + prefix.len()].copy_from_slice(prefix.as_bytes());
    let checksum: usize = header.iter().map(|b| *b as usize).sum();
    header[148..156].copy_from_slice(format!("{checksum:06o}\0 ").as_bytes());
    Ok(header)
}

fn tar_name(field: &[u8]) -> Result<&str, String> {
    let end = field.iter().position(|b| *b == 0).unwrap_or(field.len());
    if field[end..].iter().any(|b| *b != 0) {
        return Err("noncanonical tar name".into());
    }
    std::str::from_utf8(&field[..end]).map_err(|_| "tar path is not UTF-8".into())
}

/// A fully verified transport kept in memory, with file ranges rather than
/// extracted filesystem paths. Serving never follows archive-supplied paths.
#[derive(Debug)]
pub struct Bundle {
    hash: String,
    index: Vec<u8>,
    archive: Vec<u8>,
    ranges: BTreeMap<String, std::ops::Range<usize>>,
}

impl Bundle {
    pub fn hash(&self) -> &str {
        &self.hash
    }
    pub fn index_bytes(&self) -> &[u8] {
        &self.index
    }
    pub fn archive(&self) -> &[u8] {
        &self.archive
    }

    pub fn from_folder(folder: &Path) -> Result<Self, String> {
        let metadata = std::fs::symlink_metadata(folder).map_err(|e| e.to_string())?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err("app folder must be a directory, without a symbolic link".into());
        }
        fn walk(
            root: &Path,
            directory: &Path,
            depth: usize,
            files: &mut BTreeMap<String, Vec<u8>>,
            total: &mut usize,
        ) -> Result<(), String> {
            if depth > 100 {
                return Err("app folder nesting exceeds the path bound".into());
            }
            for item in std::fs::read_dir(directory).map_err(|e| e.to_string())? {
                let path = item.map_err(|e| e.to_string())?.path();
                let m = std::fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
                if m.file_type().is_symlink() {
                    return Err(format!(
                        "symbolic links are not app files: {}",
                        path.display()
                    ));
                }
                if m.is_dir() {
                    walk(root, &path, depth + 1, files, total)?;
                    continue;
                }
                if !m.is_file() {
                    return Err("app folder contains a non-regular file".into());
                }
                let name = path
                    .strip_prefix(root)
                    .map_err(|e| e.to_string())?
                    .to_str()
                    .ok_or("app path is not UTF-8")?
                    .to_string();
                validate_path(&name)?;
                if name.eq_ignore_ascii_case("bundle.json") {
                    return Err("bundle.json is reserved for the generated index".into());
                }
                if files.len() >= MAX_FILES || m.len() > MAX_FILE as u64 {
                    return Err("app file count or file size exceeds its bound".into());
                }
                let bytes = read_regular(&path, MAX_FILE)?;
                *total = total.checked_add(bytes.len()).ok_or("app size overflow")?;
                if *total > MAX_BUNDLE {
                    return Err("app files exceed 20 MB".into());
                }
                files.insert(name, bytes);
            }
            Ok(())
        }
        let mut files = BTreeMap::new();
        let mut total = 0;
        walk(folder, folder, 0, &mut files, &mut total)?;
        let index = Index {
            format: FORMAT.into(),
            files: files
                .iter()
                .map(|(path, bytes)| FileEntry {
                    path: path.clone(),
                    sha256: sha256(bytes),
                    size: bytes.len() as u64,
                })
                .collect(),
        };
        let index_bytes = serde_json::to_vec(&index).map_err(|e| e.to_string())?;
        validate_index(&index_bytes, None)?;
        files.insert("bundle.json".into(), index_bytes);
        let size = files
            .values()
            .try_fold(1024usize, |n, b| {
                n.checked_add(512 + b.len().div_ceil(512) * 512)
            })
            .ok_or("app tar size overflow")?;
        if size > MAX_BUNDLE {
            return Err("app ustar bundle exceeds 20 MB".into());
        }
        let mut archive = Vec::with_capacity(size);
        for (path, bytes) in files {
            archive.extend_from_slice(&tar_header(&path, bytes.len())?);
            archive.extend_from_slice(&bytes);
            archive.resize(archive.len().div_ceil(512) * 512, 0);
        }
        archive.resize(archive.len() + 1024, 0);
        Self::from_archive(archive, None)
    }

    pub fn from_archive(archive: Vec<u8>, expected: Option<&str>) -> Result<Self, String> {
        if archive.len() > MAX_BUNDLE || archive.len() < 1024 || archive.len() % 512 != 0 {
            return Err("app ustar size is invalid or exceeds 20 MB".into());
        }
        let mut ranges = BTreeMap::new();
        let mut folded = BTreeSet::new();
        let mut offset = 0usize;
        let mut previous = String::new();
        loop {
            let header = archive
                .get(offset..offset + 512)
                .ok_or("truncated app tar header")?;
            if header.iter().all(|b| *b == 0) {
                if archive.len() != offset + 1024 || archive[offset..].iter().any(|b| *b != 0) {
                    return Err("app tar must end with exactly two zero blocks".into());
                }
                break;
            }
            let name = tar_name(&header[..100])?;
            let prefix = tar_name(&header[345..500])?;
            let path = if prefix.is_empty() {
                name.to_string()
            } else {
                format!("{prefix}/{name}")
            };
            validate_path(&path)?;
            if path <= previous
                || !folded.insert(path.to_ascii_lowercase())
                || ranges.len() > MAX_FILES
            {
                return Err("app tar paths are duplicate or unsorted".into());
            }
            let raw_size = tar_name(&header[124..136])?;
            let size = usize::from_str_radix(raw_size, 8).map_err(|_| "invalid app tar size")?;
            if size > MAX_FILE || (path == "bundle.json" && size > MAX_INDEX) {
                return Err("app tar file exceeds its bound".into());
            }
            if header != tar_header(&path, size)?.as_slice() {
                return Err("app tar header is not canonical ustar (regular files only)".into());
            }
            let start = offset + 512;
            let end = start.checked_add(size).ok_or("app tar size overflow")?;
            let padded_end = end.div_ceil(512) * 512;
            if padded_end > archive.len() || archive[end..padded_end].iter().any(|b| *b != 0) {
                return Err("app tar file is truncated or has nonzero padding".into());
            }
            ranges.insert(path.clone(), start..end);
            previous = path;
            offset = padded_end;
        }
        let index_bytes = ranges
            .get("bundle.json")
            .map(|r| archive[r.clone()].to_vec())
            .ok_or("app tar has no bundle.json index")?;
        let index = validate_index(&index_bytes, expected)?;
        if ranges.len() != index.files.len() + 1 {
            return Err("app tar has unindexed files".into());
        }
        for file in &index.files {
            let range = ranges
                .get(&file.path)
                .ok_or("app tar is missing an indexed file")?;
            let bytes = &archive[range.clone()];
            if bytes.len() as u64 != file.size || sha256(bytes) != file.sha256 {
                return Err(format!("app file hash or size mismatch: {}", file.path));
            }
        }
        Ok(Self {
            hash: sha256(&index_bytes),
            index: index_bytes,
            archive,
            ranges,
        })
    }

    pub fn file(&self, path: &str) -> Result<&[u8], String> {
        validate_path(path)?;
        let range = self
            .ranges
            .get(path)
            .ok_or_else(|| format!("app file is not in its verified index: {path}"))?;
        Ok(&self.archive[range.clone()])
    }

    pub fn response(&self, path: &str) -> Result<Value, String> {
        let bytes = self.file(path)?;
        Ok(
            json!({ "bundleHash": self.hash(), "path": path, "sha256": sha256(bytes), "size": bytes.len(), "data": STANDARD.encode(bytes) }),
        )
    }
}

fn read_regular(path: &Path, limit: usize) -> Result<Vec<u8>, String> {
    use std::os::unix::fs::OpenOptionsExt as _;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let meta = file.metadata().map_err(|e| e.to_string())?;
    if !meta.is_file() || meta.len() > limit as u64 {
        return Err("app file is not regular or exceeds its size bound".into());
    }
    let mut bytes = Vec::with_capacity(meta.len() as usize);
    file.take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > limit {
        return Err("app file grew past its size bound".into());
    }
    Ok(bytes)
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub enabled: bool,
    pub seed: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: true,
            seed: false,
        }
    }
}

pub fn configuration(data: &Path) -> Result<Config, String> {
    let path = data.join("apps/config.json");
    if !path.exists() {
        return Ok(Config::default());
    }
    serde_json::from_slice(&read_regular(&path, 4096)?)
        .map_err(|e| format!("app cache configuration: {e}"))
}

pub fn configure(data: &Path, config: Config) -> Result<(), String> {
    let bytes = serde_json::to_vec(&config).map_err(|e| e.to_string())?;
    crate::atomic::replace(&data.join("apps/config.json"), &bytes, 0o600)
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CacheEntry {
    size: u64,
    used: u64,
    pinned: bool,
}

/// A content-addressed disk cache. Pins prevent LRU eviction, never the
/// capacity or free-space guard. Archives are re-verified before every read.
pub struct Cache {
    dir: PathBuf,
    config: Config,
    capacity: u64,
    floor: u64,
    entries: Mutex<BTreeMap<String, CacheEntry>>,
}

impl Cache {
    pub fn open(data: &Path) -> Result<Self, String> {
        let floor = crate::resources::monitor()
            .map(|m| m.limits.min_free_disk)
            .unwrap_or_else(|| crate::resources::Limits::default().min_free_disk);
        Self::open_with_limits(data, CACHE_BYTES, floor)
    }

    pub fn open_with_limits(
        data: &Path,
        capacity: u64,
        min_free_disk: u64,
    ) -> Result<Self, String> {
        let config = configuration(data)?;
        let cache = Self {
            dir: data.join("apps"),
            config,
            capacity: capacity.min(CACHE_BYTES),
            floor: min_free_disk,
            entries: Mutex::new(BTreeMap::new()),
        };
        // A new/disabled cache creates nothing. A concurrent CLI import will
        // be discovered by refresh on the first request.
        if !cache.enabled() || !cache.dir.exists() {
            return Ok(cache);
        }
        if std::fs::symlink_metadata(&cache.dir)
            .map_err(|e| e.to_string())?
            .file_type()
            .is_symlink()
        {
            return Err("app cache directory cannot be a symbolic link".into());
        }
        // Lock before discovering pin state or archives, and retain the same
        // lock through eviction. Otherwise a CLI pin could race this scan.
        let _process = cache.process_lock(true)?;
        let state = cache.dir.join("cache.json");
        let saved: BTreeMap<String, CacheEntry> = if state.exists() {
            serde_json::from_slice(&read_regular(&state, MAX_INDEX)?).unwrap_or_default()
        } else {
            BTreeMap::new()
        };
        let mut entries = BTreeMap::new();
        for item in std::fs::read_dir(&cache.dir).map_err(|e| e.to_string())? {
            let item = item.map_err(|e| e.to_string())?;
            let name = item.file_name();
            let Some(hash) = name.to_str().and_then(|n| n.strip_suffix(".tar")) else {
                continue;
            };
            if normalize_hash(hash).ok().as_deref() != Some(hash) {
                continue;
            }
            let m = std::fs::symlink_metadata(item.path()).map_err(|e| e.to_string())?;
            if !m.is_file() || m.len() > MAX_BUNDLE as u64 {
                continue;
            }
            let entry = saved.get(hash);
            entries.insert(
                hash.to_string(),
                CacheEntry {
                    size: m.len(),
                    used: entry.map(|e| e.used).unwrap_or(0),
                    pinned: entry.is_some_and(|e| e.pinned),
                },
            );
        }
        cache.room_locked(&mut entries, 0, None)?;
        *cache.entries.lock().map_err(|_| "app cache lock")? = entries;
        Ok(cache)
    }

    pub fn enabled(&self) -> bool {
        self.config.enabled
    }
    pub fn seeding(&self) -> bool {
        self.config.enabled && self.config.seed
    }

    /// Check the actual volume before a read/download allocation, and again
    /// immediately before publishing bytes (not the watchdog's old sample).
    pub fn check_disk(&self, incoming: u64) -> Result<(), String> {
        if !self.enabled() {
            return Err("app bundle downloads and cache are disabled".into());
        }
        if !crate::resources::disk_ok() {
            return Err("app cache write refused: node disk floor reached".into());
        }
        if self.floor > 0
            && !crate::resources::free_disk(&self.dir)
                .is_some_and(|free| free.saturating_sub(incoming) >= self.floor)
        {
            return Err("app cache write refused: insufficient space above the disk floor".into());
        }
        Ok(())
    }

    fn save(&self, entries: &BTreeMap<String, CacheEntry>) -> Result<(), String> {
        let bytes = serde_json::to_vec(entries).map_err(|e| e.to_string())?;
        self.check_disk(bytes.len() as u64)?;
        crate::atomic::replace(&self.dir.join("cache.json"), &bytes, 0o600)
    }

    /// Coordinate a running node with a local `app-bundle pin` process. The
    /// descriptor's close releases flock, including error paths.
    fn process_lock(&self, create: bool) -> Result<Option<std::fs::File>, String> {
        use std::os::fd::AsRawFd as _;
        use std::os::unix::fs::OpenOptionsExt as _;
        let path = self.dir.join(".lock");
        if !path.exists() {
            if !create {
                return Ok(None);
            }
            self.check_disk(4096)?;
            std::fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        }
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create(create)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path)
            .map_err(|e| e.to_string())?;
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err("app cache is being updated by another process; retry shortly".into());
        }
        Ok(Some(file))
    }

    fn refresh(&self, entries: &mut BTreeMap<String, CacheEntry>) -> Result<(), String> {
        let path = self.dir.join("cache.json");
        if !path.exists() {
            return Ok(());
        }
        let saved: BTreeMap<String, CacheEntry> =
            serde_json::from_slice(&read_regular(&path, MAX_INDEX)?).unwrap_or_default();
        if saved.len() > MAX_CACHE_BUNDLES {
            return Err("app cache metadata exceeds its entry bound".into());
        }
        for (hash, mut entry) in saved {
            if normalize_hash(&hash).ok().as_deref() != Some(hash.as_str()) {
                continue;
            }
            let file = self.dir.join(format!("{hash}.tar"));
            let Ok(meta) = std::fs::symlink_metadata(file) else {
                entries.remove(&hash);
                continue;
            };
            if !meta.is_file() || meta.len() > MAX_BUNDLE as u64 {
                entries.remove(&hash);
                continue;
            }
            entry.size = meta.len();
            entry.used = entry
                .used
                .max(entries.get(&hash).map(|e| e.used).unwrap_or(0));
            entries.insert(hash, entry);
        }
        entries.retain(|hash, _| self.dir.join(format!("{hash}.tar")).exists());
        Ok(())
    }

    fn room_locked(
        &self,
        entries: &mut BTreeMap<String, CacheEntry>,
        incoming: u64,
        replacing: Option<&str>,
    ) -> Result<(), String> {
        loop {
            let used: u64 = entries
                .iter()
                .filter(|(hash, _)| Some(hash.as_str()) != replacing)
                .map(|(_, entry)| entry.size)
                .sum();
            let count = entries
                .keys()
                .filter(|hash| Some(hash.as_str()) != replacing)
                .count()
                + usize::from(incoming > 0);
            if used.saturating_add(incoming) <= self.capacity && count <= MAX_CACHE_BUNDLES {
                return Ok(());
            }
            let victim = entries
                .iter()
                .filter(|(hash, entry)| !entry.pinned && Some(hash.as_str()) != replacing)
                .min_by_key(|(_, entry)| entry.used)
                .map(|(hash, _)| hash.clone())
                .ok_or("app cache is full of pinned bundles; unpin an app first")?;
            std::fs::remove_file(self.dir.join(format!("{victim}.tar")))
                .map_err(|e| e.to_string())?;
            entries.remove(&victim);
        }
    }

    pub fn put(&self, bundle: &Bundle, pinned: bool) -> Result<(), String> {
        // No file/directory is created before both limits pass.
        self.check_disk(bundle.archive.len() as u64 + MAX_INDEX as u64)?;
        if bundle.archive.len() as u64 > self.capacity {
            return Err("app bundle exceeds the cache capacity".into());
        }
        let mut entries = self.entries.lock().map_err(|_| "app cache lock")?;
        let _process = self.process_lock(true)?;
        self.refresh(&mut entries)?;
        self.room_locked(
            &mut entries,
            bundle.archive.len() as u64,
            Some(bundle.hash()),
        )?;
        self.check_disk(bundle.archive.len() as u64 + MAX_INDEX as u64)?;
        crate::atomic::replace(
            &self.dir.join(format!("{}.tar", bundle.hash())),
            bundle.archive(),
            0o600,
        )?;
        let was_pinned = entries.get(bundle.hash()).is_some_and(|e| e.pinned);
        entries.insert(
            bundle.hash().into(),
            CacheEntry {
                size: bundle.archive.len() as u64,
                used: now(),
                pinned: pinned || was_pinned,
            },
        );
        self.save(&entries)
    }

    pub fn get(&self, hash: &str) -> Result<Option<Bundle>, String> {
        if !self.enabled() {
            return Err("app bundle downloads and cache are disabled".into());
        }
        let hash = normalize_hash(hash)?;
        let mut entries = self.entries.lock().map_err(|_| "app cache lock")?;
        let _process = self.process_lock(false)?;
        self.refresh(&mut entries)?;
        if !entries.contains_key(&hash) {
            let path = self.dir.join(format!("{hash}.tar"));
            let Ok(meta) = std::fs::symlink_metadata(path) else {
                return Ok(None);
            };
            if !meta.is_file() || meta.len() > MAX_BUNDLE as u64 {
                return Err("app archive is not a bounded regular file".into());
            }
            self.room_locked(&mut entries, meta.len(), Some(&hash))?;
            entries.insert(
                hash.clone(),
                CacheEntry {
                    size: meta.len(),
                    used: 0,
                    pinned: false,
                },
            );
        }
        let bundle = read_regular(&self.dir.join(format!("{hash}.tar")), MAX_BUNDLE)
            .and_then(|bytes| Bundle::from_archive(bytes, Some(&hash)));
        match bundle {
            Ok(bundle) => {
                entries.get_mut(&hash).expect("known entry").used = now();
                // Reads still work at the floor; LRU metadata can wait.
                let _ = self.save(&entries);
                Ok(Some(bundle))
            }
            Err(e) => {
                let _ = std::fs::remove_file(self.dir.join(format!("{hash}.tar")));
                entries.remove(&hash);
                let _ = self.save(&entries);
                Err(e)
            }
        }
    }

    pub fn pin(&self, hash: &str, pinned: bool) -> Result<(), String> {
        let hash = normalize_hash(hash)?;
        let mut entries = self.entries.lock().map_err(|_| "app cache lock")?;
        let _process = self.process_lock(false)?;
        self.refresh(&mut entries)?;
        entries
            .get_mut(&hash)
            .ok_or("app bundle is not cached")?
            .pinned = pinned;
        self.save(&entries)
    }

    /// Refresh disk LRU and externally changed pins without reading executable
    /// archive bytes. A verified in-memory snapshot remains valid if evicted.
    fn touch(&self, hash: &str) -> Result<(), String> {
        let mut entries = self.entries.lock().map_err(|_| "app cache lock")?;
        let _process = self.process_lock(false)?;
        self.refresh(&mut entries)?;
        if let Some(entry) = entries.get_mut(hash) {
            entry.used = now();
            self.save(&entries)?;
        }
        Ok(())
    }

    pub fn import(
        &self,
        path: &Path,
        expected: Option<&str>,
        pinned: bool,
    ) -> Result<String, String> {
        self.check_disk(MAX_BUNDLE as u64 + MAX_INDEX as u64)?;
        let bundle = Bundle::from_archive(read_regular(path, MAX_BUNDLE)?, expected)?;
        self.put(&bundle, pinned)?;
        Ok(bundle.hash().into())
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .min(u64::MAX as u128) as u64
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Request {
    #[serde(rename = "bundleHash")]
    hash: String,
}

/// Node-local wallet fetcher and the separate, opt-in peer seeding handler.
pub struct Service {
    cache: Arc<Cache>,
    endpoint: Option<Endpoint>,
    peers: RwLock<Vec<EndpointAddr>>,
    downloads: Arc<tokio::sync::Semaphore>,
    peer_cursor: AtomicUsize,
    /// Two immutable, fully verified archives: at most 40 MiB. A page with
    /// many assets does not re-read/re-hash its entire archive per file.
    hot: Mutex<Vec<Arc<Bundle>>>,
}

impl Service {
    pub fn new(cache: Arc<Cache>, endpoint: Option<Endpoint>, peers: Vec<EndpointAddr>) -> Self {
        let mut seen = BTreeSet::new();
        let peers = peers
            .into_iter()
            .filter(|a| endpoint.as_ref().is_none_or(|e| a.id != e.id()) && seen.insert(a.id))
            .take(MAX_PEERS)
            .collect();
        Self {
            cache,
            endpoint,
            peers: RwLock::new(peers),
            downloads: Arc::new(tokio::sync::Semaphore::new(2)),
            peer_cursor: AtomicUsize::new(0),
            hot: Mutex::new(Vec::new()),
        }
    }

    fn remember_hot(&self, bundle: Arc<Bundle>) -> Result<(), String> {
        let mut hot = self.hot.lock().map_err(|_| "app hot cache lock")?;
        if let Some(i) = hot.iter().position(|b| b.hash() == bundle.hash()) {
            hot.remove(i);
        }
        if hot.len() == 2 {
            hot.remove(0);
        }
        hot.push(bundle);
        Ok(())
    }

    async fn cached(&self, hash: String) -> Result<Option<Arc<Bundle>>, String> {
        if !self.cache.enabled() {
            return Err("app bundle downloads and cache are disabled".into());
        }
        let hit = {
            let mut hot = self.hot.lock().map_err(|_| "app hot cache lock")?;
            hot.iter().position(|b| b.hash() == hash).map(|i| {
                let bundle = hot.remove(i);
                hot.push(bundle.clone());
                bundle
            })
        };
        if let Some(bundle) = hit {
            let cache = self.cache.clone();
            // The floor may defer LRU writes, while these already-verified
            // immutable bytes can still be read safely.
            let _ = tokio::task::spawn_blocking(move || cache.touch(&hash)).await;
            return Ok(Some(bundle));
        }
        let cache = self.cache.clone();
        let bundle = tokio::task::spawn_blocking(move || cache.get(&hash))
            .await
            .map_err(|e| e.to_string())??
            .map(Arc::new);
        if let Some(bundle) = &bundle {
            self.remember_hot(bundle.clone())?;
        }
        Ok(bundle)
    }

    pub async fn rpc(&self, hash: &str, path: &str) -> Result<Value, String> {
        let hash = normalize_hash(hash)?;
        validate_path(path)?;
        if let Some(bundle) = self.cached(hash.clone()).await? {
            return bundle.response(path);
        }
        let _permit = self
            .downloads
            .clone()
            .try_acquire_owned()
            .map_err(|_| "app download budget busy; retry shortly")?;
        self.cache
            .check_disk(MAX_BUNDLE as u64 + MAX_INDEX as u64)?;
        let bundle = tokio::time::timeout(Duration::from_secs(30), self.obtain(&hash))
            .await
            .map_err(|_| "app bundle unavailable: peer download timed out")??;
        bundle.response(path)
    }

    async fn obtain(&self, hash: &str) -> Result<Arc<Bundle>, String> {
        let endpoint = self
            .endpoint
            .as_ref()
            .ok_or("app bundle unavailable offline: no peer transport")?;
        let request =
            serde_json::to_vec(&Request { hash: hash.into() }).map_err(|e| e.to_string())?;
        let mut last = "no app content peers are known".to_string();
        let mut peers = self
            .peers
            .read()
            .map_err(|_| "app peer cache lock")?
            .clone();
        if !peers.is_empty() {
            let start = self.peer_cursor.fetch_add(8, Ordering::Relaxed) % peers.len();
            peers.rotate_left(start);
        }
        for phase in 0..2 {
            // Try every remembered ID, with four concurrent transfers and a
            // short connection budget. Successful transfers share the outer
            // 30-second deadline rather than an unrealistic 4-second cap.
            let attempts = futures::stream::iter(peers.clone())
                .map(|addr| {
                    let request = &request;
                    async move {
                        let result =
                            aether_net::app_call(endpoint, &addr, request, Duration::from_secs(30))
                                .await;
                        (addr, result)
                    }
                })
                .buffer_unordered(4);
            futures::pin_mut!(attempts);
            while let Some((addr, answer)) = attempts.next().await {
                self.cache
                    .check_disk(MAX_BUNDLE as u64 + MAX_INDEX as u64)?;
                let bytes = match answer {
                    Ok(bytes) => bytes,
                    Err(e) => {
                        last = e.to_string();
                        continue;
                    }
                };
                let expected = hash.to_string();
                let cache = self.cache.clone();
                let verified = tokio::task::spawn_blocking(move || {
                    let bundle = Bundle::from_archive(bytes, Some(&expected))?;
                    cache.put(&bundle, false)?;
                    Ok::<_, String>(bundle)
                })
                .await
                .map_err(|e| e.to_string())?;
                match verified {
                    Ok(bundle) => {
                        let bundle = Arc::new(bundle);
                        self.remember_hot(bundle.clone())?;
                        // Prefer the working source for later apps, but keep
                        // fair rotation so a stalled first source cannot hide
                        // another peer indefinitely.
                        let mut remembered =
                            self.peers.write().map_err(|_| "app peer cache lock")?;
                        if let Some(i) = remembered.iter().position(|p| p.id == addr.id) {
                            let peer = remembered.remove(i);
                            remembered.insert(0, peer);
                        }
                        self.peer_cursor.store(0, Ordering::Relaxed);
                        return Ok(bundle);
                    }
                    Err(e) => last = e,
                }
            }
            if phase == 0 {
                let mut learned = Vec::new();
                for addr in peers.iter().take(2) {
                    let answer = tokio::time::timeout(Duration::from_secs(3), async {
                        let conn = aether_net::connect_rpc(endpoint, addr, Duration::from_secs(2))
                            .await
                            .ok()?;
                        aether_net::rpc_call(&conn, "aether_walletServers", json!([]))
                            .await
                            .ok()
                    })
                    .await;
                    if let Ok(Some(Value::Array(ids))) = answer {
                        for id in ids
                            .iter()
                            .take(MAX_PEERS)
                            .filter_map(Value::as_str)
                            .filter_map(|s| s.parse::<aether_net::EndpointId>().ok())
                        {
                            if id != endpoint.id()
                                && !peers.iter().any(|p| p.id == id)
                                && !learned.iter().any(|p: &EndpointAddr| p.id == id)
                            {
                                learned.push(EndpointAddr::from(id));
                            }
                        }
                    }
                }
                if learned.is_empty() {
                    break;
                }
                let mut remembered = self.peers.write().map_err(|_| "app peer cache lock")?;
                for addr in &learned {
                    if remembered.len() < MAX_PEERS {
                        remembered.push(addr.clone());
                    }
                }
                peers = learned;
            }
        }
        Err(format!("app bundle unavailable or unverified: {last}"))
    }

    pub fn handler(self: &Arc<Self>) -> aether_net::AppsHandler {
        let service = self.clone();
        Arc::new(move |request| {
            let service = service.clone();
            Box::pin(async move {
                if !service.cache.seeding() {
                    return Err("app content seeding is disabled".into());
                }
                let request: Request =
                    serde_json::from_slice(&request).map_err(|e| format!("app request: {e}"))?;
                let hash = normalize_hash(&request.hash)?;
                let bundle = service
                    .cached(hash)
                    .await?
                    .ok_or("app bundle is not cached here")?;
                Ok(bundle.archive.clone())
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct Directory(PathBuf);
    impl Directory {
        fn new(name: &str) -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let root = Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .parent()
                .unwrap();
            let path = root.join("tmp").join(format!(
                "app-bundle-{name}-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn bundle(&self, text: &str) -> Bundle {
            let folder = self.0.join("source");
            std::fs::create_dir_all(&folder).unwrap();
            std::fs::write(folder.join("index.html"), text).unwrap();
            Bundle::from_folder(&folder).unwrap()
        }
    }
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn deterministic_bundle_identity_is_its_index_and_every_file_is_checked() {
        let directory = Directory::new("integrity");
        let bundle = directory.bundle("<h1>Try this app</h1>");
        let again = Bundle::from_folder(&directory.0.join("source")).unwrap();
        assert_eq!(bundle.archive(), again.archive());
        assert_eq!(bundle.hash(), sha256(bundle.index_bytes()));
        assert_ne!(bundle.hash(), sha256(bundle.archive()));
        assert!(Bundle::from_archive(bundle.archive().to_vec(), Some(&"00".repeat(32))).is_err());
        let mut corrupt = bundle.archive().to_vec();
        let offset = bundle.ranges["index.html"].start;
        corrupt[offset] ^= 1;
        assert!(Bundle::from_archive(corrupt, Some(bundle.hash()))
            .unwrap_err()
            .contains("mismatch"));
    }

    #[test]
    fn traversal_encoded_paths_and_non_regular_tar_entries_are_rejected() {
        let directory = Directory::new("paths");
        let bundle = directory.bundle("<p>Known bytes</p>");
        for path in [
            "../index.html",
            "/index.html",
            "a/../index.html",
            "a/./index.html",
            "a//index.html",
            "%2e%2e/index.html",
            "a\\index.html",
            "index.html/",
            "index.html?x=1",
        ] {
            assert!(validate_path(path).is_err(), "{path}");
            assert!(bundle.file(path).is_err(), "{path}");
        }
        assert!(bundle.file("unknown.js").is_err());
        let mut archive = bundle.archive().to_vec();
        archive[156] = b'2'; // symlink rather than a regular file
        assert!(Bundle::from_archive(archive, Some(bundle.hash())).is_err());
        let mut archive = bundle.archive().to_vec();
        archive[..100].fill(0);
        archive[..13].copy_from_slice(b"../index.html");
        assert!(Bundle::from_archive(archive, Some(bundle.hash())).is_err());
    }

    #[test]
    fn noncanonical_indexes_and_case_insensitive_duplicates_are_rejected() {
        let entry = |path: &str| FileEntry {
            path: path.into(),
            sha256: sha256(b"x"),
            size: 1,
        };
        let index = Index {
            format: FORMAT.into(),
            files: vec![entry("A.js"), entry("a.js")],
        };
        assert!(validate_index(&serde_json::to_vec(&index).unwrap(), None).is_err());
        let index = Index {
            format: FORMAT.into(),
            files: vec![entry("index.html")],
        };
        let pretty = serde_json::to_vec_pretty(&index).unwrap();
        assert!(validate_index(&pretty, Some(&sha256(&pretty))).is_err());
        let index = Index {
            format: FORMAT.into(),
            files: vec![entry("bundle.json")],
        };
        assert!(validate_index(&serde_json::to_vec(&index).unwrap(), None).is_err());
    }

    #[test]
    fn local_folder_symlinks_are_rejected() {
        let directory = Directory::new("links");
        directory.bundle("valid");
        std::os::unix::fs::symlink("index.html", directory.0.join("source/link.html")).unwrap();
        assert!(Bundle::from_folder(&directory.0.join("source")).is_err());
    }

    #[test]
    fn pinned_content_cannot_cross_the_disk_floor_or_capacity() {
        let directory = Directory::new("floor");
        let bundle = directory.bundle("test");
        let cache_dir = directory.0.join("cache");
        let cache = Cache::open_with_limits(&cache_dir, CACHE_BYTES, u64::MAX).unwrap();
        assert!(cache.put(&bundle, true).unwrap_err().contains("disk floor"));
        assert!(
            !cache_dir.join("apps").exists(),
            "the floor is checked before publishing or creating cache files"
        );
        let tiny = Cache::open_with_limits(&cache_dir, 1, 0).unwrap();
        assert!(tiny.put(&bundle, true).is_err());
        assert!(!cache_dir.join("apps").exists());
    }

    #[test]
    fn lru_evicts_unpinned_bundles_and_keeps_pins_across_restart() {
        let directory = Directory::new("lru");
        let a = directory.bundle("a");
        let b = directory.bundle("b");
        let c = directory.bundle("c");
        let data = directory.0.join("cache");
        let capacity = (a.archive().len() + b.archive().len()) as u64;
        let cache = Cache::open_with_limits(&data, capacity, 0).unwrap();
        cache.put(&a, true).unwrap();
        cache.put(&b, false).unwrap();
        cache.put(&c, false).unwrap();
        assert!(cache.get(a.hash()).unwrap().is_some());
        assert!(cache.get(b.hash()).unwrap().is_none());
        assert!(cache.get(c.hash()).unwrap().is_some());
        drop(cache);
        let cache = Cache::open_with_limits(&data, capacity, 0).unwrap();
        cache.pin(c.hash(), true).unwrap();
        assert!(cache.put(&b, true).unwrap_err().contains("pinned"));
        cache.pin(a.hash(), false).unwrap();
        cache.put(&b, true).unwrap();
        assert!(cache.get(a.hash()).unwrap().is_none());
    }

    #[test]
    fn corrupted_disk_content_and_cache_opt_out_never_return_bytes() {
        let directory = Directory::new("cache-integrity");
        let bundle = directory.bundle("published");
        let data = directory.0.join("cache");
        let cache = Cache::open_with_limits(&data, CACHE_BYTES, 0).unwrap();
        cache.put(&bundle, true).unwrap();
        let path = data.join(format!("apps/{}.tar", bundle.hash()));
        let mut corrupt = bundle.archive().to_vec();
        corrupt[bundle.ranges["index.html"].start] ^= 1;
        std::fs::write(&path, corrupt).unwrap();
        assert!(cache.get(bundle.hash()).is_err());
        assert!(!path.exists());
        configure(
            &data,
            Config {
                enabled: false,
                seed: true,
            },
        )
        .unwrap();
        let cache = Cache::open_with_limits(&data, CACHE_BYTES, 0).unwrap();
        assert!(!cache.seeding());
        assert!(cache.get(bundle.hash()).is_err());
        assert!(cache.put(&bundle, true).is_err());
    }

    #[test]
    fn a_running_cache_sees_a_cli_import_and_unpin() {
        let directory = Directory::new("cli-import");
        let bundle = directory.bundle("imported while running");
        let data = directory.0.join("cache");
        let running = Cache::open_with_limits(&data, CACHE_BYTES, 0).unwrap();
        let publisher = Cache::open_with_limits(&data, CACHE_BYTES, 0).unwrap();
        publisher.put(&bundle, true).unwrap();
        assert!(running.get(bundle.hash()).unwrap().is_some());
        publisher.pin(bundle.hash(), false).unwrap();
        running.get(bundle.hash()).unwrap();
        assert!(!running.entries.lock().unwrap()[bundle.hash()].pinned);
    }

    #[tokio::test]
    async fn seeding_is_opt_in_and_offline_cache_reads_verify() {
        let directory = Directory::new("seeding");
        let bundle = directory.bundle("<button>Use app</button>");
        let cache =
            Arc::new(Cache::open_with_limits(&directory.0.join("cache"), CACHE_BYTES, 0).unwrap());
        cache.put(&bundle, true).unwrap();
        let service = Arc::new(Service::new(cache, None, vec![]));
        let response = service.rpc(bundle.hash(), "index.html").await.unwrap();
        assert_eq!(
            STANDARD.decode(response["data"].as_str().unwrap()).unwrap(),
            bundle.file("index.html").unwrap()
        );
        let request = serde_json::to_vec(&Request {
            hash: bundle.hash().into(),
        })
        .unwrap();
        assert!(service.handler()(request)
            .await
            .unwrap_err()
            .contains("seeding is disabled"));
        assert!(service
            .rpc(&"00".repeat(32), "index.html")
            .await
            .unwrap_err()
            .contains("offline"));
    }

    #[tokio::test]
    async fn hot_snapshots_serve_verified_bytes_without_rereading_a_changed_archive() {
        let directory = Directory::new("hot");
        let a = directory.bundle("verified first app");
        let data = directory.0.join("cache");
        let cache = Arc::new(Cache::open_with_limits(&data, CACHE_BYTES, 0).unwrap());
        cache.put(&a, true).unwrap();
        let service = Service::new(cache.clone(), None, vec![]);
        let first = service.rpc(a.hash(), "index.html").await.unwrap();
        let archive = data.join(format!("apps/{}.tar", a.hash()));
        std::fs::write(&archive, b"changed, unverified disk bytes").unwrap();
        cache.pin(a.hash(), false).unwrap();
        let second = service.rpc(a.hash(), "index.html").await.unwrap();
        assert_eq!(
            first, second,
            "immutable verified content is served, never the changed disk bytes"
        );
        assert!(
            !cache.entries.lock().unwrap()[a.hash()].pinned,
            "hot reads preserve an external unpin"
        );
        for text in ["second app", "third app"] {
            let bundle = directory.bundle(text);
            cache.put(&bundle, false).unwrap();
            service.rpc(bundle.hash(), "index.html").await.unwrap();
        }
        let hot = service.hot.lock().unwrap();
        assert_eq!(hot.len(), 2);
        assert!(hot.iter().map(|b| b.archive().len()).sum::<usize>() <= 2 * MAX_BUNDLE);
        drop(hot);
        assert!(
            service.rpc(a.hash(), "index.html").await.is_err(),
            "evicted hot content is verified again before use"
        );
    }

    #[tokio::test]
    async fn a_known_seeder_after_eight_unreachable_peers_is_tried() {
        async fn endpoint() -> Endpoint {
            Endpoint::builder(iroh::endpoint::presets::Minimal)
                .relay_mode(iroh::RelayMode::Disabled)
                .clear_ip_transports()
                .bind_addr("127.0.0.1:0")
                .unwrap()
                .bind()
                .await
                .unwrap()
        }
        let directory = Directory::new("peer-fairness");
        let bundle = directory.bundle("<h1>Seeder nine works</h1>");
        let seed_data = directory.0.join("seed");
        configure(
            &seed_data,
            Config {
                enabled: true,
                seed: true,
            },
        )
        .unwrap();
        let cache = Arc::new(Cache::open_with_limits(&seed_data, CACHE_BYTES, 0).unwrap());
        cache.put(&bundle, true).unwrap();
        let seed_endpoint = endpoint().await;
        let seed_addr = EndpointAddr::from_parts(
            seed_endpoint.id(),
            seed_endpoint
                .bound_sockets()
                .into_iter()
                .map(aether_net::TransportAddr::Ip),
        );
        let seeder = Arc::new(Service::new(cache, Some(seed_endpoint.clone()), vec![]));
        let handler = seeder.handler();
        let slow_handler: aether_net::AppsHandler = Arc::new(move |request| {
            let handler = handler.clone();
            Box::pin(async move {
                // A legitimate transfer can outlast the connection timeout.
                // The old four-second end-to-end cap rejected these bytes.
                tokio::time::sleep(Duration::from_millis(4_200)).await;
                handler(request).await
            })
        });
        let router = aether_net::serve_with_apps(
            seed_endpoint,
            |_| async { Value::Null },
            None,
            None,
            Some(slow_handler),
        );
        let client_endpoint = endpoint().await;
        let cache =
            Arc::new(Cache::open_with_limits(&directory.0.join("wallet"), CACHE_BYTES, 0).unwrap());
        let mut peers: Vec<_> = (101..=108)
            .map(|i| EndpointAddr::from(aether_net::devnet_node_id(i)))
            .collect();
        peers.push(seed_addr);
        let client = Service::new(cache, Some(client_endpoint.clone()), peers);
        let result = tokio::time::timeout(
            Duration::from_secs(15),
            client.rpc(bundle.hash(), "index.html"),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(
            STANDARD.decode(result["data"].as_str().unwrap()).unwrap(),
            bundle.file("index.html").unwrap()
        );
        assert_eq!(client.hot.lock().unwrap().len(), 1);
        router.shutdown().await.unwrap();
        client_endpoint.close().await;
    }
}
