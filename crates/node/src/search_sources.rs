//! Protocol source pins, published with the network description and RPC status.
//! These identify contract ABIs; they never select publishers or categories.
use aether_execution::WorldState;
use aether_types::{Address, B256};
use alloy_primitives::keccak256;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchSource {
    pub address: Address,
    pub code_hash: B256,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SearchSources {
    pub app_registries: Vec<SearchSource>,
    pub name_services: Vec<SearchSource>,
}

impl SearchSources {
    pub fn from_network(network: &Value) -> Result<Self, String> {
        let sources = match network.get("search") {
            None => Self::default(),
            Some(value) => {
                serde_json::from_value(value.clone()).map_err(|e| format!("network.search: {e}"))?
            }
        };
        sources.validated()
    }

    pub fn validated(mut self) -> Result<Self, String> {
        if self
            .app_registries
            .len()
            .saturating_add(self.name_services.len())
            > 64
        {
            return Err("network.search: at most 64 protocol sources".into());
        }
        for list in [&mut self.app_registries, &mut self.name_services] {
            if list
                .iter()
                .any(|source| source.address == Address::ZERO || source.code_hash == B256::ZERO)
            {
                return Err(
                    "network.search: source address and runtime code hash must be nonzero".into(),
                );
            }
            list.sort_by_key(|source| (source.address, source.code_hash));
            list.dedup();
            if list
                .windows(2)
                .any(|pair| pair[0].address == pair[1].address)
            {
                return Err("network.search: one runtime code hash per source address".into());
            }
        }
        if self.app_registries.iter().any(|source| {
            self.name_services
                .iter()
                .any(|other| other.address == source.address)
        }) {
            return Err("network.search: a source cannot be both registry protocols".into());
        }
        Ok(self)
    }

    pub fn configured(&self) -> bool {
        !self.app_registries.is_empty() || !self.name_services.is_empty()
    }

    pub fn fingerprint(&self) -> B256 {
        let normalized = self
            .clone()
            .validated()
            .expect("validated protocol source pins");
        keccak256(serde_json::to_vec(&normalized).expect("protocol pins serialize"))
    }

    pub fn allows_app(&self, address: Address, state: &WorldState) -> bool {
        Self::allows(&self.app_registries, address, state)
    }

    pub fn allows_name(&self, address: Address, state: &WorldState) -> bool {
        Self::allows(&self.name_services, address, state)
    }

    fn allows(sources: &[SearchSource], address: Address, state: &WorldState) -> bool {
        sources.iter().any(|source| {
            source.address == address && source.code_hash == state.code_hash(&address)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pins_are_visible_strict_and_order_independent() {
        let a = SearchSource {
            address: Address::repeat_byte(1),
            code_hash: B256::repeat_byte(2),
        };
        let b = SearchSource {
            address: Address::repeat_byte(3),
            code_hash: B256::repeat_byte(4),
        };
        let one = SearchSources {
            app_registries: vec![a.clone(), b.clone()],
            name_services: vec![],
        };
        let two = SearchSources {
            app_registries: vec![b, a.clone(), a],
            name_services: vec![],
        };
        assert_eq!(one.fingerprint(), two.fingerprint());
        assert!(!SearchSources::from_network(&serde_json::json!({}))
            .unwrap()
            .configured());
        assert!(SearchSources::from_network(&serde_json::json!({"search":{"boost":1}})).is_err());
    }
}
