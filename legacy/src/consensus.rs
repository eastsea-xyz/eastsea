use crate::types::{Address, EncryptedTx, Hash256, Vertex};
use parking_lot::RwLock;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct DagEngine {
    validators: Vec<Address>,
    threshold_f: usize,
    vertices: Arc<RwLock<HashMap<Hash256, Vertex>>>,
    round_index: Arc<RwLock<HashMap<u64, Vec<Hash256>>>>,
    committed_vertices: Arc<RwLock<HashSet<Hash256>>>,
    current_round: Arc<RwLock<u64>>,
}

impl DagEngine {
    pub fn new(validators: Vec<Address>) -> Self {
        let n = validators.len();
        let f = (n.saturating_sub(1)) / 3;
        DagEngine {
            validators,
            threshold_f: f,
            vertices: Arc::new(RwLock::new(HashMap::new())),
            round_index: Arc::new(RwLock::new(HashMap::new())),
            committed_vertices: Arc::new(RwLock::new(HashSet::new())),
            current_round: Arc::new(RwLock::new(0)),
        }
    }

    pub fn quorum(&self) -> usize {
        2 * self.threshold_f + 1
    }

    pub fn get_parents_for_round(&self, round: u64) -> Vec<Hash256> {
        if round == 0 {
            return Vec::new();
        }
        let r_idx = self.round_index.read();
        r_idx.get(&(round - 1)).cloned().unwrap_or_default()
    }

    pub fn insert_vertex(&self, vertex: Vertex) -> bool {
        let h = vertex.hash;
        let round = vertex.round;

        {
            let mut v_map = self.vertices.write();
            if v_map.contains_key(&h) {
                return false;
            }
            v_map.insert(h, vertex);
        }

        {
            let mut r_map = self.round_index.write();
            r_map.entry(round).or_default().push(h);
        }

        {
            let mut cur = self.current_round.write();
            if round > *cur {
                *cur = round;
            }
        }
        true
    }

    pub fn get_all_vertices(&self) -> Vec<Vertex> {
        let v_map = self.vertices.read();
        v_map.values().cloned().collect()
    }

    pub fn get_current_round(&self) -> u64 {
        *self.current_round.read()
    }


    /// Elect an anchor for round `r`
    pub fn get_anchor_for_round(&self, round: u64) -> Option<Hash256> {
        if round == 0 {
            return None;
        }
        let leader_idx = (round as usize) % self.validators.len();
        let leader_addr = self.validators[leader_idx];

        let r_map = self.round_index.read();
        let v_map = self.vertices.read();
        if let Some(hashes) = r_map.get(&round) {
            for h in hashes {
                if let Some(v) = v_map.get(h) {
                    if v.author == leader_addr {
                        return Some(*h);
                    }
                }
            }
        }
        None
    }

    /// Check if anchor in round `r` has paths from at least quorum in round `r+1`
    pub fn try_commit_anchor(&self, anchor_hash: Hash256, anchor_round: u64) -> Option<Vec<EncryptedTx>> {
        let next_round = anchor_round + 1;
        let r_map = self.round_index.read();
        let next_vertices = match r_map.get(&next_round) {
            Some(v) if v.len() >= self.quorum() => v.clone(),
            _ => return None,
        };

        // Check reachability
        let v_map = self.vertices.read();
        let mut votes = 0;
        for nv_hash in &next_vertices {
            if let Some(nv) = v_map.get(nv_hash) {
                if nv.parents.contains(&anchor_hash) {
                    votes += 1;
                }
            }
        }

        if votes < self.quorum() {
            return None;
        }

        // Anchor is committed! Perform causal linearization via BFS/topological sort
        let mut committed = self.committed_vertices.write();
        if committed.contains(&anchor_hash) {
            return None;
        }

        let mut ordered_vertices = Vec::new();
        let mut queue = VecDeque::new();
        let mut visited = HashSet::new();

        queue.push_back(anchor_hash);
        visited.insert(anchor_hash);

        while let Some(curr) = queue.pop_front() {
            if committed.contains(&curr) {
                continue;
            }
            ordered_vertices.push(curr);
            if let Some(v) = v_map.get(&curr) {
                for p in &v.parents {
                    if !visited.contains(p) && !committed.contains(p) {
                        visited.insert(*p);
                        queue.push_back(*p);
                    }
                }
            }
        }

        // Reverse to get topological order: ancestors first, anchor last
        ordered_vertices.reverse();

        let mut ordered_txs = Vec::new();
        for v_hash in ordered_vertices {
            committed.insert(v_hash);
            if let Some(v) = v_map.get(&v_hash) {
                ordered_txs.extend(v.transactions.clone());
            }
        }

        Some(ordered_txs)
    }
}
