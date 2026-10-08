//! Engine wiring: buffered block broadcast + marshal (ordered finalized delivery,
//! backfill, disk archives) + simplex consensus.
//! Adapted from alto-chain `engine.rs` (MIT OR Apache-2.0, commonwarexyz/alto).

use crate::application::Application;
use crate::block::{Block, PublicKey};
use crate::epochs::{RotatingProvider, ScheduleEpocher};
use crate::voting::DurableVote;
use crate::key_binding::signing::{Elector, Scheme, ELECTOR};
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
use commonware_consensus::types::Epocher as _;
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
/// The vote journal's own write buffer. Simplex keeps one section file per view
/// and one write buffer per open section; a view holds a few votes (KBs). The
/// journal is only pruned below the last finalization minus view retention, so
/// while the chain stalls it gathers one section per burned view (2026-09-29:
/// 190 sections) and every one of them carried a full WRITE_BUFFER of RAM.
/// 64 KiB still batches thousands of votes per flush.
const VOTE_WRITE_BUFFER: NonZero<usize> = NZUsize!(64 * 1024);
/// A vote journal past this many section files is worth a warning: the journal
/// opens every section at startup (one file descriptor each), and a validator
/// holds ~70 more, so a few hundred sections exhausts launchd's 256-fd default
/// exactly as on 2026-09-29 ("Too many open files", crash-looped).
const VOTE_JOURNAL_WARN_SECTIONS: usize = 128;
/// The divisor turning the soft open-file limit into the share of it at which
/// the journal count is worth a warning (2026-09-29 red-team 5): raising the
/// limit bounds nothing by itself — while the chain does not finalize, every
/// burned view leaves a section file behind, and every section is one
/// descriptor at the next startup. Half leaves room for the ~70 files a
/// validator holds besides them, and time to raise the limit again.
const VOTE_JOURNAL_LIMIT_DIVISOR: u64 = 2;

/// The section count at which to warn about the vote journal: half the soft
/// open-file limit, never below the plain "many sections" bar — launchd's
/// 256 warns where the bar already is, a raised 65,536 only as the journal
/// really approaches it (and an unlimited process never on fd count).
fn vote_journal_warn_at(soft_limit: u64) -> usize {
    (soft_limit / VOTE_JOURNAL_LIMIT_DIVISOR).max(VOTE_JOURNAL_WARN_SECTIONS as u64) as usize
}

/// This process's soft open-file limit (RLIMIT_INFINITY reads as u64::MAX; 0
/// when it cannot be read, which leaves the fixed bar in charge).
#[cfg(unix)]
fn soft_nofile() -> u64 {
    let mut lim: libc::rlimit = unsafe { std::mem::zeroed() };
    (unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut lim) } == 0).then_some(lim.rlim_cur).unwrap_or(0)
}

#[cfg(not(unix))]
fn soft_nofile() -> u64 {
    0
}
const PAGE_CACHE_PAGE_SIZE: NonZero<u16> = page_size(4_096);
const PAGE_CACHE_CAPACITY: NonZero<usize> = NZUsize!(8_192);
const MAX_REPAIR: NonZero<usize> = NZUsize!(20);
const MAX_PENDING_ACKS: NonZero<usize> = NZUsize!(16);
pub use aether_light::MAX_BLOCK_BYTES;

pub struct Config<B: Blocker<PublicKey = PublicKey>, P: Provider<PublicKey = PublicKey>> {
    pub blocker: B,
    pub provider: P,
    pub partition_prefix: String,
    /// Durable evidence that this Mac entered an epoch's voting engine. If
    /// its journal later disappears, even before any block finalized, it may
    /// have signed and must not start a fresh journal with the same key.
    pub journal_dir: Option<std::path::PathBuf>,
    pub me: PublicKey,
    pub scheme: aether_light::Scheme,
    /// The binding of a persisted validator key; simulations and devnet have none.
    pub key_binding: Option<crate::key_binding::Guard>,
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
    consensus: Consensus<E, Scheme, Elector, B, Digest, Voter<E>, Voter<E>, MarshalMailbox<Scheme, Standard<Block>>, Sequential>,
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
/// 4. The `-consensus-r<view>` vote journal already exists: the value may only
///    restart a past recovery, whose journal replays the votes it cast. A first
///    recovery — no such journal — always refuses (audit 4, A4-3): the last
///    finalized view is not evidence of the highest view this Mac may have
///    signed, so a fresh journal under the floor could sign a view twice with
///    the same committee share. The safe path is no override at all: follow
///    until the next committee round starts a fresh share and journal.
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
    if !journal_exists {
        return Err(format!(
            "{value}, but the {partition} vote journal does not exist: the last finalization is not evidence of the highest view this Mac may have signed, and a fresh journal there could sign twice with this committee share. Leave AETHER_RECOVER_CONSENSUS unset and restart: this Mac follows and serves until the next committee round starts a fresh share and journal"
        ));
    }
    tracing::warn!(view, height, "restarting consensus from a stored finalization with its own vote journal");
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

/// The process exits with this code when its vote journal cannot be trusted
/// (red team #4): the archive shows this Mac already delivered finalizations
/// in the epoch, but the journal holding the votes it cast in that epoch is
/// gone. Voting must not resume on an empty journal — that is how a key
/// signs twice — so the supervisor follows instead, until the next
/// committee round brings a fresh share and journal.
pub const EXIT_JOURNAL: i32 = 8;

/// The restart gate's verdict on the vote journal: the state database, the
/// block archive and the vote journal are diagnosed apart (red team #4), and
/// only a journal this Mac's own history explains may take more votes.
///
/// - `journal_has_votes`: the current epoch's journal partition holds votes —
///   a normal restart, whatever the archive says. A restarted
///   `AETHER_RECOVER_CONSENSUS` recovery passes the same way: the journal its
///   past recovery created holds the votes it cast. There is no override
///   bypass (audit 4, A4-3) — a journal-less recovery never reaches the gate,
///   `recover` refuses it first;
/// - `delivered`: the last height this Mac delivered as a validator. `None`
///   (a joining member's fresh archive) or below `epoch_start` (the epoch
///   began after its last delivery — including every earlier epoch) means it
///   cannot have voted in this epoch, so an absent journal is expected;
/// - anything else — deliveries inside the epoch without the journal they
///   were voted into — is untrusted, as is a journal that cannot even be
///   looked at (`lookup_failed`).
fn journal_gate(
    journal_has_votes: bool,
    delivered: Option<u64>,
    epoch_start: u64,
    lookup_failed: bool,
    previously_started: bool,
) -> Result<(), &'static str> {
    if lookup_failed {
        return Err("the vote journal cannot be examined");
    }
    if journal_has_votes {
        return Ok(());
    }
    if previously_started {
        return Err("this Mac entered voting in this epoch, but its vote journal is gone");
    }
    match delivered {
        Some(h) if h >= epoch_start => Err("finalizations from this epoch are stored, but this epoch's vote journal holds no votes"),
        _ => Ok(()),
    }
}

/// How many blobs a partition holds (0 when it does not exist yet).
async fn partition_blobs<E: Storage>(context: &E, partition: &str) -> Result<usize, commonware_runtime::Error> {
    match context.scan(partition).await {
        Ok(names) => Ok(names.len()),
        Err(commonware_runtime::Error::PartitionMissing(_)) => Ok(0),
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
        let scheme = Scheme::new(cfg.scheme, cfg.key_binding);
        scheme.check_binding();
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

        // Recovery (docs/design/13-roadmap.md F-P0, audit 4 A4-3):
        // AETHER_RECOVER_CONSENSUS=<view>@<height> only restarts a past
        // recovery — the `-consensus-r<view>` journal it created replays the
        // votes cast above that floor, so nobody votes twice in a view. A
        // first recovery (no such journal) is refused in `recover`: the last
        // finalized view is not evidence of the highest view this Mac may
        // have signed, and a fresh journal under it could double-vote with
        // the same committee share; the safe alternative is following until
        // the next committee round. Once the committee moves to a new epoch
        // the value is ignored. A bare <view> means the last stored
        // finalization.
        let recovered = match std::env::var("AETHER_RECOVER_CONSENSUS").ok() {
            Some(v) => match recover(&context, &finalizations, &prefix, cfg.epocher.current(), &v).await {
                Ok(f) => f,
                Err(e) => panic!("AETHER_RECOVER_CONSENSUS {e}"),
            },
            None => None,
        };

        let epocher = cfg.epocher;
        let epoch = epocher.current();
        let floor_digest = cfg.epoch_floor.unwrap_or_else(|| cfg.genesis.digest());
        // The vote journal keeps one section file per view and opens every
        // section at startup. Say how many we are about to open: a stalled
        // chain piles up sections (they are pruned only below the last
        // finalization), and on 2026-09-29 that is what hit the fd limit.
        let vote_partition = match &recovered {
            Some(f) => format!("{prefix}-consensus-r{}", f.view().get()),
            None if epoch.get() == 0 => format!("{prefix}-consensus"),
            None => format!("{prefix}-consensus-e{}", epoch.get()),
        };
        let journal_sections = partition_blobs(&context, &vote_partition).await;
        let marker = cfg.journal_dir.as_ref().map(|d| d.join(format!("vote-epoch-{}.seen", epoch.get())));
        let previously_started = marker.as_ref().is_some_and(|p| p.exists());
        // The restart gate (red team #4): before consensus casts one more
        // vote, the votes this Mac already cast in this epoch must still be
        // accounted for. A missing journal with deliveries inside the epoch
        // means the journal was lost — never voted over with a fresh one.
        if let Err(why) = journal_gate(
            matches!(&journal_sections, Ok(n) if *n > 0),
            Certificates::last_index(&finalizations).map(|h| h.get()),
            epocher.first(epoch).map(|h| h.get()).unwrap_or(0),
            journal_sections.is_err(),
            previously_started,
        ) {
            tracing::error!(
                partition = %vote_partition,
                epoch = epoch.get(),
                %why,
                "this Mac's vote journal cannot be trusted: voting does not resume. \
                 The supervisor follows instead; the next committee round starts a \
                 fresh share and journal"
            );
            std::process::exit(EXIT_JOURNAL);
        }
        // Publish this before consensus can sign. A crash after publication
        // but before its first vote is conservative: it follows until the
        // next epoch if the journal is absent, never risks a double vote.
        if let Some(path) = marker.filter(|_| !previously_started) {
            if let Err(e) = crate::atomic::create(&path, epoch.get().to_string().as_bytes(), 0o600) {
                tracing::error!(%e, "cannot record vote-journal ownership; refusing to vote");
                std::process::exit(EXIT_JOURNAL);
            }
        }
        match journal_sections {
            Ok(sections) if sections > VOTE_JOURNAL_WARN_SECTIONS => warn!(
                partition = %vote_partition,
                sections,
                "vote journal holds many section files (one per view; pruned only below the last finalization): every one is opened at startup"
            ),
            Ok(sections) => tracing::info!(partition = %vote_partition, sections, "vote journal sections"),
            Err(e) => unreachable!("the gate exited on a journal lookup failure: {e}"),
        }
        // That count is a startup snapshot. While the chain runs without
        // finalizing, every burned view leaves another section file behind and
        // nothing bounds the pile — the raised limit only moves the ceiling
        // (2026-09-29 red-team 5). Keep counting as it grows and warn as the
        // journal approaches half the limit: that is the room to act in before
        // the next restart opens every section at once.
        {
            let watch = vote_partition.clone();
            context.child("journal_watch").spawn(|ctx| async move {
                loop {
                    ctx.sleep(std::time::Duration::from_secs(60)).await;
                    let at = vote_journal_warn_at(soft_nofile());
                    match partition_blobs(&ctx, &watch).await {
                        Ok(sections) if sections >= at => warn!(
                            partition = %watch,
                            sections,
                            warn_at = at,
                            "vote journal approaching the open-file limit (one section file per view, every one opened at startup): finalize — or recover and prune — before it is hit"
                        ),
                        Ok(_) => {}
                        Err(e) => warn!(%e, partition = %watch, "could not count vote journal sections"),
                    }
                }
            });
        }
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
        scheme.check_binding();
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
                partition: vote_partition,
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
                write_buffer: VOTE_WRITE_BUFFER,
                blocker: cfg.blocker,
                page_cache,
                elector: ELECTOR,
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
        // The RPC server has its own lifetime. A dead voting engine must
        // never leave it serving healthy-looking status indefinitely.
        std::process::exit(crate::supervisor::EXIT_FATAL_TASK);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The journal warning tracks the limit it will actually hit: launchd's
    /// 256-fd default warns where the fixed bar already is (128 sections plus
    /// the ~70 files a validator holds besides them), a raised limit only as
    /// the journal reaches half of it, and an unreadable or unlimited one
    /// falls back to the fixed bar alone.
    #[test]
    fn the_journal_warning_tracks_the_open_file_limit() {
        assert_eq!(vote_journal_warn_at(0), VOTE_JOURNAL_WARN_SECTIONS);
        assert_eq!(vote_journal_warn_at(256), 128);
        assert_eq!(vote_journal_warn_at(65_536), 32_768);
        assert_eq!(vote_journal_warn_at(u64::MAX), (u64::MAX / 2) as usize);
        assert!(vote_journal_warn_at(64) >= 64, "a bar above the limit itself still warns early");
    }

    /// Red team #4, the restart gate: a validator resumes voting only when
    /// the votes it cast in this epoch are still accounted for. Every way a
    /// Mac legitimately finds no journal passes; the one that means loss —
    /// deliveries inside the epoch, journal gone — refuses. There is no
    /// override bypass (audit 4, A4-3): a journal-less recovery never
    /// reaches the gate, `recover` refuses it first, and a restarted
    /// recovery passes through its own journal's votes like any restart.
    #[test]
    fn the_journal_gate_refuses_only_a_lost_journal() {
        const START: u64 = 3_600;

        // A normal restart mid-epoch: the journal holds this epoch's votes.
        assert!(journal_gate(true, Some(4_200), START, false, true).is_ok());
        // A first committee at genesis (epoch 0, journal present).
        assert!(journal_gate(true, Some(9), 0, false, true).is_ok());

        // A joining member: fresh archives, nothing delivered, no journal yet.
        assert!(journal_gate(false, None, START, false, false).is_ok());
        // The epoch just began; its deliveries are all from earlier epochs.
        assert!(journal_gate(false, Some(3_599), START, false, false).is_ok());
        // Exactly at the boundary: still nothing delivered inside the epoch.
        assert!(journal_gate(false, Some(START - 1), START, false, false).is_ok());
        // A validator can sign before the first finalization; its durable
        // epoch marker still forbids a new journal after loss.
        assert!(journal_gate(false, None, START, false, true).is_err());

        // The loss the gate exists for: this Mac delivered inside the epoch,
        // and the journal those votes went into is gone. A journal-less
        // recovery is exactly this case — the old bypass would have allowed
        // it to vote again from an empty journal.
        assert!(journal_gate(false, Some(START), START, false, false).is_err());
        assert!(journal_gate(false, Some(4_200), START, false, false).is_err());
        // And with the journal's own votes present, those same deliveries are
        // the normal restart this whole check must not break — including a
        // restarted AETHER_RECOVER_CONSENSUS recovery.
        assert!(journal_gate(true, Some(4_200), START, false, true).is_ok());
        // A journal that cannot be examined is not trusted either.
        assert!(journal_gate(false, Some(4_200), START, true, false).is_err());
    }
}
