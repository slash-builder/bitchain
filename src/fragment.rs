//! Recursive fragmentation — format v2, locked by data-architect (CLUS-20).
//!
//! Content is split into **leaves** (raw chunk bytes) by a configurable
//! chunking profile (FastCDC or fixed-size), then leaves are folded
//! bottom-up into **list-nodes** at a fixed 1024-child fanout until a
//! single root remains. Both leaves and list-nodes are stored as ordinary
//! content-addressed blocks under the reserved anonymous context — a
//! fragmentation-internal node has no logical identity beyond its bytes.
//!
//! A list-node's body is nothing but its children's refs, back to back:
//! `hash (32B) | uncompressed_len (8B)` per child, in order. Reconstruction
//! needs to know how many list-node levels sit above the leaves for a given
//! root — the builder always produces a depth-uniform tree (every leaf at
//! the same depth), so a single `depth` counter alongside `root_type`
//! disambiguates every level unambiguously. This is one field beyond the
//! spec's literal `root_type: leaf|list`, added because reconstruction is
//! otherwise ambiguous; it's additive, not a deviation from the locked
//! wire format for leaves/list-nodes themselves.

use crate::addressing::{Address, Context, Hash, HASH_LEN};
use crate::error::{BitchainError, Result};
use crate::store::{ReadBlock, WriteBlock};
use fastcdc::v2020::FastCDC;
use serde::{Deserialize, Serialize};

/// Fixed child fanout for list-nodes — a determinism lock, not a tunable.
pub const FANOUT: usize = 1024;

/// Size in bytes of one child ref within a list-node body.
pub const CHILD_REF_LEN: usize = HASH_LEN + 8;

/// How raw content is split into leaves. Recorded in the manifest so any
/// implementation reconstructing the same bytes from the same profile
/// reaches the same root hash.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "codec", rename_all = "snake_case")]
pub enum ChunkingProfile {
    /// Fixed-size chunking: every chunk is `block_size` bytes except
    /// possibly the last.
    Fixed { block_size: u64 },
    /// FastCDC content-defined chunking.
    FastCdc {
        min_size: u32,
        avg_size: u32,
        max_size: u32,
    },
}

impl Default for ChunkingProfile {
    fn default() -> Self {
        ChunkingProfile::FastCdc {
            min_size: 4 * 1024,
            avg_size: 16 * 1024,
            max_size: 64 * 1024,
        }
    }
}

/// Whether a fragment tree's root is a raw leaf (content fit in one chunk)
/// or a list-node (one or more fold levels above the leaves).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RootType {
    Leaf,
    List,
}

/// The result of fragmenting a byte slice into the store.
#[derive(Debug, Clone, Copy)]
pub struct FragmentTree {
    pub root: Address,
    pub root_type: RootType,
    /// Number of list-node levels above the leaves (0 for a bare leaf).
    pub depth: u32,
    pub uncompressed_len: u64,
}

fn chunk_offsets(data: &[u8], profile: &ChunkingProfile) -> Vec<(usize, usize)> {
    if data.is_empty() {
        return Vec::new();
    }
    match profile {
        ChunkingProfile::Fixed { block_size } => {
            let block_size = (*block_size).max(1) as usize;
            data.chunks(block_size)
                .scan(0usize, |pos, c| {
                    let start = *pos;
                    *pos += c.len();
                    Some((start, c.len()))
                })
                .collect()
        }
        ChunkingProfile::FastCdc {
            min_size,
            avg_size,
            max_size,
        } => FastCDC::new(data, *min_size, *avg_size, *max_size)
            .map(|c| (c.offset, c.length))
            .collect(),
    }
}

/// Split `data` into leaves per `profile`, store each leaf, then fold
/// leaves into list-nodes at `FANOUT` until a single root remains.
pub fn build_fragment_tree(
    data: &[u8],
    profile: &ChunkingProfile,
    store: &mut dyn WriteBlock,
) -> Result<FragmentTree> {
    let offsets = chunk_offsets(data, profile);

    if offsets.is_empty() {
        let hash = store.put(&[])?;
        return Ok(FragmentTree {
            root: Address::anonymous(hash),
            root_type: RootType::Leaf,
            depth: 0,
            uncompressed_len: 0,
        });
    }

    let mut level: Vec<(Hash, u64)> = Vec::with_capacity(offsets.len());
    for (start, len) in &offsets {
        let chunk = &data[*start..*start + *len];
        let hash = store.put(chunk)?;
        level.push((hash, *len as u64));
    }

    let mut depth = 0u32;
    let mut root_type = RootType::Leaf;
    while level.len() > 1 {
        root_type = RootType::List;
        depth += 1;
        let mut next = Vec::with_capacity(level.len().div_ceil(FANOUT));
        for group in level.chunks(FANOUT) {
            let mut body = Vec::with_capacity(group.len() * CHILD_REF_LEN);
            let mut total = 0u64;
            for (hash, len) in group {
                body.extend_from_slice(hash.as_bytes());
                body.extend_from_slice(&len.to_le_bytes());
                total += len;
            }
            let hash = store.put(&body)?;
            next.push((hash, total));
        }
        level = next;
    }

    let (root_hash, total_len) = level[0];
    Ok(FragmentTree {
        root: Address::anonymous(root_hash),
        root_type,
        depth,
        uncompressed_len: total_len,
    })
}

/// Reconstruct the original bytes for a fragment tree rooted at `root` with
/// the given `depth` (list-node levels above the leaves).
pub fn read_fragment_tree(root: Hash, depth: u32, store: &dyn ReadBlock) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    read_recursive(root, depth, store, &mut out)?;
    Ok(out)
}

fn read_recursive(hash: Hash, depth: u32, store: &dyn ReadBlock, out: &mut Vec<u8>) -> Result<()> {
    let bytes = store.get(&hash)?;
    if depth == 0 {
        out.extend_from_slice(&bytes);
        return Ok(());
    }
    if bytes.len() % CHILD_REF_LEN != 0 {
        return Err(BitchainError::CorruptManifest(format!(
            "list-node {hash} body length {} is not a multiple of {CHILD_REF_LEN}",
            bytes.len()
        )));
    }
    for record in bytes.chunks(CHILD_REF_LEN) {
        let child_hash = Hash::from_bytes(record[..HASH_LEN].try_into().unwrap());
        read_recursive(child_hash, depth - 1, store, out)?;
    }
    Ok(())
}

/// Walk a fragment tree without materializing content, collecting every
/// hash reached (leaves and list-nodes). Used by GC's mark phase.
pub fn walk_hashes(
    hash: Hash,
    depth: u32,
    store: &dyn ReadBlock,
    live: &mut Vec<Hash>,
) -> Result<()> {
    live.push(hash);
    if depth == 0 {
        return Ok(());
    }
    let bytes = store.get(&hash)?;
    if bytes.len() % CHILD_REF_LEN != 0 {
        return Err(BitchainError::CorruptManifest(format!(
            "list-node {hash} body length {} is not a multiple of {CHILD_REF_LEN}",
            bytes.len()
        )));
    }
    for record in bytes.chunks(CHILD_REF_LEN) {
        let child_hash = Hash::from_bytes(record[..HASH_LEN].try_into().unwrap());
        walk_hashes(child_hash, depth - 1, store, live)?;
    }
    Ok(())
}

/// A context is never assigned to a leaf or list-node — fragmentation
/// internal nodes are always anonymous. Exposed as a helper so callers
/// never have to hand-roll `Context::ANONYMOUS` at call sites.
pub fn internal_node_context() -> Context {
    Context::ANONYMOUS
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::addressing::PartitionId;
    use crate::store::PartitionStore;
    use tempfile::tempdir;

    fn store_for(name: &str) -> (tempfile::TempDir, PartitionStore) {
        let dir = tempdir().unwrap();
        let partition_id = PartitionId::derive(name);
        let store = PartitionStore::open(dir.path(), partition_id).unwrap();
        (dir, store)
    }

    #[test]
    fn empty_input_is_a_leaf() {
        let (_dir, mut store) = store_for("fragment-empty");
        let tree = build_fragment_tree(&[], &ChunkingProfile::default(), &mut store).unwrap();
        assert_eq!(tree.root_type, RootType::Leaf);
        assert_eq!(tree.depth, 0);
        assert_eq!(tree.uncompressed_len, 0);
    }

    #[test]
    fn small_input_is_a_single_leaf() {
        let (_dir, mut store) = store_for("fragment-small");
        let data = b"small enough to fit in one chunk";
        let profile = ChunkingProfile::Fixed { block_size: 4096 };
        let tree = build_fragment_tree(data, &profile, &mut store).unwrap();
        assert_eq!(tree.root_type, RootType::Leaf);
        assert_eq!(tree.depth, 0);
        assert_eq!(tree.uncompressed_len, data.len() as u64);

        let back = read_fragment_tree(tree.root.hash, tree.depth, &store).unwrap();
        assert_eq!(back, data);
    }

    #[test]
    fn multi_chunk_input_roundtrips_through_a_list_node() {
        let (_dir, mut store) = store_for("fragment-multi");
        let data: Vec<u8> = (0..50_000u32).flat_map(|n| n.to_le_bytes()).collect();
        let profile = ChunkingProfile::Fixed { block_size: 4096 };
        let tree = build_fragment_tree(&data, &profile, &mut store).unwrap();
        assert_eq!(tree.root_type, RootType::List);
        assert_eq!(tree.uncompressed_len, data.len() as u64);

        let back = read_fragment_tree(tree.root.hash, tree.depth, &store).unwrap();
        assert_eq!(back, data);
    }

    #[test]
    fn deep_fanout_forces_multiple_list_levels() {
        // FANOUT is 1024; force > 1024 leaves so a second fold level is
        // required, proving `depth` tracks real tree height correctly.
        let (_dir, mut store) = store_for("fragment-deep");
        let block_size = 16u64;
        let data = vec![7u8; (block_size as usize) * (FANOUT + 5)];
        let profile = ChunkingProfile::Fixed { block_size };
        let tree = build_fragment_tree(&data, &profile, &mut store).unwrap();
        assert_eq!(tree.depth, 2, "expected two fold levels above the leaves");

        let back = read_fragment_tree(tree.root.hash, tree.depth, &store).unwrap();
        assert_eq!(back, data);
    }

    #[test]
    fn identical_bytes_same_profile_yield_identical_root_hash() {
        let data = b"determinism is the whole point of content addressing".repeat(200);
        let profile = ChunkingProfile::default();

        let (_dir_a, mut store_a) = store_for("determinism-a");
        let tree_a = build_fragment_tree(&data, &profile, &mut store_a).unwrap();

        let (_dir_b, mut store_b) = store_for("determinism-b");
        let tree_b = build_fragment_tree(&data, &profile, &mut store_b).unwrap();

        assert_eq!(tree_a.root.hash, tree_b.root.hash);
        assert_eq!(tree_a.depth, tree_b.depth);
        assert_eq!(tree_a.root_type, tree_b.root_type);
    }
}
