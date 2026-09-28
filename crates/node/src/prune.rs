//! History pruning (roadmap B4, docs/research/history-compression-2026.md).
//!
//! A normal node keeps only the recent blocks: once an era (8192 blocks) is
//! sealed into its era file and older than the retention window, its marshal
//! blocks and finalization certificates, block summaries, receipts and a
//! follower's finality proofs go. The era file stays (or, with
//! `keep_era_files` off, only the era's root, which the history index keeps),
//! and peers serve old eras (`crate::era_net`), checked against a certified
//! history root before use.
//!
//! Pruning needs era files, which only history v2 networks write: the 7780
//! testnet (and any network without `"history": 2`) keeps everything, as it
//! always did.

use crate::chain::Chain;
use aether_state::mmr::ERA_LEN;

/// Default retention: 30 days.
pub const DEFAULT_RETAIN_DAYS: u64 = 30;
/// How often a node checks whether an era can go.
pub const INTERVAL: std::time::Duration = std::time::Duration::from_secs(60);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Retention {
    /// Blocks kept behind the head (rounded down to whole eras when pruning).
    pub blocks: u64,
    /// Keep era files of pruned eras (else only their roots).
    pub keep_era_files: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HistoryMode {
    /// Keep every block, certificate, summary and receipt.
    Archive,
    Prune(Retention),
}

impl HistoryMode {
    /// From the command line: `--history archive|prune` (default: prune on
    /// history v2 networks, archive otherwise), `--retain-days`, `--drop-era-files`.
    pub fn resolve(
        flag: Option<&str>,
        history_v2: bool,
        retain_days: u64,
        block_time_ms: u64,
        drop_era_files: bool,
    ) -> Result<HistoryMode, String> {
        let prune = match flag {
            None => history_v2,
            Some("archive") => false,
            Some("prune") if history_v2 => true,
            Some("prune") => {
                return Err("--history prune needs era files, which only history v2 networks (network.json \"history\": 2) write; this network keeps every block".into())
            }
            Some(other) => return Err(format!("--history {other}: expected archive or prune")),
        };
        if !prune {
            return Ok(HistoryMode::Archive);
        }
        if retain_days == 0 {
            return Err("--retain-days must be at least 1".into());
        }
        let blocks = retain_days.saturating_mul(86_400_000) / block_time_ms.max(1);
        Ok(HistoryMode::Prune(Retention { blocks, keep_era_files: !drop_era_files }))
    }

    pub fn prunes(&self) -> bool {
        matches!(self, HistoryMode::Prune(_))
    }
}

/// The height below which everything may go: the last era boundary at least
/// `retain` blocks behind `head`, but never past an era that is not sealed
/// yet (`staged`: eras whose blocks still wait for their file), and never
/// back below what is already pruned.
pub fn cutoff(head: u64, retain: u64, pruned_below: u64, staged: &[u64]) -> u64 {
    let target = head.saturating_sub(retain) / ERA_LEN * ERA_LEN;
    let unsealed = staged.iter().map(|e| e * ERA_LEN).min().unwrap_or(u64::MAX);
    target.min(unsealed).max(pruned_below)
}

/// One pruning pass: returns the new cutoff when something was pruned.
pub fn prune_once(chain: &Chain, r: &Retention) -> Result<Option<(u64, crate::store::PruneReport)>, String> {
    let Some(store) = chain.store() else { return Ok(None) };
    let head = chain.finalized_height();
    let before = chain.lock().pruned_below;
    let staged = store.staged_eras().map_err(|e| e.to_string())?;
    let c = cutoff(head, r.blocks, before, &staged);
    if c <= before {
        return Ok(None);
    }
    let report = chain.prune(c)?;
    if !r.keep_era_files {
        for e in before / ERA_LEN..c / ERA_LEN {
            let _ = std::fs::remove_file(store.era_dir().join(crate::era::file_name(e)));
        }
    }
    tracing::info!(below = c, summaries = report.summaries, receipts = report.receipts, proofs = report.proofs, "pruned history (era files kept: {})", r.keep_era_files);
    Ok(Some((c, report)))
}

/// Prune every `INTERVAL`, and ask marshal (validators) to drop its blocks and
/// certificates below the same height.
pub async fn run(chain: Chain, r: Retention, marshal: Option<crate::rpc::Marshal>) {
    loop {
        let (c, rr) = (chain.clone(), r.clone());
        match tokio::task::spawn_blocking(move || prune_once(&c, &rr)).await {
            Ok(Err(e)) => tracing::warn!(%e, "pruning failed; nothing was lost, will retry"),
            Err(e) => tracing::warn!(%e, "pruning task failed"),
            Ok(Ok(_)) => {}
        }
        let below = chain.lock().pruned_below;
        if let (Some(m), true) = (&marshal, below > 0) {
            // Idempotent; also covers a restart after the store was pruned.
            m.prune(commonware_consensus::types::Height::new(below));
        }
        tokio::time::sleep(INTERVAL).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cutoff_keeps_the_window_and_unsealed_eras() {
        let day = 86_400;
        // Nothing old enough yet.
        assert_eq!(cutoff(10 * ERA_LEN, 30 * day, 0, &[]), 0);
        // 30 days behind a head of 40 days: whole eras only.
        let head = 40 * day;
        let c = cutoff(head, 30 * day, 0, &[head / ERA_LEN]);
        assert_eq!(c, (10 * day) / ERA_LEN * ERA_LEN);
        assert!(head - c >= 30 * day && c.is_multiple_of(ERA_LEN));
        // An era waiting for its file holds pruning back.
        assert_eq!(cutoff(head, 30 * day, 0, &[3, head / ERA_LEN]), 3 * ERA_LEN);
        // Never goes back.
        assert_eq!(cutoff(head, 30 * day, 50 * ERA_LEN, &[3]), 50 * ERA_LEN);
    }

    #[test]
    fn modes_from_flags() {
        assert_eq!(HistoryMode::resolve(None, false, 30, 1000, false).unwrap(), HistoryMode::Archive, "7780: unchanged");
        assert!(HistoryMode::resolve(Some("prune"), false, 30, 1000, false).is_err(), "no era files on 7780");
        assert_eq!(
            HistoryMode::resolve(None, true, 30, 1000, false).unwrap(),
            HistoryMode::Prune(Retention { blocks: 30 * 86_400, keep_era_files: true })
        );
        assert_eq!(HistoryMode::resolve(Some("archive"), true, 30, 1000, false).unwrap(), HistoryMode::Archive);
        assert_eq!(
            HistoryMode::resolve(Some("prune"), true, 7, 500, true).unwrap(),
            HistoryMode::Prune(Retention { blocks: 7 * 86_400 * 2, keep_era_files: false })
        );
        assert!(HistoryMode::resolve(Some("x"), true, 30, 1000, false).is_err());
        assert!(HistoryMode::resolve(None, true, 0, 1000, false).is_err());
    }
}
