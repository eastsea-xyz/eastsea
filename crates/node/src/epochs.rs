//! Committee epochs (validator rotation, design D8/D9).
//!
//! Each reshare starts a new epoch at an agreed height; the committee identity
//! (BLS group key) never changes, so certificates from every epoch verify under
//! the same key: that is what lets wallets pin one key forever, and lets a
//! newly joined validator verify old blocks while it catches up.

use aether_light::Scheme;
use commonware_consensus::types::{Epoch, EpochInfo, Epocher, Height};
use commonware_cryptography::certificate::{Provider, Scoped};
use std::sync::Arc;

/// Epoch `k > 0` starts at `starts[k - 1]`; epoch 0 starts at genesis. The last
/// epoch is open-ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScheduleEpocher {
    starts: Arc<Vec<u64>>,
}

impl ScheduleEpocher {
    /// `starts` must be strictly increasing and > 0.
    pub fn new(starts: Vec<u64>) -> Self {
        assert!(starts.first().is_none_or(|s| *s > 1), "epoch 1 must start after genesis");
        assert!(starts.windows(2).all(|w| w[0] + 1 < w[1]), "epoch starts must increase by more than one");
        ScheduleEpocher { starts: Arc::new(starts) }
    }

    /// The epoch the node runs in: the latest one.
    pub fn current(&self) -> Epoch {
        Epoch::new(self.starts.len() as u64)
    }

    fn bounds(&self, epoch: Epoch) -> Option<(Height, Height)> {
        let k = epoch.get() as usize;
        if k > self.starts.len() {
            return None;
        }
        let first = if k == 0 { 0 } else { self.starts[k - 1] };
        let last = self.starts.get(k).map(|next| next - 1).unwrap_or(u64::MAX - 1);
        Some((Height::new(first), Height::new(last)))
    }
}

impl Epocher for ScheduleEpocher {
    fn containing(&self, height: Height) -> Option<EpochInfo> {
        let k = self.starts.iter().take_while(|s| **s <= height.get()).count() as u64;
        let epoch = Epoch::new(k);
        let (first, last) = self.bounds(epoch)?;
        Some(EpochInfo::new(epoch, height, first, last))
    }

    fn first(&self, epoch: Epoch) -> Option<Height> {
        self.bounds(epoch).map(|b| b.0)
    }

    fn last(&self, epoch: Epoch) -> Option<Height> {
        self.bounds(epoch).map(|b| b.1)
    }
}

/// Our signing scheme for the current epoch; a verify-only scheme (same group
/// key) for every other epoch.
#[derive(Clone)]
pub struct RotatingProvider {
    current: Epoch,
    signer: Arc<Scheme>,
    verifier: Arc<Scheme>,
}

impl RotatingProvider {
    pub fn new(current: Epoch, signer: Scheme, identity: aether_light::Identity) -> Self {
        let verifier = Scheme::certificate_verifier(&aether_light::consensus_namespace(), identity);
        RotatingProvider { current, signer: Arc::new(signer), verifier: Arc::new(verifier) }
    }
}

impl Provider for RotatingProvider {
    type Scope = Epoch;
    type Scheme = Scheme;

    fn scoped(&self, epoch: Epoch) -> Option<Scoped<Scheme>> {
        if epoch == self.current {
            Some(Scoped::scheme(self.signer.clone()))
        } else {
            Some(Scoped::verifier(self.verifier.clone()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schedule_bounds() {
        let e = ScheduleEpocher::new(vec![25, 100]);
        assert_eq!(e.current(), Epoch::new(2));
        let at = |h| e.containing(Height::new(h)).unwrap().epoch().get();
        assert_eq!((at(0), at(24), at(25), at(99), at(100), at(1_000_000)), (0, 0, 1, 1, 2, 2));
        assert_eq!(e.last(Epoch::new(0)), Some(Height::new(24)));
        assert_eq!(e.first(Epoch::new(1)), Some(Height::new(25)));
        assert_eq!(e.last(Epoch::new(2)), Some(Height::new(u64::MAX - 1)));
        assert_eq!(e.first(Epoch::new(3)), None);
        let single = ScheduleEpocher::new(vec![]);
        assert_eq!(single.current(), Epoch::zero());
        assert_eq!(single.containing(Height::new(5)).unwrap().epoch(), Epoch::zero());
    }
}
