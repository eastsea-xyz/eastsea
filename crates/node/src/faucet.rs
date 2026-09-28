//! Testnet faucet (docs/design/12-launch-plan.md step 3).
//!
//! A public network has no public development keys: genesis funds one faucet
//! account whose key only the node operator holds (`aether faucet-key`). Nodes
//! started with that key answer `aether_faucet`: a fixed amount per address,
//! once per cooldown, and at most one grant per `MIN_INTERVAL` overall.
//! Per-device limits (App Attest) come in step 7.

use crate::chain::Chain;
use aether_crypto::{address_of, P256Signer, Signer as _};
use aether_execution::{sign_call_with, EvmCall};
use aether_types::{Address, Bytes, FeeVector, TxEnvelope, U256};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// 10 test AETH per grant.
pub const GRANT: u128 = 10 * 10u128.pow(18);
pub const COOLDOWN: Duration = Duration::from_secs(24 * 3600);
pub const MIN_INTERVAL: Duration = Duration::from_secs(1);
/// Grants per 24 h in total (fresh addresses cost nothing, so the faucet itself is capped).
pub const MAX_PER_DAY: u32 = 5_000;
/// Genesis supply of a faucet account (test tokens, no value).
pub const SUPPLY: u128 = 1_000_000_000 * 10u128.pow(18);
const TIP: u128 = 1_000_000_000;
/// Remembered addresses before old entries are dropped.
const MAX_TRACKED: usize = 100_000;
/// If this many grants are pending, assume some were dropped and resync the nonce.
const MAX_PENDING: u64 = 64;

#[derive(Debug, PartialEq, Eq)]
pub enum FaucetError {
    /// This address got a grant recently; retry after the given time.
    Cooldown(Duration),
    /// Too many requests overall; retry shortly.
    Busy,
    /// The faucet gave out its daily total; retry after the given time.
    DailyLimit(Duration),
    Signing(String),
}

impl std::fmt::Display for FaucetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FaucetError::Cooldown(d) => write!(
                f,
                "this address already received test tokens; try again in {} min",
                d.as_secs().div_ceil(60)
            ),
            FaucetError::Busy => write!(f, "faucet busy; try again in a second"),
            FaucetError::DailyLimit(d) => write!(
                f,
                "the faucet reached its daily limit; try again in {} min",
                d.as_secs().div_ceil(60)
            ),
            FaucetError::Signing(e) => write!(f, "faucet signing failed: {e}"),
        }
    }
}

struct State {
    last: HashMap<Address, Instant>,
    last_any: Option<Instant>,
    next_nonce: u64,
    day_start: Option<Instant>,
    today: u32,
}

pub struct Faucet {
    signer: P256Signer,
    pub address: Address,
    state: Mutex<State>,
}

impl Faucet {
    pub fn from_seed(seed: &[u8; 32]) -> Result<Self, String> {
        let signer = P256Signer::from_seed(seed).map_err(|e| e.to_string())?;
        let address = address_of(&signer.public_key()).map_err(|e| e.to_string())?;
        Ok(Faucet {
            signer,
            address,
            state: Mutex::new(State {
                last: HashMap::new(),
                last_any: None,
                next_nonce: 0,
                day_start: None,
                today: 0,
            }),
        })
    }

    /// Load a key written by `generate` (hex seed).
    pub fn load(path: &Path) -> Result<Self, String> {
        let hex_seed =
            std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let bytes = hex::decode(hex_seed.trim()).map_err(|e| format!("{}: {e}", path.display()))?;
        let seed: [u8; 32] = bytes
            .try_into()
            .map_err(|_| format!("{}: expected a 32-byte hex seed", path.display()))?;
        Self::from_seed(&seed)
    }

    /// Create a new random faucet key at `path` (owner-only permissions); returns its address.
    pub fn generate(path: &Path) -> Result<Address, String> {
        if path.exists() {
            return Err(format!("{} already exists", path.display()));
        }
        let seed: [u8; 32] = rand::random();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        write_private(path, hex::encode(seed).as_bytes())?;
        Ok(Self::from_seed(&seed)?.address)
    }

    /// Undo a grant that never reached the mempool, so its nonce is not skipped.
    pub fn cancel(&self, to: Address, tx: &TxEnvelope) {
        let mut st = self.state.lock().expect("faucet lock");
        if st.next_nonce == tx.header.nonce + 1 {
            st.next_nonce = tx.header.nonce;
            st.today = st.today.saturating_sub(1);
            st.last.remove(&to);
        }
    }

    /// The key's public half as x‖y hex (for a registrar key).
    pub fn public_hex(&self) -> String {
        let (x, y) = aether_crypto::p256_xy(&self.signer.public_key().bytes).expect("P-256 key");
        format!("{}{}", hex::encode(x), hex::encode(y))
    }

    /// Sign arbitrary bytes (a registrar attestation): raw r‖s.
    pub fn sign_bytes(&self, msg: &[u8]) -> Result<([u8; 32], [u8; 32]), String> {
        let sig = self.signer.sign(msg).map_err(|e| format!("{e:?}"))?;
        Ok((
            sig[..32].try_into().expect("32"),
            sig[32..64].try_into().expect("32"),
        ))
    }

    /// Sign a call from this key's account with no tip, caps at 2x `base`: free
    /// while the network is uncongested (R1′), so a new node account needs no funds.
    pub fn sign_tx(
        &self,
        chain_id: u64,
        nonce: u64,
        call: &EvmCall,
        base: FeeVector,
    ) -> Result<TxEnvelope, String> {
        let caps = FeeVector {
            exec: base.exec.saturating_mul(2),
            state: 0,
            prove: base.prove.saturating_mul(2),
        };
        sign_call_with(&self.signer, chain_id, nonce, caps, 0, call).map_err(|e| format!("{e:?}"))
    }

    /// Sign a grant to `to` if the limits allow it.
    pub fn grant(
        &self,
        chain: &Chain,
        to: Address,
        now: Instant,
    ) -> Result<TxEnvelope, FaucetError> {
        let (cfg, onchain_nonce, base) = {
            let g = chain.lock();
            let base = Chain::next_base_fee(&g.cfg, &g.finalized);
            (g.cfg.clone(), g.finalized.state.nonce(&self.address), base)
        };
        let mut st = self.state.lock().expect("faucet lock");
        if let Some(t) = st.last_any {
            if now.saturating_duration_since(t) < MIN_INTERVAL {
                return Err(FaucetError::Busy);
            }
        }
        let day = Duration::from_secs(24 * 3600);
        match st.day_start {
            Some(t) if now.saturating_duration_since(t) < day => {
                if st.today >= MAX_PER_DAY {
                    return Err(FaucetError::DailyLimit(
                        day - now.saturating_duration_since(t),
                    ));
                }
            }
            _ => {
                st.day_start = Some(now);
                st.today = 0;
            }
        }
        if let Some(t) = st.last.get(&to) {
            let waited = now.saturating_duration_since(*t);
            if waited < COOLDOWN {
                return Err(FaucetError::Cooldown(COOLDOWN - waited));
            }
        }
        if st.next_nonce < onchain_nonce || st.next_nonce - onchain_nonce > MAX_PENDING {
            st.next_nonce = onchain_nonce;
        }
        let call = EvmCall {
            to: Some(to),
            value: U256::from(GRANT),
            input: Bytes::new(),
            gas_limit: 21_000,
            delegate: None,
        };
        let caps = FeeVector {
            exec: base.exec.max(TIP) * 2 + TIP,
            state: 0,
            prove: base.prove.max(TIP) * 2,
        };
        let tx = sign_call_with(&self.signer, cfg.chain_id, st.next_nonce, caps, TIP, &call)
            .map_err(|e| FaucetError::Signing(e.to_string()))?;
        st.next_nonce += 1;
        st.today += 1;
        st.last_any = Some(now);
        if st.last.len() >= MAX_TRACKED {
            st.last
                .retain(|_, t| now.saturating_duration_since(*t) < COOLDOWN);
        }
        st.last.insert(to, now);
        Ok(tx)
    }
}

#[cfg(unix)]
fn write_private(path: &Path, data: &[u8]) -> Result<(), String> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|e| e.to_string())?;
    f.write_all(data).map_err(|e| e.to_string())
}

#[cfg(not(unix))]
fn write_private(path: &Path, data: &[u8]) -> Result<(), String> {
    std::fs::write(path, data).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chain::ChainConfig;
    use aether_types::GasVector;

    fn setup() -> (Faucet, Chain) {
        let f = Faucet::from_seed(&[5u8; 32]).unwrap();
        let cfg = ChainConfig {
            chain_id: 9,
            limits: GasVector {
                exec: 30_000_000,
                state: u64::MAX,
                prove: 200_000_000,
            },
            alloc: vec![(f.address, U256::from(SUPPLY))],
            fees: true,
            registrar: None,
            epoch_blocks: 0,
            min_streak: None,
            draw_epochs: None,
            history_v2: false,
            node_rewards: false,
        };
        (f, Chain::new(cfg).0)
    }

    #[test]
    fn grants_are_rate_limited_per_address_and_overall() {
        let (f, chain) = setup();
        let (a, b) = (Address::repeat_byte(1), Address::repeat_byte(2));
        let t0 = Instant::now();
        let tx = f.grant(&chain, a, t0).unwrap();
        assert_eq!(tx.header.nonce, 0);
        assert!(aether_execution::validate_stateless(&tx, 9).is_ok());
        assert_eq!(
            f.grant(&chain, b, t0),
            Err(FaucetError::Busy),
            "one grant per second overall"
        );
        let t1 = t0 + Duration::from_secs(2);
        assert!(
            matches!(f.grant(&chain, a, t1), Err(FaucetError::Cooldown(_))),
            "same address within the cooldown"
        );
        assert_eq!(
            f.grant(&chain, b, t1).unwrap().header.nonce,
            1,
            "nonces advance while grants are pending"
        );
        assert!(
            f.grant(&chain, a, t0 + COOLDOWN + Duration::from_secs(1))
                .is_ok(),
            "after the cooldown"
        );
    }

    #[test]
    fn the_faucet_has_a_daily_total() {
        let (f, chain) = setup();
        let t0 = Instant::now();
        for i in 0..MAX_PER_DAY {
            let mut a = [0u8; 20];
            a[..4].copy_from_slice(&i.to_be_bytes());
            assert!(f
                .grant(
                    &chain,
                    Address::from(a),
                    t0 + Duration::from_secs(u64::from(i) * 2)
                )
                .is_ok());
        }
        let late = t0 + Duration::from_secs(u64::from(MAX_PER_DAY) * 2);
        assert!(matches!(
            f.grant(&chain, Address::repeat_byte(0xee), late),
            Err(FaucetError::DailyLimit(_))
        ));
        assert!(
            f.grant(
                &chain,
                Address::repeat_byte(0xee),
                t0 + Duration::from_secs(24 * 3600 + 1)
            )
            .is_ok(),
            "a new day"
        );
    }

    #[test]
    fn grants_pay_the_current_base_fee() {
        let (f, chain) = setup();
        let tx = f
            .grant(&chain, Address::repeat_byte(3), Instant::now())
            .unwrap();
        let base = Chain::next_base_fee(&chain.cfg(), &chain.lock().finalized);
        assert!(tx.header.max_fee.exec >= base.exec + tx.header.tip);
        assert!(tx.header.max_fee.prove >= base.prove);
    }

    #[test]
    fn key_files_round_trip_and_are_private() {
        let dir = std::env::temp_dir().join(format!("aether-faucet-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("faucet.key");
        let addr = Faucet::generate(&path).unwrap();
        assert_eq!(Faucet::load(&path).unwrap().address, addr);
        assert!(Faucet::generate(&path).is_err(), "never overwrites a key");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
