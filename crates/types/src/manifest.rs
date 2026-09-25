//! History / snapshot distribution manifest (docs/design/02-types.md).

use alloc::string::String;
use alloc::vec::Vec;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManifestKind {
    HistoryChunk,
    StateSnapshot,
    Release,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Mirror {
    Https { url: String },
    Iroh { ticket: String },
    Torrent { magnet: String, webseeds: Vec<String> },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub version: u32,
    pub chain_id: u64,
    pub kind: ManifestKind,
    pub range: Option<(u64, u64)>,
    /// BLAKE3 of the content, hex.
    pub blake3: String,
    pub size: u64,
    pub mirrors: Vec<Mirror>,
    /// Ed25519 signature over the manifest with `signature` emptied.
    pub signature: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;
    use alloc::vec;

    #[test]
    fn json_shape_matches_design() {
        let m = Manifest {
            version: 1,
            chain_id: 8453001,
            kind: ManifestKind::HistoryChunk,
            range: Some((100000, 100999)),
            blake3: "ab".to_string(),
            size: 1,
            mirrors: vec![Mirror::Https { url: "https://x".to_string() }],
            signature: "".to_string(),
        };
        let j = serde_json::to_string(&m).unwrap();
        assert!(j.contains(r#""kind":"history_chunk""#));
        assert!(j.contains(r#""type":"https""#));
        assert_eq!(serde_json::from_str::<Manifest>(&j).unwrap(), m);
    }
}
