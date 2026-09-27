//! Path selection that never carries traffic over private overlays.
//!
//! iroh learns candidate paths from every local interface, including a
//! Tailscale tunnel if the machine has one. Aether validators must work for
//! strangers on the public internet, so overlay paths are never selected:
//! traffic uses a direct public/LAN path, or else a public relay.

use crate::is_overlay_or_local;
use iroh::endpoint::transports::{FourTuple, PathSelection, PathSelectionContext, PathSelector};
use std::time::Duration;

#[derive(Debug, Default)]
pub struct PublicPathSelector;

/// `None` = must not be used. Lower sorts first: direct before relay, then RTT.
fn rank(path: &FourTuple, rtt: Duration) -> Option<(u8, Duration)> {
    match path {
        FourTuple::Ip { remote, local } => {
            if is_overlay_or_local(remote.ip()) || local.is_some_and(is_overlay_or_local) {
                None
            } else {
                Some((0, rtt))
            }
        }
        FourTuple::Relay { .. } => Some((1, rtt)),
        FourTuple::Custom { .. } => None,
    }
}

impl PathSelector for PublicPathSelector {
    fn select(&self, ctx: &PathSelectionContext<'_>) -> PathSelection {
        let current = ctx.current();
        let mut best = None;
        let mut current_rank = None;
        for p in ctx.paths() {
            let Some(stats) = p.stats() else { continue };
            let Some(r) = rank(p.network_path(), stats.rtt) else { continue };
            if Some(p.network_path()) == current && current_rank.is_none_or(|c| r < c) {
                current_rank = Some(r);
            }
            if best.as_ref().is_none_or(|(_, b)| r < *b) {
                best = Some((p, r));
            }
        }
        let mut sel = PathSelection::none();
        let Some((p, (tier, rtt))) = best else { return sel };
        // Keep an allowed current path unless the best one is clearly better
        // (other tier, or >20% and >5ms faster) to avoid flapping.
        let keep = current_rank.is_some_and(|(ct, crtt)| ct == tier && crtt <= rtt + (rtt / 5).max(Duration::from_millis(5)));
        if !keep {
            sel.set(&p);
        }
        sel
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlay_and_loopback_paths_are_never_ranked() {
        let rtt = Duration::from_millis(3);
        let ts = FourTuple::Ip { remote: "100.86.13.70:1".parse().unwrap(), local: None };
        let via_ts = FourTuple::Ip { remote: "203.0.113.20:1".parse().unwrap(), local: Some("100.101.1.2".parse().unwrap()) };
        let lo = FourTuple::Ip { remote: "127.0.0.1:1".parse().unwrap(), local: None };
        let public = FourTuple::Ip { remote: "203.0.113.20:1".parse().unwrap(), local: Some("192.168.0.10".parse().unwrap()) };
        assert_eq!(rank(&ts, rtt), None);
        assert_eq!(rank(&via_ts, rtt), None);
        assert_eq!(rank(&lo, rtt), None);
        assert_eq!(rank(&public, rtt), Some((0, rtt)));
    }

    #[test]
    fn overlay_ranges() {
        for ip in ["100.64.0.1", "100.127.255.254", "fd7a:115c:a1e0::1", "127.0.0.1", "fe80::1"] {
            assert!(is_overlay_or_local(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["100.63.255.255", "100.128.0.1", "203.0.113.20", "192.168.0.1", "2001:db8::1"] {
            assert!(!is_overlay_or_local(ip.parse().unwrap()), "{ip}");
        }
    }
}
