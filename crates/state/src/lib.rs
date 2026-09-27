//! State layer (docs/design/05-state.md).
//!
//! `tree`   — EIP-7864 unified binary tree over 31-byte stems with 256-leaf
//!            stem subtrees, generic over the `Hasher`; inclusion and absence proofs.
//! `layout` — EIP-7864 key derivation and account encoding (basic data, code
//!            hash, storage slots, code chunks).
//! `mmr`    — Merkle Mountain Range over block hashes (history root in every block).
//! `witness` — stateless witness: the part of the tree a block touches, enough
//!            to check the pre-state root and compute the post-state root.
//! `repo`   — `StateRepository` trait and the in-memory reference implementation
//!            that differential tests compare persistent engines against.

pub mod layout;
pub mod mmr;
pub mod repo;
pub mod tree;
pub mod witness;

pub use repo::{MemRepo, StateRepository};
pub use tree::{BinaryTree, Proof, ProofError, Stem, TreeKey, Value};
pub use witness::{PartialTree, Witness, WitnessError};
