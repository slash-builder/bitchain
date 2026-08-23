//! `(hash, context)` addressing — format v2, locked by data-architect (CLUS-20).
//!
//! Physical storage is keyed by **hash alone** (BLAKE3 of the stored bytes);
//! that is the only thing dedup and the packfile layer ever look at. `Context`
//! is the partition-scoped *logical identity* layered on top: it is what lets
//! two different logical objects that happen to hash to the same bytes be
//! distinguished (or, for fragmentation-internal nodes, explicitly declared
//! anonymous so identity collapses to content).
//!
//! Context is assigned once, at ingest, and is immutable thereafter — a new
//! logical identity is a new context, never a mutation of an old one.

use crate::error::{BitchainError, Result};
use std::fmt;

/// Length in bytes of a BLAKE3 content hash.
pub const HASH_LEN: usize = 32;
/// Length in bytes of a context.
pub const CONTEXT_LEN: usize = 16;
/// Length in bytes of a partition id.
pub const PARTITION_ID_LEN: usize = 16;

/// A BLAKE3 content hash. This is the sole key for physical block storage
/// and dedup within a partition.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Hash([u8; HASH_LEN]);

impl Hash {
    /// Hash `bytes` with BLAKE3. This defines block identity for the entire
    /// storage engine — never change the algorithm without a format version
    /// bump.
    pub fn of(bytes: &[u8]) -> Self {
        Hash(*blake3::hash(bytes).as_bytes())
    }

    pub fn from_bytes(bytes: [u8; HASH_LEN]) -> Self {
        Hash(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; HASH_LEN] {
        &self.0
    }

    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }

    pub fn from_hex(s: &str) -> Result<Self> {
        let bytes = hex::decode(s)
            .map_err(|e| BitchainError::InvalidAddress(format!("bad hash hex: {e}")))?;
        let arr: [u8; HASH_LEN] = bytes
            .try_into()
            .map_err(|_| BitchainError::InvalidAddress("hash must be 32 bytes".into()))?;
        Ok(Hash(arr))
    }
}

impl fmt::Debug for Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Hash({})", self.to_hex())
    }
}

impl fmt::Display for Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_hex())
    }
}

/// A 16-byte, partition-scoped logical identity. Canonical construction is
/// `BLAKE3(logical-identity-string)[0:16]`. The all-zero context is
/// reserved: it means "anonymous — identity is content", and is used for
/// fragmentation internal nodes (leaves and list-nodes), which have no
/// logical identity of their own.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Context([u8; CONTEXT_LEN]);

impl Context {
    /// Reserved anonymous context: `0x00..00`. Identity is content.
    pub const ANONYMOUS: Context = Context([0u8; CONTEXT_LEN]);

    /// Derive a canonical context from a logical-identity string:
    /// `BLAKE3(logical-identity-string)[0:16]`.
    pub fn derive(logical_identity: &str) -> Self {
        let full = blake3::hash(logical_identity.as_bytes());
        let mut out = [0u8; CONTEXT_LEN];
        out.copy_from_slice(&full.as_bytes()[..CONTEXT_LEN]);
        Context(out)
    }

    pub fn from_bytes(bytes: [u8; CONTEXT_LEN]) -> Self {
        Context(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; CONTEXT_LEN] {
        &self.0
    }

    pub fn is_anonymous(&self) -> bool {
        self.0 == [0u8; CONTEXT_LEN]
    }

    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }

    pub fn from_hex(s: &str) -> Result<Self> {
        let bytes = hex::decode(s)
            .map_err(|e| BitchainError::InvalidAddress(format!("bad context hex: {e}")))?;
        let arr: [u8; CONTEXT_LEN] = bytes
            .try_into()
            .map_err(|_| BitchainError::InvalidAddress("context must be 16 bytes".into()))?;
        Ok(Context(arr))
    }
}

impl fmt::Debug for Context {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_anonymous() {
            write!(f, "Context(ANONYMOUS)")
        } else {
            write!(f, "Context({})", self.to_hex())
        }
    }
}

impl Default for Context {
    fn default() -> Self {
        Context::ANONYMOUS
    }
}

/// A 16-byte partition id: the physical storage boundary. Canonical
/// construction is `BLAKE3(household/namespace-id)[0:16]`. No pack ever
/// spans partitions; dedup and GC are both scoped to a single partition.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PartitionId([u8; PARTITION_ID_LEN]);

impl PartitionId {
    /// Derive a canonical partition id from a household/namespace string.
    pub fn derive(namespace: &str) -> Self {
        let full = blake3::hash(namespace.as_bytes());
        let mut out = [0u8; PARTITION_ID_LEN];
        out.copy_from_slice(&full.as_bytes()[..PARTITION_ID_LEN]);
        PartitionId(out)
    }

    pub fn from_bytes(bytes: [u8; PARTITION_ID_LEN]) -> Self {
        PartitionId(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; PARTITION_ID_LEN] {
        &self.0
    }

    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }

    pub fn from_hex(s: &str) -> Result<Self> {
        let bytes = hex::decode(s)
            .map_err(|e| BitchainError::InvalidAddress(format!("bad partition id hex: {e}")))?;
        let arr: [u8; PARTITION_ID_LEN] = bytes
            .try_into()
            .map_err(|_| BitchainError::InvalidAddress("partition id must be 16 bytes".into()))?;
        Ok(PartitionId(arr))
    }

    /// The two-hex-char shard prefix used for the `packs/<prefix>/` fanout
    /// directory of the *pack id* (not the partition id itself) — kept here
    /// as a shared helper since both packfile and partition-store need the
    /// identical prefixing rule.
    pub fn shard_prefix(hex_id: &str) -> &str {
        &hex_id[..2.min(hex_id.len())]
    }
}

impl fmt::Debug for PartitionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PartitionId({})", self.to_hex())
    }
}

/// A full `(hash, context)` address: the logical identity of a stored
/// object within a partition.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Address {
    pub hash: Hash,
    pub context: Context,
}

impl Address {
    pub fn new(hash: Hash, context: Context) -> Self {
        Address { hash, context }
    }

    /// The reserved anonymous address for raw content-addressed bytes
    /// (fragmentation internal nodes).
    pub fn anonymous(hash: Hash) -> Self {
        Address {
            hash,
            context: Context::ANONYMOUS,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_roundtrips_through_hex() {
        let h = Hash::of(b"hello world");
        let hex = h.to_hex();
        let h2 = Hash::from_hex(&hex).unwrap();
        assert_eq!(h, h2);
    }

    #[test]
    fn context_derivation_is_deterministic() {
        let a = Context::derive("household:acme/user:dj");
        let b = Context::derive("household:acme/user:dj");
        let c = Context::derive("household:acme/user:someone-else");
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert!(!a.is_anonymous());
    }

    #[test]
    fn anonymous_context_is_all_zero() {
        assert!(Context::ANONYMOUS.is_anonymous());
        assert_eq!(Context::ANONYMOUS.as_bytes(), &[0u8; CONTEXT_LEN]);
    }

    #[test]
    fn partition_id_derivation_is_deterministic() {
        let a = PartitionId::derive("household:acme");
        let b = PartitionId::derive("household:acme");
        assert_eq!(a, b);
    }
}
