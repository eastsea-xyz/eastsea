//! Design 33's separate salted BEP-44 ContactV1 read advertisement.
//! Unsalted pkarr/iroh address publication remains owned by the address adapter.
//! This is known-key service discovery; it does not implement a torrent introducer.

use super::{Endpoint, EndpointId, SecretKey, TransportAddr};
use n0_mainline::{Dht, MutableItem, SigningKey};
use sha2::{Digest, Sha256};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;

pub const MAX_CONTACT_BYTES: usize = 768;
pub const CONTACT_TTL: u64 = 7200;

pub fn chain_fingerprint(chain_id: u64, identity: &[u8; 96], genesis: &[u8; 32]) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"eastsea-chain-v1\0");
    hash.update(chain_id.to_be_bytes());
    hash.update(identity);
    hash.update(genesis);
    hash.finalize().into()
}

pub fn contact_salt(chain: &[u8; 32], group: u16) -> [u8; 53] {
    let mut salt = [0u8; 53];
    salt[..19].copy_from_slice(b"eastsea-contact-v1\0");
    salt[19..51].copy_from_slice(chain);
    salt[51..].copy_from_slice(&group.to_be_bytes());
    salt
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Contact {
    pub chain: [u8; 32],
    pub chain_id: u64,
    pub group: u16,
    pub node: EndpointId,
    pub issued: u64,
    pub expires: u64,
    pub height: u64,
    pub direct: Vec<SocketAddr>,
    pub relays: Vec<String>,
}

impl Contact {
    pub fn verify_item(
        item: &MutableItem,
        expected_key: &EndpointId,
        chain: &[u8; 32],
        group: u16,
        now: u64,
        sequence_floor: i64,
    ) -> Result<Self, String> {
        let salt = contact_salt(chain, group);
        if item.key() != expected_key.as_bytes()
            || item.salt() != Some(salt.as_slice())
            || item.seq() <= 0
            || item.seq() < sequence_floor
        {
            return Err("contact key, salt or sequence".into());
        }
        verify_signed_item(item, expected_key, &salt)?;
        let contact = Self::decode(item.value(), chain, group, now)?;
        if contact.node != *expected_key {
            return Err("contact embedded node key".into());
        }
        Ok(contact)
    }

    pub fn for_endpoint(
        endpoint: &Endpoint,
        chain: [u8; 32],
        chain_id: u64,
        group: u16,
        height: u64,
        now: u64,
    ) -> Self {
        let addr = endpoint.addr();
        let direct = addr
            .addrs
            .iter()
            .filter_map(|addr| match addr {
                TransportAddr::Ip(addr) if super::is_public_ip(addr.ip()) && addr.port() != 0 => {
                    Some(*addr)
                }
                _ => None,
            })
            .take(4)
            .collect();
        let relays = addr
            .relay_urls()
            .map(ToString::to_string)
            .filter(|url| url.starts_with("https://") && url.len() <= 128 && url.is_ascii())
            .take(2)
            .collect();
        Self {
            chain,
            chain_id,
            group,
            node: endpoint.id(),
            issued: now,
            expires: now.saturating_add(CONTACT_TTL),
            height,
            direct,
            relays,
        }
    }

    pub fn encode(&self) -> Result<Vec<u8>, String> {
        if self.direct.len() > 4 || self.relays.len() > 2 {
            return Err("contact address count".into());
        }
        let mut out = Vec::with_capacity(MAX_CONTACT_BYTES);
        out.extend_from_slice(b"ESCA\x01\x00\x01"); // v1, read/proofs role only
        out.extend_from_slice(&self.chain);
        out.extend_from_slice(&self.chain_id.to_be_bytes());
        out.extend_from_slice(&self.group.to_be_bytes());
        out.extend_from_slice(self.node.as_bytes());
        out.extend_from_slice(&self.issued.to_be_bytes());
        out.extend_from_slice(&self.expires.to_be_bytes());
        out.extend_from_slice(&self.height.to_be_bytes());
        out.extend_from_slice(&[0u8; 36]); // No ServiceIndex, hash and size both zero.
        out.push(self.direct.len() as u8);
        for address in &self.direct {
            if !super::is_public_ip(address.ip()) || address.port() == 0 {
                return Err("contact non-public direct address".into());
            }
            match address.ip() {
                IpAddr::V4(ip) => {
                    out.push(4);
                    out.extend_from_slice(&ip.octets());
                }
                IpAddr::V6(ip) => {
                    out.push(6);
                    out.extend_from_slice(&ip.octets());
                }
            }
            out.extend_from_slice(&address.port().to_be_bytes());
        }
        out.push(self.relays.len() as u8);
        for relay in &self.relays {
            if relay.len() > 128 || !relay.is_ascii() || !relay.starts_with("https://") {
                return Err("contact relay URL".into());
            }
            let url = relay
                .parse::<iroh::RelayUrl>()
                .map_err(|e| format!("contact relay URL: {e}"))?;
            if !url.username().is_empty() || url.password().is_some() {
                return Err("contact relay credentials".into());
            }
            out.push(relay.len() as u8);
            out.extend_from_slice(relay.as_bytes());
        }
        out.extend_from_slice(&[0, 0]); // No HTTPS gateway, no candidate binding.
        if out.len() > MAX_CONTACT_BYTES {
            return Err("contact too large".into());
        }
        Ok(out)
    }

    pub fn decode(bytes: &[u8], chain: &[u8; 32], group: u16, now: u64) -> Result<Self, String> {
        if bytes.len() > MAX_CONTACT_BYTES || bytes.len() < 145 {
            return Err("contact size".into());
        }
        let mut cursor = Cursor(bytes);
        if cursor.take(7)? != b"ESCA\x01\x00\x01" {
            return Err("contact version or role".into());
        }
        let found_chain = cursor.array::<32>()?;
        let chain_id = u64::from_be_bytes(cursor.array()?);
        let found_group = u16::from_be_bytes(cursor.array()?);
        if &found_chain != chain || found_group != group {
            return Err("contact wrong network".into());
        }
        let node =
            EndpointId::from_bytes(&cursor.array()?).map_err(|e| format!("contact node: {e}"))?;
        let issued = u64::from_be_bytes(cursor.array()?);
        let expires = u64::from_be_bytes(cursor.array()?);
        let height = u64::from_be_bytes(cursor.array()?);
        if issued > now.saturating_add(120)
            || expires <= now
            || expires <= issued
            || expires > issued.saturating_add(CONTACT_TTL)
        {
            return Err("contact expired or invalid time".into());
        }
        if cursor.take(36)?.iter().any(|b| *b != 0) {
            return Err("unsupported contact ServiceIndex".into());
        }
        let count = cursor.byte()?;
        if count > 4 {
            return Err("contact direct count".into());
        }
        let mut direct = Vec::with_capacity(count as usize);
        for _ in 0..count {
            let ip = match cursor.byte()? {
                4 => IpAddr::V4(Ipv4Addr::from(cursor.array::<4>()?)),
                6 => IpAddr::V6(Ipv6Addr::from(cursor.array::<16>()?)),
                _ => return Err("contact address family".into()),
            };
            let port = u16::from_be_bytes(cursor.array()?);
            if !super::is_public_ip(ip) || port == 0 {
                return Err("contact non-public direct address".into());
            }
            direct.push(SocketAddr::new(ip, port));
        }
        let count = cursor.byte()?;
        if count > 2 {
            return Err("contact relay count".into());
        }
        let mut relays = Vec::with_capacity(count as usize);
        for _ in 0..count {
            let length = cursor.byte()? as usize;
            if length > 128 {
                return Err("contact relay length".into());
            }
            let relay = std::str::from_utf8(cursor.take(length)?)
                .map_err(|_| "contact relay UTF-8")?
                .to_string();
            if !relay.is_ascii() || !relay.starts_with("https://") {
                return Err("contact relay URL".into());
            }
            let url = relay
                .parse::<iroh::RelayUrl>()
                .map_err(|e| format!("contact relay URL: {e}"))?;
            if !url.username().is_empty() || url.password().is_some() {
                return Err("contact relay credentials".into());
            }
            relays.push(relay);
        }
        if cursor.take(2)? != [0, 0] || !cursor.0.is_empty() {
            return Err("unsupported contact fields or trailing bytes".into());
        }
        Ok(Self {
            chain: found_chain,
            chain_id,
            group,
            node,
            issued,
            expires,
            height,
            direct,
            relays,
        })
    }

    pub fn signed(&self, secret: &SecretKey, seq: i64) -> Result<MutableItem, String> {
        if seq <= 0 || secret.public() != self.node {
            return Err("contact signer or sequence".into());
        }
        let salt = contact_salt(&self.chain, self.group);
        Ok(MutableItem::new(
            &SigningKey::from_bytes(&secret.to_bytes()),
            &self.encode()?,
            seq,
            Some(&salt),
        ))
    }
}

fn verify_signed_item(item: &MutableItem, key: &EndpointId, salt: &[u8]) -> Result<(), String> {
    if item.key() != key.as_bytes() || item.salt() != Some(salt) || item.seq() <= 0 {
        return Err("contact key, salt or sequence".into());
    }
    if item.target() != &MutableItem::target_from_key(key.as_bytes(), Some(salt)) {
        return Err("contact target".into());
    }
    if item.value().len() > MAX_CONTACT_BYTES {
        return Err("contact size".into());
    }
    let mut signable = format!("4:salt{}:", salt.len()).into_bytes();
    signable.extend_from_slice(salt);
    signable
        .extend_from_slice(format!("3:seqi{}e1:v{}:", item.seq(), item.value().len()).as_bytes());
    signable.extend_from_slice(item.value());
    key.verify(&signable, &iroh::Signature::from_bytes(item.signature()))
        .map_err(|_| "contact signature".to_string())
}

struct Cursor<'a>(&'a [u8]);
impl<'a> Cursor<'a> {
    fn take(&mut self, count: usize) -> Result<&'a [u8], String> {
        if self.0.len() < count {
            return Err("truncated contact".into());
        }
        let (taken, rest) = self.0.split_at(count);
        self.0 = rest;
        Ok(taken)
    }
    fn byte(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }
    fn array<const N: usize>(&mut self) -> Result<[u8; N], String> {
        self.take(N)?
            .try_into()
            .map_err(|_| "truncated contact".into())
    }
}

/// Caller retains sequence floors and signs only after durably increasing them.
pub struct Publisher(Dht);
impl Publisher {
    pub fn new() -> Result<Self, String> {
        Dht::builder()
            .build()
            .map(Self)
            .map_err(|e| format!("contact DHT: {e}"))
    }
    pub async fn publish(&self, item: MutableItem) -> Result<(), String> {
        tokio::time::timeout(Duration::from_secs(30), self.0.put_mutable(item, None))
            .await
            .map_err(|_| "contact DHT publish timed out".to_string())?
            .map(|_| ())
            .map_err(|e| format!("contact DHT publish: {e}"))
    }
    /// Recover a remote high-water mark before the first update. A network error
    /// does not justify sequence rollback; callers retry rather than replacing it.
    pub async fn latest_seq(&self, key: &EndpointId, salt: &[u8]) -> Result<i64, String> {
        let item = tokio::time::timeout(
            Duration::from_secs(30),
            self.0.get_mutable_most_recent(key.as_bytes(), Some(salt)),
        )
        .await
        .map_err(|_| "contact DHT lookup timed out".to_string())?
        .map_err(|e| format!("contact DHT lookup: {e}"))?;
        match item {
            Some(item) => {
                verify_signed_item(&item, key, salt)?;
                Ok(item.seq())
            }
            None => Ok(0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn contact() -> Contact {
        Contact {
            chain: [3; 32],
            chain_id: 7781,
            group: 2,
            node: SecretKey::from_bytes(&[1; 32]).public(),
            issued: 1000,
            expires: 8200,
            height: 7,
            direct: vec!["8.8.8.8:1234".parse().unwrap()],
            relays: vec!["https://use1-1.relay.n0.iroh.link/".into()],
        }
    }
    #[test]
    fn salted_contact_preserves_the_unsalted_address_slot_and_network_scope() {
        let contact = contact();
        let salt = contact_salt(&contact.chain, contact.group);
        assert_eq!(salt.len(), 53);
        let item = contact.signed(&SecretKey::from_bytes(&[1; 32]), 9).unwrap();
        assert_eq!(
            Contact::verify_item(&item, &contact.node, &contact.chain, contact.group, 1100, 9)
                .unwrap(),
            contact
        );
        assert!(
            Contact::verify_item(
                &item,
                &contact.node,
                &contact.chain,
                contact.group,
                1100,
                10
            )
            .is_err()
        );
        assert_eq!(item.salt(), Some(salt.as_slice()));
        assert_ne!(
            item.target(),
            &MutableItem::target_from_key(contact.node.as_bytes(), None)
        );
        let bytes = contact.encode().unwrap();
        assert_eq!(
            Contact::decode(&bytes, &contact.chain, contact.group, 1100).unwrap(),
            contact
        );
        assert!(Contact::decode(&bytes, &[9; 32], contact.group, 1100).is_err());
        assert!(Contact::decode(&bytes, &contact.chain, contact.group + 1, 1100).is_err());
        let forged = MutableItem::new_signed_unchecked(
            *contact.node.as_bytes(),
            [0; 64],
            &bytes,
            10,
            Some(&salt),
        );
        assert!(
            Contact::verify_item(
                &forged,
                &contact.node,
                &contact.chain,
                contact.group,
                1100,
                9
            )
            .is_err()
        );
    }
    #[test]
    fn contacts_reject_expiry_truncation_private_hints_and_oversized_urls() {
        let mut contact = contact();
        let bytes = contact.encode().unwrap();
        for length in 0..bytes.len() {
            assert!(
                Contact::decode(&bytes[..length], &contact.chain, contact.group, 1100).is_err()
            );
        }
        assert!(Contact::decode(&bytes, &contact.chain, contact.group, 8200).is_err());
        contact.direct = vec!["192.168.1.2:1234".parse().unwrap()];
        assert!(contact.encode().is_err());
        contact.direct.clear();
        contact.relays = vec![format!("https://{}.test/", "x".repeat(129))];
        assert!(contact.encode().is_err());
    }
    #[test]
    fn chain_fingerprint_binds_identity_genesis_chain_id_and_group_salt() {
        let chain = chain_fingerprint(7781, &[1; 96], &[2; 32]);
        assert_ne!(chain, chain_fingerprint(7780, &[1; 96], &[2; 32]));
        assert_ne!(chain, chain_fingerprint(7781, &[2; 96], &[2; 32]));
        assert_ne!(chain, chain_fingerprint(7781, &[1; 96], &[3; 32]));
        assert_ne!(contact_salt(&chain, 0), contact_salt(&chain, 1));
    }
}
