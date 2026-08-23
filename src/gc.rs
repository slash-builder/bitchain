//! Garbage collection — mark-and-sweep, scoped to a single partition.
//!
//! **Mark:** walk every manifest under `<partition>/manifests/`, and for
//! each entry, walk its fragment-list tree (leaves + list-nodes) to compute
//! the live hash set.
//!
//! **Sweep + repack:** write every live hash into a fresh set of packs in a
//! staging area, then atomically swap the staged `packs/`/`active/`
//! directories in for the partition's real ones. Dead entries are never
//! copied forward — GC's job is exactly to not carry them into the new
//! packs.
//!
//! GC never crosses partitions: it only ever opens the one `PartitionStore`
//! it was asked to collect, and only ever touches that partition's
//! subdirectory on disk.

use crate::addressing::{Hash, PartitionId};
use crate::error::Result;
use crate::fragment::walk_hashes;
use crate::manifest::Manifest;
use crate::store::{PartitionStore, ReadBlock, WriteBlock};
use std::collections::HashSet;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Copy, Default)]
pub struct GcReport {
    pub live_count: usize,
    pub reclaimed_count: usize,
}

/// Run mark-and-sweep GC for `partition_id` under `store_root`.
pub fn collect(store_root: &Path, partition_id: PartitionId) -> Result<GcReport> {
    let mut store = PartitionStore::open(store_root, partition_id)?;
    // Fold any in-flight active pack into a sealed one first so the mark
    // phase and the repack below both see a stable, fully-indexed set.
    store.seal_active()?;

    let live = mark_live_set(&store)?;
    let total_before = store.all_hashes().len();

    // Repack live hashes into a staging store, then swap it in.
    let staging_root = store_root.join(format!(".gc-staging-{}", partition_id.to_hex()));
    if staging_root.exists() {
        fs::remove_dir_all(&staging_root)?;
    }
    fs::create_dir_all(&staging_root)?;
    {
        let mut staged = PartitionStore::open(&staging_root, partition_id)?;
        for hash in &live {
            let bytes = store.get(hash)?;
            let rewritten = staged.put(&bytes)?;
            debug_assert_eq!(rewritten, *hash, "content-addressed put must preserve hash");
        }
        staged.seal_active()?;
    }

    let real_partition_root = store.partition_root().to_path_buf();
    drop(store); // release file handles before swapping directories on disk

    let staged_partition_root = staging_root.join(partition_id.to_hex());
    swap_in_staged_partition(&real_partition_root, &staged_partition_root)?;
    fs::remove_dir_all(&staging_root).ok();

    Ok(GcReport {
        live_count: live.len(),
        reclaimed_count: total_before.saturating_sub(live.len()),
    })
}

/// Estimate `(live_count, total_count, utilization_pct)` for `partition_id`
/// without repacking anything — the read-only half of what `collect` does,
/// used by `gc --target-utilization` to decide whether a repack is even
/// worth doing before paying for one.
pub fn utilization(store_root: &Path, partition_id: PartitionId) -> Result<(usize, usize, f64)> {
    let store = PartitionStore::open(store_root, partition_id)?;
    let live = mark_live_set(&store)?;
    let total = store.all_hashes().len();
    let pct = if total == 0 {
        100.0
    } else {
        (live.len() as f64 / total as f64) * 100.0
    };
    Ok((live.len(), total, pct))
}

fn mark_live_set(store: &PartitionStore) -> Result<HashSet<Hash>> {
    let mut live = HashSet::new();
    for manifest_path in store.list_manifests()? {
        let json = fs::read_to_string(&manifest_path)?;
        let manifest = Manifest::from_json(&json)?;
        for entry in &manifest.entries {
            let mut collected = Vec::new();
            walk_hashes(entry.root_hash()?, entry.depth, store, &mut collected)?;
            live.extend(collected);
        }
    }
    Ok(live)
}

fn swap_in_staged_partition(real_root: &Path, staged_root: &Path) -> Result<()> {
    let real_packs = real_root.join("packs");
    let real_active = real_root.join("active");
    let staged_packs = staged_root.join("packs");
    let staged_active = staged_root.join("active");

    let backup_packs = real_root.join("packs.pre-gc");
    let backup_active = real_root.join("active.pre-gc");
    fs::remove_dir_all(&backup_packs).ok();
    fs::remove_dir_all(&backup_active).ok();

    if real_packs.exists() {
        fs::rename(&real_packs, &backup_packs)?;
    }
    if real_active.exists() {
        fs::rename(&real_active, &backup_active)?;
    }

    fs::create_dir_all(&staged_packs)?; // in case GC produced no packs at all
    fs::rename(&staged_packs, &real_packs)?;
    if staged_active.exists() {
        fs::rename(&staged_active, &real_active)?;
    } else {
        fs::create_dir_all(&real_active)?;
    }

    fs::remove_dir_all(&backup_packs).ok();
    fs::remove_dir_all(&backup_active).ok();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fragment::{build_fragment_tree, ChunkingProfile};
    use tempfile::tempdir;

    #[test]
    fn gc_reclaims_unreferenced_content_and_keeps_referenced_content() {
        let dir = tempdir().unwrap();
        let partition_id = PartitionId::derive("gc-test");

        let (kept_root, kept_depth, manifest_path);
        {
            let mut store = PartitionStore::open(dir.path(), partition_id).unwrap();
            let profile = ChunkingProfile::Fixed { block_size: 4096 };

            // Referenced by a manifest — must survive GC.
            let kept = build_fragment_tree(b"keep me alive", &profile, &mut store).unwrap();
            kept_root = kept.root.hash;
            kept_depth = kept.depth;

            let mut manifest = Manifest::new(partition_id);
            manifest.entries.push(crate::manifest::ManifestEntry {
                path: "kept.txt".into(),
                context: crate::addressing::Context::ANONYMOUS.to_hex(),
                root_hash: kept_root.to_hex(),
                root_type: kept.root_type,
                depth: kept.depth,
                uncompressed_len: kept.uncompressed_len,
                chunking_profile: profile.clone(),
            });
            manifest_path = store
                .write_manifest("kept", &manifest.to_json_pretty().unwrap())
                .unwrap();

            // Not referenced by any manifest — must be reclaimed by GC.
            let _orphan =
                build_fragment_tree(b"nobody references this", &profile, &mut store).unwrap();

            store.seal_active().unwrap();
        }
        assert!(manifest_path.exists());

        let report = collect(dir.path(), partition_id).unwrap();
        assert_eq!(
            report.reclaimed_count, 1,
            "exactly the orphan chunk should be reclaimed"
        );

        let store = PartitionStore::open(dir.path(), partition_id).unwrap();
        assert!(store.has(&kept_root), "referenced content must survive GC");
        let back = crate::fragment::read_fragment_tree(kept_root, kept_depth, &store).unwrap();
        assert_eq!(back, b"keep me alive");
    }

    #[test]
    fn gc_in_one_partition_never_touches_another() {
        let dir = tempdir().unwrap();
        let partition_a = PartitionId::derive("gc-isolation-a");
        let partition_b = PartitionId::derive("gc-isolation-b");

        let hash_b;
        {
            let mut store_b = PartitionStore::open(dir.path(), partition_b).unwrap();
            hash_b = store_b
                .put(b"partition b content, unreferenced by any manifest")
                .unwrap();
            store_b.seal_active().unwrap();
        }
        {
            let mut store_a = PartitionStore::open(dir.path(), partition_a).unwrap();
            let _ = store_a.put(b"partition a content").unwrap();
            store_a.seal_active().unwrap();
        }

        // GC partition A only. Partition B has no manifests either, but GC
        // must never even look at it, let alone reclaim from it.
        collect(dir.path(), partition_a).unwrap();

        let store_b = PartitionStore::open(dir.path(), partition_b).unwrap();
        assert!(
            store_b.has(&hash_b),
            "GC scoped to partition A must not reclaim partition B's content"
        );
    }
}
