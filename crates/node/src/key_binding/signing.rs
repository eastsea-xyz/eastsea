//! Node-local hardware checks around the existing consensus cryptography.
//!
//! Certificates, signatures and leader selection retain the raw scheme's
//! types and behavior. Only producing a signature consults the key binding.

use super::Guard;
use aether_light::Scheme as RawScheme;
use commonware_codec::Read;
use commonware_consensus::{simplex::elector, types::Round};
use commonware_cryptography::{
    certificate::{self, AssemblyError, Attestation, Verification},
    Digest, Sha256,
};
use commonware_parallel::Strategy;
use commonware_utils::{iter::NonEmpty, ordered::Set, Participant};
use rand::CryptoRng;

/// The existing BLS scheme, checking this Mac's binding before every vote.
#[derive(Clone, Debug)]
pub struct Scheme {
    inner: RawScheme,
    guard: Option<Guard>,
}

impl Scheme {
    pub fn new(inner: RawScheme, guard: Option<Guard>) -> Self {
        Self { inner, guard }
    }

    pub fn check_binding(&self) {
        if let Some(guard) = &self.guard {
            guard.check_or_exit();
        }
    }

    pub fn certificate_verifier(namespace: &[u8], identity: aether_light::Identity) -> Self {
        RawScheme::certificate_verifier(namespace, identity).into()
    }
}

/// Simulations and verify-only schemes have no persisted hardware-bound key.
impl From<RawScheme> for Scheme {
    fn from(inner: RawScheme) -> Self {
        Self::new(inner, None)
    }
}

fn raw_attestation(attestation: Attestation<Scheme>) -> Attestation<RawScheme> {
    Attestation {
        signer: attestation.signer,
        signature: attestation.signature,
    }
}

fn bound_attestation(attestation: Attestation<RawScheme>) -> Attestation<Scheme> {
    Attestation {
        signer: attestation.signer,
        signature: attestation.signature,
    }
}

impl certificate::Verifier for Scheme {
    type Subject<'a, D: Digest> = <RawScheme as certificate::Verifier>::Subject<'a, D>;
    type Faults = <RawScheme as certificate::Verifier>::Faults;
    type PublicKey = <RawScheme as certificate::Verifier>::PublicKey;
    type Certificate = <RawScheme as certificate::Verifier>::Certificate;

    fn verify_certificate<R, D>(
        &self,
        rng: &mut R,
        subject: Self::Subject<'_, D>,
        certificate: &Self::Certificate,
        strategy: &impl Strategy,
    ) -> bool
    where
        R: CryptoRng,
        D: Digest,
    {
        self.inner
            .verify_certificate(rng, subject, certificate, strategy)
    }

    fn verify_certificates<'a, R, D, I>(
        &self,
        rng: &mut R,
        certificates: NonEmpty<I>,
        strategy: &impl Strategy,
    ) -> bool
    where
        R: CryptoRng,
        D: Digest,
        I: Iterator<Item = (Self::Subject<'a, D>, &'a Self::Certificate)>,
    {
        self.inner.verify_certificates(rng, certificates, strategy)
    }

    fn verify_certificates_bisect<'a, R, D>(
        &self,
        rng: &mut R,
        certificates: &[(Self::Subject<'a, D>, &'a Self::Certificate)],
        strategy: &impl Strategy,
    ) -> Vec<bool>
    where
        R: CryptoRng,
        D: Digest,
        Self::Subject<'a, D>: Copy,
        Self::Certificate: 'a,
    {
        self.inner
            .verify_certificates_bisect(rng, certificates, strategy)
    }

    fn is_batchable() -> bool {
        <RawScheme as certificate::Verifier>::is_batchable()
    }

    fn certificate_codec_config(&self) -> <Self::Certificate as Read>::Cfg {
        self.inner.certificate_codec_config()
    }

    fn certificate_codec_config_unbounded() -> <Self::Certificate as Read>::Cfg {
        <RawScheme as certificate::Verifier>::certificate_codec_config_unbounded()
    }
}

impl certificate::Scheme for Scheme {
    type Signature = <RawScheme as certificate::Scheme>::Signature;

    fn me(&self) -> Option<Participant> {
        self.inner.me()
    }

    fn participants(&self) -> &Set<Self::PublicKey> {
        self.inner.participants()
    }

    fn sign<D: Digest>(&self, subject: Self::Subject<'_, D>) -> Option<Attestation<Self>> {
        // Includes timeout/nullify votes, which have no Automaton callback.
        // Every vote checks, including the first one of each epoch.
        self.check_binding();
        self.inner.sign(subject).map(bound_attestation)
    }

    fn verify_attestation<R, D>(
        &self,
        rng: &mut R,
        subject: Self::Subject<'_, D>,
        attestation: &Attestation<Self>,
        strategy: &impl Strategy,
    ) -> bool
    where
        R: CryptoRng,
        D: Digest,
    {
        let raw = raw_attestation(attestation.clone());
        self.inner.verify_attestation(rng, subject, &raw, strategy)
    }

    fn verify_attestations<R, D, I>(
        &self,
        rng: &mut R,
        subject: Self::Subject<'_, D>,
        attestations: I,
        strategy: &impl Strategy,
    ) -> Verification<Self>
    where
        R: CryptoRng,
        D: Digest,
        I: IntoIterator<Item = Attestation<Self>>,
        I::IntoIter: Send,
    {
        let result = self.inner.verify_attestations(
            rng,
            subject,
            attestations.into_iter().map(raw_attestation),
            strategy,
        );
        Verification::new(
            result.verified.into_iter().map(bound_attestation).collect(),
            result.invalid,
        )
    }

    fn assemble<I>(
        &self,
        attestations: NonEmpty<I>,
        strategy: &impl Strategy,
    ) -> Result<Self::Certificate, AssemblyError>
    where
        I: Iterator<Item = Attestation<Self>> + Send,
    {
        let (first, rest) = attestations.into_parts();
        self.inner.assemble(
            NonEmpty::new(raw_attestation(first), rest.map(raw_attestation)),
            strategy,
        )
    }

    fn is_attributable() -> bool {
        <RawScheme as certificate::Scheme>::is_attributable()
    }
}

/// Adapter for the exact leader-election configuration used by light clients.
#[derive(Clone, Debug)]
pub struct Elector(aether_light::Elector);

pub const ELECTOR: Elector = Elector(aether_light::ELECTOR);

#[derive(Clone, Debug)]
pub struct LeaderElector(elector::RandomElector<RawScheme, Sha256>);

impl elector::Config<Scheme> for Elector {
    type Elector = LeaderElector;

    fn build(
        self,
        participants: &Set<<Scheme as certificate::Verifier>::PublicKey>,
    ) -> Self::Elector {
        LeaderElector(
            <aether_light::Elector as elector::Config<RawScheme>>::build(self.0, participants),
        )
    }
}

impl elector::Elector<Scheme> for LeaderElector {
    fn terms(&self) -> elector::Terms {
        <elector::RandomElector<RawScheme, Sha256> as elector::Elector<RawScheme>>::terms(&self.0)
    }

    fn elect(
        &self,
        round: Round,
        certificate: Option<&<Scheme as certificate::Verifier>::Certificate>,
    ) -> Participant {
        <elector::RandomElector<RawScheme, Sha256> as elector::Elector<RawScheme>>::elect(
            &self.0,
            round,
            certificate,
        )
    }
}

#[cfg(all(test, feature = "test-seam", debug_assertions))]
mod tests {
    use super::*;
    use commonware_codec::Encode;
    use commonware_consensus::{
        simplex::types::{Proposal, Subject},
        types::{Epoch, View},
    };
    use commonware_cryptography::{
        certificate::{Scheme as _, Verifier as _},
        sha256::Digest as BlockDigest,
        Hasher as _,
    };
    use commonware_parallel::Sequential;
    use commonware_utils::ordered::Quorum as _;
    use rand::SeedableRng;
    use sha2::{Digest as _, Sha256 as BindingHash};
    use std::path::{Path, PathBuf};

    fn signers() -> Vec<RawScheme> {
        let (participants, polynomial, shares) = aether_light::devnet_threshold(4);
        shares
            .into_iter()
            .map(|(_, share)| {
                RawScheme::signer(
                    &aether_light::consensus_namespace(),
                    participants.clone(),
                    polynomial.clone(),
                    share,
                )
                .expect("devnet share matches")
            })
            .collect()
    }

    fn proposal(epoch: u64) -> Proposal<BlockDigest> {
        Proposal::new(
            Round::new(Epoch::new(epoch), View::new(1)),
            View::zero(),
            Sha256::hash(&[b"key-binding vote"]),
        )
    }

    #[test]
    fn raw_and_bound_votes_have_identical_wire_bytes_and_verify() {
        let raw = signers();
        let bound: Vec<_> = raw.iter().cloned().map(Scheme::from).collect();
        let proposal = proposal(0);
        for subject in [
            Subject::Notarize {
                proposal: &proposal,
            },
            Subject::Finalize {
                proposal: &proposal,
            },
            Subject::Nullify {
                round: proposal.round,
            },
        ] {
            let raw_votes: Vec<_> = raw
                .iter()
                .map(|s| s.sign(subject).expect("raw share signs"))
                .collect();
            let bound_votes: Vec<_> = bound
                .iter()
                .map(|s| s.sign(subject).expect("bound share signs"))
                .collect();
            for (raw_vote, bound_vote) in raw_votes.iter().zip(&bound_votes) {
                assert_eq!(
                    raw_vote.encode(),
                    bound_vote.encode(),
                    "guarding must preserve vote bytes"
                );
            }
            let mut rng = rand::rngs::StdRng::from_seed([1; 32]);
            let checked =
                bound[0].verify_attestations(&mut rng, subject, bound_votes.clone(), &Sequential);
            assert!(checked.invalid.is_empty());
            assert_eq!(checked.verified.len(), bound_votes.len());
            let raw_certificate = raw[0]
                .assemble(
                    NonEmpty::try_new(raw_votes.into_iter()).unwrap(),
                    &Sequential,
                )
                .unwrap();
            let certificate = bound[0]
                .assemble(
                    NonEmpty::try_new(bound_votes.into_iter()).unwrap(),
                    &Sequential,
                )
                .unwrap();
            assert_eq!(
                raw_certificate.encode(),
                certificate.encode(),
                "guarding must preserve certificate bytes"
            );
            assert!(raw[0].verify_certificate(&mut rng, subject, &certificate, &Sequential));
            assert!(bound[0].verify_certificate(&mut rng, subject, &raw_certificate, &Sequential));
            assert!(bound[0].verify_certificates(
                &mut rng,
                NonEmpty::new((subject, &certificate), std::iter::empty()),
                &Sequential
            ));
        }
    }

    #[test]
    fn guarded_elector_matches_light_client_for_certified_rounds() {
        let raw = signers();
        let raw_elector = <aether_light::Elector as elector::Config<RawScheme>>::build(
            aether_light::ELECTOR,
            raw[0].participants(),
        );
        let bound_elector =
            <Elector as elector::Config<Scheme>>::build(ELECTOR, raw[0].participants());
        assert_eq!(
            <LeaderElector as elector::Elector<Scheme>>::terms(&bound_elector),
            <elector::RandomElector<RawScheme, Sha256> as elector::Elector<RawScheme>>::terms(
                &raw_elector
            ),
        );
        for epoch in [0, 1, 7, u64::MAX] {
            let proposal = proposal(epoch);
            let first = proposal.round;
            assert_eq!(
                <LeaderElector as elector::Elector<Scheme>>::elect(&bound_elector, first, None),
                <elector::RandomElector<RawScheme, Sha256> as elector::Elector<RawScheme>>::elect(
                    &raw_elector,
                    first,
                    None
                ),
            );
            for subject in [
                Subject::Notarize {
                    proposal: &proposal,
                },
                Subject::Finalize {
                    proposal: &proposal,
                },
                Subject::Nullify { round: first },
            ] {
                let votes = raw
                    .iter()
                    .map(|s| s.sign(subject).expect("raw share signs"));
                let certificate = raw[0]
                    .assemble(NonEmpty::try_new(votes).unwrap(), &Sequential)
                    .unwrap();
                let next = Round::new(Epoch::new(epoch), View::new(2));
                assert_eq!(
                    <LeaderElector as elector::Elector<Scheme>>::elect(&bound_elector, next, Some(&certificate)),
                    <elector::RandomElector<RawScheme, Sha256> as elector::Elector<RawScheme>>::elect(&raw_elector, next, Some(&certificate)),
                );
            }
        }
    }

    struct Dir(PathBuf);

    impl Dir {
        fn new() -> Self {
            let root = Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .parent()
                .unwrap();
            let unique = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = root.join("tmp").join(format!(
                "aether-binding-signing-{}-{unique}",
                std::process::id()
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    const CHILD_DIR: &str = "AETHER_BINDING_SIGNING_CHILD_DIR";
    const FIRST_VOTE: &str = "BOUND_FIRST_VOTE_VERIFIED";
    const AFTER_NULLIFY: &str = "BOUND_NULLIFY_AFTER_UUID_CHANGE";

    #[test]
    fn runtime_uuid_change_refuses_nullify_in_next_epoch() {
        let dir = Dir::new();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "key_binding::signing::tests::runtime_uuid_change_child",
                "--nocapture",
            ])
            .env(CHILD_DIR, &dir.0)
            .env("AETHER_TEST_PLATFORM_UUID", "MAC-A")
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains(FIRST_VOTE),
            "child must first sign and verify a bound vote: {stderr}"
        );
        assert_eq!(
            output.status.code(),
            Some(super::super::EXIT_KEY_ELSEWHERE),
            "UUID change must stop before the next epoch's nullify vote: {stderr}"
        );
        assert!(
            !stderr.contains(AFTER_NULLIFY),
            "a nullify signature must not be produced after UUID changes: {stderr}"
        );
    }

    #[test]
    fn runtime_uuid_change_child() {
        let Some(dir) = std::env::var_os(CHILD_DIR) else {
            return;
        };
        let dir = PathBuf::from(dir);
        let raw = signers().remove(0);
        let public = raw.participants().key(raw.me().unwrap()).unwrap().clone();
        let hash = BindingHash::digest([b"eastsea.bind".as_slice(), b"MAC-A"].concat());
        std::fs::write(
            dir.join(super::super::BINDING_FILE),
            serde_json::to_vec(&serde_json::json!({
                "validator_pub": hex::encode(public.as_ref()),
                "platform_uuid_hash": hex::encode(hash),
                "created_at": 1,
            }))
            .unwrap(),
        )
        .unwrap();
        let scheme = Scheme::new(raw, Some(Guard::for_validator(&dir, &public)));
        let proposal = proposal(0);
        let subject = Subject::Notarize {
            proposal: &proposal,
        };
        let vote = scheme.sign(subject).expect("matching bound share signs");
        let mut rng = rand::rngs::StdRng::from_seed([2; 32]);
        assert!(scheme.verify_attestation(&mut rng, subject, &vote, &Sequential));
        eprintln!("{FIRST_VOTE}");
        std::env::set_var("AETHER_TEST_PLATFORM_UUID", "MAC-B");
        let _ = scheme.sign::<BlockDigest>(Subject::Nullify {
            round: Round::new(Epoch::new(1), View::new(1)),
        });
        eprintln!("{AFTER_NULLIFY}");
    }
}
