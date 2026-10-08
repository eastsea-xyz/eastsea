//! Marshal's finalized-block and certificate archives (roadmap B4).
//!
//! An archive node (and every node of a network without era files, such as the
//! 7780 testnet) keeps marshal's `immutable` archives: everything forever, in
//! the partitions it always used. A pruning node keeps them in `prunable`
//! archives with one section per era (8192 heights), so marshal's `prune`
//! drops whole eras once they are sealed into era files and older than the
//! retention window. The two layouts live in different partitions: switching
//! an existing node's layout would orphan its archive, so a node that already
//! has an immutable archive keeps it (see `Layout::choose`).
//!
//! Memory: a prunable archive indexes each kept item in memory (~40 B by key,
//! ~16 B by height); 30 days of 1 s blocks in both archives is ~2 × 2.6M items.
//! Keys of pruned items are dropped lazily (when a new key collides with one),
//! so a long-running pruning node carries up to ~40 B per pruned block until it
//! restarts.

use crate::block::Block;
use crate::engine::Finalization;
use commonware_consensus::marshal::store::{Blocks, Certificates};
use commonware_consensus::types::Height;
use commonware_cryptography::sha256::Digest;
use commonware_runtime::{buffer::paged::CacheRef, BufferPooler, Clock, Handle, Metrics, Storage};
use commonware_storage::archive::{self, immutable, prunable, Identifier};
use commonware_storage::translator::EightCap;
use std::num::{NonZeroU64, NonZeroUsize};

/// Which archives a node's marshal uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    /// Keep every finalized block and certificate (the layout nodes always had).
    Immutable,
    /// Keep them in era-sized sections that can be pruned.
    Prunable,
}

impl Layout {
    /// Prunable when the node prunes, unless `data` already holds an immutable
    /// archive under `prefix` (an existing node keeps its layout and data).
    pub fn choose(prune: bool, data: &std::path::Path, prefix: &str) -> Layout {
        let existing = data.join(format!("{prefix}-blocks-metadata"));
        if prune && !existing.exists() {
            Layout::Prunable
        } else {
            Layout::Immutable
        }
    }
}

/// Partition names of the prunable layout (never ending in `-blocks-metadata`,
/// which `partition_prefix` looks for).
pub fn prunable_config<C>(prefix: &str, name: &str, page_cache: CacheRef, codec_config: C, buffers: Buffers) -> prunable::Config<EightCap, C> {
    prunable::Config {
        translator: EightCap,
        metadata_partition: format!("{prefix}-recent-{name}-marks"),
        key_partition: format!("{prefix}-recent-{name}-keys"),
        key_page_cache: page_cache,
        value_partition: format!("{prefix}-recent-{name}-values"),
        compression: Some(3),
        codec_config,
        items_per_section: NonZeroU64::new(aether_state::mmr::ERA_LEN).expect("era length"),
        key_write_buffer: buffers.write,
        value_write_buffer: buffers.write,
        replay_buffer: buffers.replay,
    }
}

#[derive(Clone, Copy)]
pub struct Buffers {
    pub write: NonZeroUsize,
    pub replay: NonZeroUsize,
}

/// Finalized blocks in either layout.
pub enum FinalizedBlocks<E: BufferPooler + Storage + Metrics + Clock> {
    Immutable(immutable::Archive<E, Digest, Block>),
    Prunable(prunable::Archive<EightCap, E, Digest, Block>),
}

/// Finalization certificates in either layout.
pub enum FinalizedCerts<E: BufferPooler + Storage + Metrics + Clock> {
    Immutable(immutable::Archive<E, Digest, Finalization>),
    Prunable(prunable::Archive<EightCap, E, Digest, Finalization>),
}

impl<E: BufferPooler + Storage + Metrics + Clock> Blocks for FinalizedBlocks<E> {
    type Block = Block;
    type Error = archive::Error;

    async fn put(self, block: Block) -> Result<Self, Self::Error> {
        Ok(match self {
            Self::Immutable(a) => Self::Immutable(Blocks::put(a, block).await?),
            Self::Prunable(a) => Self::Prunable(Blocks::put(a, block).await?),
        })
    }

    async fn sync(self) -> Result<Self, Self::Error> {
        Ok(match self {
            Self::Immutable(a) => Self::Immutable(Blocks::sync(a).await?),
            Self::Prunable(a) => Self::Prunable(Blocks::sync(a).await?),
        })
    }

    async fn start_sync(self) -> Result<(Self, Handle<()>), Self::Error> {
        Ok(match self {
            Self::Immutable(a) => {
                let (a, h) = Blocks::start_sync(a).await?;
                (Self::Immutable(a), h)
            }
            Self::Prunable(a) => {
                let (a, h) = Blocks::start_sync(a).await?;
                (Self::Prunable(a), h)
            }
        })
    }

    async fn get(&self, id: Identifier<'_, Digest>) -> Result<Option<Block>, Self::Error> {
        match self {
            Self::Immutable(a) => Blocks::get(a, id).await,
            Self::Prunable(a) => Blocks::get(a, id).await,
        }
    }

    async fn prune(self, min: Height) -> Result<Self, Self::Error> {
        Ok(match self {
            Self::Immutable(a) => Self::Immutable(a),
            Self::Prunable(a) => Self::Prunable(Blocks::prune(a, min).await?),
        })
    }

    fn missing_items(&self, start: Height, max: usize) -> Vec<Height> {
        match self {
            Self::Immutable(a) => Blocks::missing_items(a, start, max),
            Self::Prunable(a) => Blocks::missing_items(a, start, max),
        }
    }

    fn next_gap(&self, value: Height) -> (Option<Height>, Option<Height>) {
        match self {
            Self::Immutable(a) => Blocks::next_gap(a, value),
            Self::Prunable(a) => Blocks::next_gap(a, value),
        }
    }

    fn last_index(&self) -> Option<Height> {
        match self {
            Self::Immutable(a) => Blocks::last_index(a),
            Self::Prunable(a) => Blocks::last_index(a),
        }
    }
}

impl<E: BufferPooler + Storage + Metrics + Clock> Certificates for FinalizedCerts<E> {
    type BlockDigest = Digest;
    type Commitment = Digest;
    type Scheme = crate::key_binding::signing::Scheme;
    type Error = archive::Error;

    async fn put(self, height: Height, digest: Digest, finalization: Finalization) -> Result<Self, Self::Error> {
        Ok(match self {
            Self::Immutable(a) => Self::Immutable(Certificates::put(a, height, digest, finalization).await?),
            Self::Prunable(a) => Self::Prunable(Certificates::put(a, height, digest, finalization).await?),
        })
    }

    async fn sync(self) -> Result<Self, Self::Error> {
        Ok(match self {
            Self::Immutable(a) => Self::Immutable(Certificates::sync(a).await?),
            Self::Prunable(a) => Self::Prunable(Certificates::sync(a).await?),
        })
    }

    async fn start_sync(self) -> Result<(Self, Handle<()>), Self::Error> {
        Ok(match self {
            Self::Immutable(a) => {
                let (a, h) = Certificates::start_sync(a).await?;
                (Self::Immutable(a), h)
            }
            Self::Prunable(a) => {
                let (a, h) = Certificates::start_sync(a).await?;
                (Self::Prunable(a), h)
            }
        })
    }

    async fn get(&self, id: Identifier<'_, Digest>) -> Result<Option<Finalization>, Self::Error> {
        match self {
            Self::Immutable(a) => Certificates::get(a, id).await,
            Self::Prunable(a) => Certificates::get(a, id).await,
        }
    }

    async fn has(&self, height: Height) -> Result<bool, Self::Error> {
        match self {
            Self::Immutable(a) => Certificates::has(a, height).await,
            Self::Prunable(a) => Certificates::has(a, height).await,
        }
    }

    async fn prune(self, min: Height) -> Result<Self, Self::Error> {
        Ok(match self {
            Self::Immutable(a) => Self::Immutable(a),
            Self::Prunable(a) => Self::Prunable(Certificates::prune(a, min).await?),
        })
    }

    fn last_index(&self) -> Option<Height> {
        match self {
            Self::Immutable(a) => Certificates::last_index(a),
            Self::Prunable(a) => Certificates::last_index(a),
        }
    }

    fn ranges_from(&self, from: Height) -> impl Iterator<Item = (Height, Height)> {
        let v: Vec<(Height, Height)> = match self {
            Self::Immutable(a) => Certificates::ranges_from(a, from).collect(),
            Self::Prunable(a) => Certificates::ranges_from(a, from).collect(),
        };
        v.into_iter()
    }
}
