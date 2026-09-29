//! Engine wiring: buffered block broadcast + marshal (ordered finalized delivery,
//! backfill, disk archives) + simplex consensus.
//! Adapted from alto-chain `engine.rs` (MIT OR Apache-2.0, commonwarexyz/alto).

use crate::application::Application;
use crate::block::{Block, PublicKey};
use crate::epochs::{RotatingProvider, ScheduleEpocher};
use crate::voting::DurableVote;
use aether_light::Scheme;
use commonware_broadcast::buffered;
pub use commonware_consensus::marshal::core::Mailbox as MarshalMailboxOf;
use commonware_consensus::{
    marshal::{
        self,
        core::{Actor as MarshalActor, Mailbox as MarshalMailbox},
        resolver::handler,
        standard::{Inline, Standard},
    },
    simplex::{self, Engine as Consensus},
    types::{Epoch, ViewDelta},
};
use commonware_consensus::{Epochable as _, Viewable as _};
use commonware_cryptography::{sha256::Digest, Digestible as _};
use commonware_p2p::{Blocker, Provider, Receiver, Sender};
use commonware_parallel::Sequential;
use commonware_resolver::TargetedResolver;
use commonware_runtime::{
    buffer::paged::{page_size, CacheRef},
    spawn_cell, BufferPooler, Clock, ContextCell, Handle, Metrics, Spawner, Storage,
};
use crate::archive::{prunable_config, Buffers, FinalizedBlocks, FinalizedCerts};
pub use crate::archive::Layout;
use commonware_consensus::marshal::store::{Blocks, Certificates};
use commonware_storage::archive::{immutable, prunable, Identifier};
use commonware_utils::{NZUsize, NZU64};
use futures::future::try_join_all;
use governor::clock::Clock as GClock;
use rand::{CryptoRng, Rng};
use std::num::{NonZero, NonZeroUsize};
use std::time::Duration;
use tracing::{error, warn};

type Activity = simplex::types::Activity<Scheme, Digest>;
pub type Finalization = simplex::types::Finalization<Scheme, Digest>;
/// Marshal adapter. `Inline`, not `Deferred`: `Application::verify` (execution
/// and the FOCIL inclusion-list rule, which depends on what this node has seen)
/// must decide the notarize vote, never certification. See `crate::voting`.
pub type Marshaled<E> = Inline<E, Scheme, Application, Block, ScheduleEpocher>;
/// What consensus drives: the adapter, voting notarize only once the block is durable.
pub type Voter<E> = DurableVote<E, Marshaled<E>>;

const SYNCER_ACTIVITY_TIMEOUT_MULTIPLIER: u64 = 10;
const PRUNABLE_ITEMS_PER_SECTION: NonZero<u64> = NZU64!(4_096);
const IMMUTABLE_ITEMS_PER_SECTION: NonZero<u64> = NZU64!(262_144);
const FREEZER_TABLE_INITIAL_SIZE: u32 = 2u32.pow(14);
const FREEZER_TABLE_RESIZE_FREQUENCY: u8 = 4;
const FREEZER_TABLE_RESIZE_CHUNK_SIZE: u32 = 2u32.pow(16);
const FREEZER_JOURNAL_TARGET_SIZE: u64 = 1024 * 1024 * 1024;
const FREEZER_JOURNAL_COMPRESSION: Option<u8> = Some(3);
const REPLAY_BUFFER: NonZero<usize> = NZUsize!(8 * 1024 * 1024);
const WRITE_BUFFER: NonZero<usize> = NZUsize!(1024 * 1024);
const PAGE_CACHE_PAGE_SIZE: NonZero<u16> = page_size(4_096);
const PAGE_CACHE_CAPACITY: NonZero<usize> = NZUsize!(8_192);
const MAX_REPAIR: NonZero<usize> = NZUsize!(20);
const MAX_PENDING_ACKS: NonZero<usize> = NZUsize!(16);
pub use aether_light::MAX_BLOCK_BYTES;

pub struct Config<B: Blocker<PublicKey = PublicKey>, P: Provider<PublicKey = PublicKey>> {
    pub blocker: B,
    pub provider: P,
    pub partition_prefix: String,
    pub me: PublicKey,
    pub scheme: Scheme,
    /// Committee identity (verifies certificates of every epoch).
    pub identity: aether_light::Identity,
    /// The chain's consensus group (0 today): certificates verify under its
    /// namespace.
    pub group: u16,
    /// Epoch boundaries; the node runs the latest epoch.
    pub epocher: ScheduleEpocher,
    /// Digest consensus starts from in the current epoch: the chain genesis in
    /// epoch 0, else the last block of the previous epoch.
    pub epoch_floor: Option<Digest>,
    pub genesis: Block,
    /// A voting node that joins with no validator history starts from the last
    /// block it verified as a follower (and its finalization) instead of genesis.
    pub anchor: Option<(Block, Finalization)>,
    /// Marshal's archive layout (`Layout::Prunable` on pruning nodes, roadmap B4).
    pub layout: Layout,
    pub application: Application,
    pub mailbox_size: usize,
    pub leader_timeout: Duration,
    pub certification_timeout: Duration,
    pub nullify_retry: Duration,
    pub fetch_timeout: Duration,
    pub activity_timeout: ViewDelta,
    pub skip_timeout: Duration,
}

#[allow(clippy::type_complexity)]
pub struct Engine<E, B, P>
where
    E: BufferPooler + Clock + GClock + Rng + CryptoRng + Spawner + Storage + Metrics,
    B: Blocker<PublicKey = PublicKey>,
    P: Provider<PublicKey = PublicKey>,
{
    context: ContextCell<E>,
    buffer: buffered::Engine<E, PublicKey, Block, P>,
    buffer_mailbox: buffered::Mailbox<PublicKey, Block>,
    marshal: MarshalActor<E, Standard<Block>, RotatingProvider, FinalizedCerts<E>, FinalizedBlocks<E>, ScheduleEpocher, Sequential>,
    marshaled: Marshaled<E>,
    /// Handle for reading finalized blocks and certificates (served over RPC).
    pub mailbox: MarshalMailbox<Scheme, Standard<Block>>,
    consensus: Consensus<E, Scheme, aether_light::Elector, B, Digest, Voter<E>, Voter<E>, MarshalMailbox<Scheme, Standard<Block>>, Sequential>,
}

fn archive_cfg<C>(prefix: &str, name: &str, page_cache: CacheRef, codec_config: C) -> immutable::Config<C> {
    immutable::Config {
        metadata_partition: format!("{prefix}-{name}-metadata"),
        freezer_table_partition: format!("{prefix}-{name}-freezer-table"),
        freezer_table_initial_size: FREEZER_TABLE_INITIAL_SIZE,
        freezer_table_resize_frequency: FREEZER_TABLE_RESIZE_FREQUENCY,
        freezer_table_resize_chunk_size: FREEZER_TABLE_RESIZE_CHUNK_SIZE,
        freezer_key_partition: format!("{prefix}-{name}-freezer-key-journal"),
        freezer_key_page_cache: page_cache,
        freezer_key_write_buffer: WRITE_BUFFER,
        freezer_value_partition: format!("{prefix}-{name}-freezer-value-journal"),
        freezer_value_write_buffer: WRITE_BUFFER,
        freezer_value_target_size: FREEZER_JOURNAL_TARGET_SIZE,
        freezer_value_compression: FREEZER_JOURNAL_COMPRESSION,
        ordinal_partition: format!("{prefix}-{name}-ordinal"),
        ordinal_write_buffer: WRITE_BUFFER,
        items_per_section: IMMUTABLE_ITEMS_PER_SECTION,
        codec_config,
        replay_buffer: REPLAY_BUFFER,
    }
}

/// The finalization `AETHER_RECOVER_CONSENSUS=<view>@<height>` names, or Err
/// with why that value cannot start this node. The runbook
/// (docs/ops/consensus-recovery.md) is enforced in this order:
///
/// 1. `<view>@<height>` parses (a bare `<view>` means the last stored
///    finalization, so it names a height that exists).
/// 2. A finalization is stored at `<height>`, and it is at `<view>`.
/// 3. It is for the current committee epoch; an earlier epoch is ignored
///    (Ok(None)) rather than refused, so a value left set after an epoch change
///    never bricks the node.
/// 4. A first recovery (no vote journal for the view exists yet) must name the
///    last stored finalization: starting lower forks the node away from
///    finalizations other validators keep. Restarts after the recovery find the
///    journal and start wherever the value says, as before.
pub async fn recover<E: BufferPooler + Clock + Metrics + Storage>(
    context: &E,
    finalizations: &FinalizedCerts<E>,
    prefix: &str,
    epoch: Epoch,
    value: &str,
) -> Result<Option<Finalization>, String> {
    let last = Certificates::last_index(finalizations).map(|h| h.get());
    let (view, height) = match value.split_once('@') {
        Some((a, b)) => (a.parse::<u64>().ok(), b.parse::<u64>().ok()),
        None => (value.parse::<u64>().ok(), last),
    };
    let (Some(view), Some(height)) = (view, height) else {
        return Err(format!("{value}: expected <view>@<height>"));
    };
    let Some(f) = Certificates::get(finalizations, Identifier::<Digest>::Index(height)).await.ok().flatten() else {
        return Err(format!("{value}, but no finalization is stored at height {height}"));
    };
    if f.view().get() != view {
        return Err(format!("{value}, but the finalization at height {height} is at view {}: refusing", f.view().get()));
    }
    if f.epoch() != epoch {
        tracing::info!(view, "AETHER_RECOVER_CONSENSUS is for an earlier epoch; ignored");
        return Ok(None);
    }
    let partition = format!("{prefix}-consensus-r{view}");
    let journal_exists = partition_exists(context, &partition)
        .await
        .map_err(|e| format!("{value}, but could not look for the {partition} vote journal: {e}"))?;
    if Some(height) != last && !journal_exists {
        return Err(format!(
            "{value}, but the first recovery (the {partition} vote journal does not exist yet) must name the last stored finalization, height {}",
            last.unwrap_or_default()
        ));
    }
    tracing::warn!(view, height, "recovering consensus from a stored finalization with a vote journal of its own");
    Ok(Some(f))
}

/// Whether a storage partition exists (any blob was ever created in it).
async fn partition_exists<E: Storage>(context: &E, partition: &str) -> Result<bool, commonware_runtime::Error> {
    match context.scan(partition).await {
        Ok(_) => Ok(true),
        Err(commonware_runtime::Error::PartitionMissing(_)) => Ok(false),
        Err(e) => Err(e),
    }
}

impl<E, B, P> Engine<E, B, P>
where
    E: BufferPooler + Clock + GClock + Rng + CryptoRng + Spawner + Storage + Metrics,
    B: Blocker<PublicKey = PublicKey>,
    P: Provider<PublicKey = PublicKey>,
{
    pub async fn new(context: E, cfg: Config<B, P>) -> Self {
        let mailbox_size = NonZeroUsize::new(cfg.mailbox_size).expect("mailbox size must be non-zero");
        let (buffer, buffer_mailbox) = buffered::Engine::new(
            context.child("buffer"),
            buffered::Config {
                public_key: cfg.me,
                mailbox_size,
                deque_size: 10,
                priority: true,
                codec_config: Block::codec_config(MAX_BLOCK_BYTES),
                peer_provider: cfg.provider,
            },
        );
        let page_cache = CacheRef::from_pooler(&context, PAGE_CACHE_PAGE_SIZE, PAGE_CACHE_CAPACITY);
        let prefix = cfg.partition_prefix.clone();
        let (finalizations, mut blocks) = match cfg.layout {
            Layout::Immutable => (
                FinalizedCerts::Immutable(
                    immutable::Archive::init(
                        context.child("finalizations_by_height"),
                        // Threshold certificates are fixed-size; their codec config is `()`.
                        archive_cfg(&prefix, "finalizations", page_cache.clone(), ()),
                    )
                    .await
                    .expect("finalizations archive"),
                ),
                FinalizedBlocks::Immutable(
                    immutable::Archive::init(
                        context.child("finalized_blocks"),
                        archive_cfg(&prefix, "blocks", page_cache.clone(), Block::codec_config(MAX_BLOCK_BYTES)),
                    )
                    .await
                    .expect("blocks archive"),
                ),
            ),
            // Pruning (roadmap B4): one section per era, dropped once sealed and old.
            Layout::Prunable => {
                let buffers = Buffers { write: WRITE_BUFFER, replay: REPLAY_BUFFER };
                (
                    FinalizedCerts::Prunable(
                        prunable::Archive::init(
                            context.child("finalizations_by_height"),
                            prunable_config(&prefix, "finalizations", page_cache.clone(), (), buffers),
                        )
                        .await
                        .expect("finalizations archive"),
                    ),
                    FinalizedBlocks::Prunable(
                        prunable::Archive::init(
                            context.child("finalized_blocks"),
                            prunable_config(&prefix, "blocks", page_cache.clone(), Block::codec_config(MAX_BLOCK_BYTES), buffers),
                        )
                        .await
                        .expect("blocks archive"),
                    ),
                )
            }
        };
        tracing::info!(layout = ?cfg.layout, "finalized block archive");
        // Fresh archives + an anchor: store the anchor block so marshal installs
        // the floor locally (nobody in a brand-new voting set has older blocks).
        let start = match cfg.anchor {
            Some((block, finalization)) if blocks.last_index().is_none() => {
                tracing::info!(height = block.height.get(), "starting from the verified anchor block");
                blocks = Blocks::put(blocks, block).await.expect("store anchor block");
                blocks = Blocks::sync(blocks).await.expect("sync anchor block");
                marshal::Start::Floor(finalization)
            }
            _ => marshal::Start::Genesis(cfg.genesis.clone()),
        };

        // Re-execute finalized blocks newer than the durable state checkpoint, so a
        // restarted validator resumes exactly where marshal's delivery resumes.
        let restored = cfg.application.finalized_height();
        let mut replayed = 0u64;
        if let Some(last) = blocks.last_index() {
            for h in restored + 1..=last.get() {
                match Blocks::get(&blocks, Identifier::Index(h)).await {
                    Ok(Some(block)) => {
                        if let Err(e) = cfg.application.replay(&block) {
                            warn!(height = h, ?e, "replay stopped");
                            break;
                        }
                        replayed = h;
                    }
                    _ => break,
                }
            }
        }
        tracing::info!(checkpoint = restored, replayed_to = replayed.max(restored), "restored finalized state");

        // Recovery (docs/design/13-roadmap.md F-P0): AETHER_RECOVER_CONSENSUS=<view>@<height>
        // starts simplex from the finalization stored at <height>, which must be at <view>,
        // with a vote journal of its own. Every validator restarts with the same value and
        // keeps it set: later restarts reuse the same floor and journal, so nobody votes
        // twice in a view. Once the committee moves to a new epoch the value is ignored.
        // Used when a view was notarized but its block reached no store (the chain then
        // cannot extend it); votes above that finalization are dropped, and nothing
        // finalized changes. A bare <view> means the last stored finalization.
        let recovered = match std::env::var("AETHER_RECOVER_CONSENSUS").ok() {
            Some(v) => match recover(&context, &finalizations, &prefix, cfg.epocher.current(), &v).await {
                Ok(f) => f,
                Err(e) => panic!("AETHER_RECOVER_CONSENSUS {e}"),
            },
            None => None,
        };

        let scheme = cfg.scheme;
        let epocher = cfg.epocher;
        let epoch = epocher.current();
        let floor_digest = cfg.epoch_floor.unwrap_or_else(|| cfg.genesis.digest());
        let (marshal, marshal_mailbox, _) = MarshalActor::init(
            context.child("marshal"),
            finalizations,
            blocks,
            marshal::Config {
                provider: RotatingProvider::new(epoch, scheme.clone(), cfg.identity, cfg.group),
                epocher: epocher.clone(),
                partition_prefix: prefix.clone(),
                mailbox_size,
                view_retention: ViewDelta::new(cfg.activity_timeout.get().saturating_mul(SYNCER_ACTIVITY_TIMEOUT_MULTIPLIER)),
                start,
                prunable_items_per_section: PRUNABLE_ITEMS_PER_SECTION,
                replay_buffer: REPLAY_BUFFER,
                key_write_buffer: WRITE_BUFFER,
                value_write_buffer: WRITE_BUFFER,
                block_codec_config: Block::codec_config(MAX_BLOCK_BYTES),
                max_repair: MAX_REPAIR,
                max_pending_acks: MAX_PENDING_ACKS,
                page_cache: page_cache.clone(),
                strategy: Sequential,
            },
        )
        .await;

        let marshaled = Marshaled::<E>::new(context.child("marshaled"), cfg.application, marshal_mailbox.clone(), epocher);
        let voter = DurableVote::new(context.child("voter"), marshaled.clone(), marshal_mailbox.clone());
        let consensus = Consensus::new(
            context.child("consensus"),
            simplex::Config {
                epoch,
                scheme,
                automaton: voter.clone(),
                relay: voter,
                reporter: marshal_mailbox.clone(),
                track_historical_votes: false,
                // One vote journal per epoch: a new committee never replays the old one's votes.
                partition: match &recovered {
                    Some(f) => format!("{prefix}-consensus-r{}", f.view().get()),
                    None if epoch.get() == 0 => format!("{prefix}-consensus"),
                    None => format!("{prefix}-consensus-e{}", epoch.get()),
                },
                mailbox_size,
                floor: match recovered {
                    Some(f) => simplex::Floor::Finalized(f),
                    None => simplex::Floor::Genesis(floor_digest),
                },
                leader_timeout: cfg.leader_timeout,
                certification_timeout: cfg.certification_timeout,
                timeout_retry: cfg.nullify_retry,
                fetch_timeout: cfg.fetch_timeout,
                view_retention: cfg.activity_timeout,
                skip: simplex::SkipPolicy::Enabled { timeout: cfg.skip_timeout, budget: simplex::SkipBudget::Participants },
                forward: simplex::ForwardPolicy::Disabled,
                replay_buffer: REPLAY_BUFFER,
                write_buffer: WRITE_BUFFER,
                blocker: cfg.blocker,
                page_cache,
                elector: aether_light::ELECTOR,
                strategy: Sequential,
            },
        );
        let _ = std::marker::PhantomData::<Activity>;
        Self { context: ContextCell::new(context), buffer, buffer_mailbox, marshal, marshaled, mailbox: marshal_mailbox, consensus }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn start(
        mut self,
        pending: (impl Sender<PublicKey = PublicKey>, impl Receiver<PublicKey = PublicKey>),
        recovered: (impl Sender<PublicKey = PublicKey>, impl Receiver<PublicKey = PublicKey>),
        resolver: (impl Sender<PublicKey = PublicKey>, impl Receiver<PublicKey = PublicKey>),
        broadcast: (impl Sender<PublicKey = PublicKey>, impl Receiver<PublicKey = PublicKey>),
        marshal: (handler::Receiver<Digest>, impl TargetedResolver<Key = handler::Key<Digest>, Subscriber = handler::Annotation, PublicKey = PublicKey>),
    ) -> Handle<()> {
        spawn_cell!(self.context, self.run(pending, recovered, resolver, broadcast, marshal))
    }

    #[allow(clippy::too_many_arguments)]
    async fn run(
        self,
        pending: (impl Sender<PublicKey = PublicKey>, impl Receiver<PublicKey = PublicKey>),
        recovered: (impl Sender<PublicKey = PublicKey>, impl Receiver<PublicKey = PublicKey>),
        resolver: (impl Sender<PublicKey = PublicKey>, impl Receiver<PublicKey = PublicKey>),
        broadcast: (impl Sender<PublicKey = PublicKey>, impl Receiver<PublicKey = PublicKey>),
        marshal: (handler::Receiver<Digest>, impl TargetedResolver<Key = handler::Key<Digest>, Subscriber = handler::Annotation, PublicKey = PublicKey>),
    ) {
        let buffer_handle = self.buffer.start(broadcast);
        let marshal_handle = self.marshal.start(self.marshaled, self.buffer_mailbox, marshal);
        let consensus_handle = self.consensus.start(pending, recovered, resolver);
        if let Err(e) = try_join_all(vec![buffer_handle, marshal_handle, consensus_handle]).await {
            error!(?e, "engine failed");
        } else {
            warn!("engine stopped");
        }
    }
}
