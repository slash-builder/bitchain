//! Garbage collection — mark-and-sweep, scoped to a single partition.
//!
//! **Mark:** walk every manifest under `<partition>/manifests/`, and for
//! each entry, walk its fragment-list tree (leaves + list-nodes) to compute
//! the live hash set.
//!
//! **Sweep + repack:** copy every live entry's `(meta, payload)` byte-for-
//! byte into a fresh set of packs in a staging area, then atomically swap
//! the staged `packs/`/`active/` directories in for the partition's real
//! ones. Dead entries are never copied forward — GC's job is exactly to
//! not carry them into the new packs.
//!
//! **Meta-preserving, deliberately (2026-09-23 storage-addressing
//! rulings).** The repack loop used to be `let bytes = store.get(hash)?;
//! staged.put(&bytes)?` — a full plaintext decode-then-re-encode round
//! trip through storage-kit's validated, format-agnostic API. That path
//! reconstructs `EntryMeta` from scratch on the way back in, so the day
//! `key_epoch` or `flags` carries real meaning, an ordinary GC run would
//! silently re-stamp every entry's `key_epoch` to the staging store's
//! current epoch and every entry's `flags` to the default, with no error
//! and every hash still verifying — the 2026-09-22 format-reservations spec
//! calls this out as a mandatory part of the v3 work, not a future nicety
//! (§"Addition — the finding that changes the implementation order"). This
//! module instead uses `PartitionStore::get_raw`/`put_raw`, the byte-exact
//! primitives storage-kit added for exactly this job: GC relocates an
//! entry's `(meta, payload)` verbatim, without decoding or interpreting
//! what the meta means. That keeps GC key-blind under D-2 exactly as it
//! does today, and it is the property that lets it stay key-blind once
//! encryption lands — a key-holding re-encryption pass is a deliberately
//! separate, opt-in operation with a different name, not a variant of this
//! one.
//!
//! GC never crosses partitions: it only ever opens the one `PartitionStore`
//! it was asked to collect, and only ever touches that partition's
//! subdirectory on disk.

use crate::addressing::{Hash, PartitionId};
use crate::error::Result;
use crate::fragment::walk_hashes;
use crate::manifest::Manifest;
use crate::store::PartitionStore;
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
    // v3 split "create" from "open": `PartitionStore::open` no longer
    // provisions a partition on first use (spec §3.7), and `create` needs a
    // `Subject` to re-derive `partition_id` from — which GC does not have
    // and structurally cannot reconstruct for an arbitrary partition it is
    // asked to collect. `copy_partition_record` is the provisioning path
    // that requires no `Subject`: it carries the *real* partition's own
    // record (retention_class, encryption_mode, key state) into the
    // staging store verbatim, so the swapped-in result is provisioned
    // under the same terms as the partition GC is replacing rather than
    // under some reconstructed default. It also fails closed if a
    // mismatched record already exists at the destination, the same
    // anti-laundering check a real replication push gets.
    storage_kit::copy_partition_record(store_root, &staging_root, partition_id)?;
    {
        let mut staged = PartitionStore::open(&staging_root, partition_id)?;
        for hash in &live {
            // Byte-exact, meta-preserving copy — see the module docs. Never
            // `store.get()`/`staged.put()`, which would decode and
            // re-derive `EntryMeta` from scratch.
            let (meta, payload) = store.get_raw(hash)?;
            let rewritten = staged.put_raw(meta, &payload)?;
            debug_assert_eq!(rewritten, *hash, "meta-preserving copy must preserve hash");
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
    use crate::store::ImmutableStore;
    use std::path::Path;
    use storage_kit::{EncryptionMode, PartitionSpec, RetentionClass, Subject, SubjectKind};
    use tempfile::tempdir;

    /// Provision a fresh partition for `namespace` and open it.
    ///
    /// v3 split `create` from `open` (spec §3.7): `open` no longer
    /// provisions on first use, and `create` requires a `Subject` whose
    /// domain-prefixed derivation matches `partition_id` (`verify_derivation`
    /// in storage-kit's `partition.rs`), which a bare `PartitionId::derive`
    /// cannot supply. GC itself never provisions a fresh partition — it only
    /// ever collects one `pack`/CLI usage already created (see `collect`'s
    /// own `copy_partition_record` for how *its* staging store is
    /// provisioned without a `Subject`) — so this constructor is test-only
    /// scaffolding standing in for that upstream provisioning step, not
    /// something GC's own code needs.
    fn create_test_partition(store_root: &Path, namespace: &str) -> (PartitionId, PartitionStore) {
        let partition_id = PartitionId::for_service(namespace);
        let spec = PartitionSpec {
            subject: Subject {
                kind: SubjectKind::Service,
                id: namespace.to_string(),
            },
            retention_class: RetentionClass::Standard,
            encryption_mode: EncryptionMode::Plaintext,
            label: None,
        };
        let store = PartitionStore::create(store_root, partition_id, spec).unwrap();
        (partition_id, store)
    }

    #[test]
    fn gc_reclaims_unreferenced_content_and_keeps_referenced_content() {
        let dir = tempdir().unwrap();

        let (kept_root, kept_depth, manifest_path, partition_id);
        {
            let (id, mut store) = create_test_partition(dir.path(), "gc-test");
            partition_id = id;
            let profile = ChunkingProfile::Fixed { block_size: 4096 };

            // Referenced by a manifest — must survive GC.
            let kept = build_fragment_tree(b"keep me alive", &profile, &mut store).unwrap();
            kept_root = kept.root;
            kept_depth = kept.depth;

            let mut manifest = Manifest::new(partition_id);
            manifest.entries.push(crate::manifest::ManifestEntry {
                path: "kept.txt".into(),
                context: crate::context::Context::ANONYMOUS.to_hex(),
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

        let (partition_a, partition_b, hash_b);
        {
            let (id_b, mut store_b) = create_test_partition(dir.path(), "gc-isolation-b");
            partition_b = id_b;
            hash_b = store_b
                .put(b"partition b content, unreferenced by any manifest")
                .unwrap();
            store_b.seal_active().unwrap();
        }
        {
            let (id_a, mut store_a) = create_test_partition(dir.path(), "gc-isolation-a");
            partition_a = id_a;
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

    /// The single most important test in this change (2026-09-23
    /// storage-addressing rulings). Writes an entry carrying **non-default**
    /// `EntryMeta` — `key_epoch`/`flags` a real writer never produces today,
    /// via `PartitionStore::put_raw` (storage-kit's byte-exact primitive;
    /// there is no way to do this through the normal, validated `put()`
    /// path, by design) — runs a real `collect()`, and asserts the meta
    /// survived byte-for-byte. This fails against the old
    /// `store.get()`/`staged.put()` repack, which would silently re-stamp
    /// both fields to their defaults with no error and the hash still
    /// verifying.
    #[test]
    fn gc_preserves_non_default_entry_meta_through_a_collect() {
        let dir = tempdir().unwrap();

        let data = b"entry carrying non-default meta, must survive a collect byte-for-byte";
        let (partition_id, hash, manifest_path, injected_meta);
        {
            let (id, mut store) = create_test_partition(dir.path(), "gc-meta-preserving-test");
            partition_id = id;
            let (codec, payload) = storage_kit::packfile::encode(data).unwrap();
            // Non-default on both fields a real writer never sets today —
            // proving the copy is meta-preserving, not merely hash-preserving.
            let meta = storage_kit::packfile::EntryMeta {
                hash: storage_kit::Hash::of(data),
                codec,
                flags: 0x01,
                key_epoch: 7,
                uncompressed_len: data.len() as u64,
                stored_len: payload.len() as u64,
            };
            hash = meta.hash;
            injected_meta = meta;
            store.put_raw(meta, &payload).unwrap();

            // Referenced by a manifest as a bare leaf (depth 0) — GC's mark
            // phase (`walk_hashes`) never calls `get()` on a depth-0 leaf,
            // so this entry's non-reserved meta never touches the
            // validated read path during mark, only during the raw
            // byte-copy sweep this test exists to prove.
            let mut manifest = Manifest::new(partition_id);
            manifest.entries.push(crate::manifest::ManifestEntry {
                path: "meta.bin".into(),
                context: crate::context::Context::ANONYMOUS.to_hex(),
                root_hash: hash.to_hex(),
                root_type: crate::fragment::RootType::Leaf,
                depth: 0,
                uncompressed_len: data.len() as u64,
                chunking_profile: ChunkingProfile::Fixed { block_size: 4096 },
            });
            manifest_path = store
                .write_manifest("meta", &manifest.to_json_pretty().unwrap())
                .unwrap();
            store.seal_active().unwrap();
        }
        assert!(manifest_path.exists());

        let report = collect(dir.path(), partition_id).unwrap();
        assert_eq!(report.live_count, 1);
        assert_eq!(report.reclaimed_count, 0);

        let store = PartitionStore::open(dir.path(), partition_id).unwrap();
        let (meta_after, payload_after) = store.get_raw(&hash).unwrap();
        assert_eq!(
            meta_after, injected_meta,
            "collect() must preserve entry meta byte-for-byte, not re-derive it"
        );
        let decoded = storage_kit::packfile::decode(meta_after.codec, &payload_after).unwrap();
        assert_eq!(decoded, data, "payload bytes must also survive unchanged");
    }
}
