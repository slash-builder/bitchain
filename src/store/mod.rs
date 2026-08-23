//! Partition / packfile management — format v2.
//!
//! A `PartitionStore` owns exactly one partition's on-disk subtree:
//!
//! ```text
//! <store_root>/<partition_id_hex>/
//!   active/active.pack        — currently-open, appendable pack (no trailer)
//!   packs/<xx>/<pack_id>.pack — sealed, immutable, content-addressed packs
//!   packs/<xx>/<pack_id>.idx  — derived index, rebuildable, never authoritative
//!   manifests/*.json          — manifests ingested into this partition
//! ```
//!
//! No pack file, and no `PartitionStore`, ever spans more than one
//! partition — that's the physical boundary the whole GC and dedup story
//! relies on.

pub mod packfile;

use crate::addressing::{Hash, PartitionId};
use crate::error::{BitchainError, Result};
use packfile::PackWriter;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Soft cap: an active pack is sealed once it reaches this size.
pub const SEAL_CAP_BYTES: u64 = 128 * 1024 * 1024;

/// Enumerate every partition that already has an on-disk directory under
/// `store_root`. Used by CLI commands (`ls`, `gc`, `push`) that operate
/// "over every partition" when the caller doesn't name one explicitly — this
/// is directory discovery only, not partition creation (mkdir-on-first-write
/// stays the only place a partition directory is ever created).
pub fn list_partitions(store_root: &Path) -> Result<Vec<PartitionId>> {
    if !store_root.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in fs::read_dir(store_root)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if name.len() == 32 && name.chars().all(|c| c.is_ascii_hexdigit()) {
            out.push(PartitionId::from_hex(name)?);
        }
    }
    out.sort_by_key(|p| p.to_hex());
    Ok(out)
}

/// Read-only half of the storage contract: fetch bytes by content hash,
/// verifying identity on every read.
pub trait ReadBlock {
    fn get(&self, hash: &Hash) -> Result<Vec<u8>>;
    fn has(&self, hash: &Hash) -> bool;
}

/// Write half of the storage contract: append immutable content, keyed and
/// deduplicated by hash.
pub trait WriteBlock {
    /// Store `data`, returning its content hash. A `put` for content already
    /// present in this partition is a no-op dedup hit, not a duplicate
    /// write.
    fn put(&mut self, data: &[u8]) -> Result<Hash>;
}

/// The full storage contract a backend must satisfy: content-addressed,
/// partition-scoped, immutable once written.
pub trait ImmutableStore: ReadBlock + WriteBlock {
    fn partition_id(&self) -> PartitionId;
}

struct SealedPack {
    pack_id: Hash,
    path: PathBuf,
    /// hash -> byte offset of the entry within this pack.
    offsets: HashMap<Hash, u64>,
}

/// The reference `ImmutableStore` implementation: a single partition's
/// packfiles on the local filesystem.
pub struct PartitionStore {
    root: PathBuf,
    partition_id: PartitionId,
    active: PackWriter,
    sealed: Vec<SealedPack>,
    /// hash -> index into `sealed`, for O(1) dispatch on read.
    sealed_lookup: HashMap<Hash, usize>,
}

impl PartitionStore {
    /// Open (creating on first use) the store for `partition_id` under
    /// `store_root`. Rebuilds/self-heals any missing `.idx` files from
    /// their `.pack` counterparts — the index is derived, never trusted
    /// blindly.
    pub fn open(store_root: &Path, partition_id: PartitionId) -> Result<Self> {
        let root = store_root.join(partition_id.to_hex());
        let packs_dir = root.join("packs");
        let active_path = root.join("active").join("active.pack");
        fs::create_dir_all(&packs_dir)?;

        let active = PackWriter::open_active(&active_path, partition_id)?;

        let mut sealed = Vec::new();
        let mut sealed_lookup = HashMap::new();
        if packs_dir.exists() {
            for shard_entry in fs::read_dir(&packs_dir)? {
                let shard_entry = shard_entry?;
                if !shard_entry.file_type()?.is_dir() {
                    continue;
                }
                for pack_entry in fs::read_dir(shard_entry.path())? {
                    let pack_entry = pack_entry?;
                    let path = pack_entry.path();
                    if path.extension().and_then(|e| e.to_str()) != Some("pack") {
                        continue;
                    }
                    let pack_id_hex = path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .ok_or_else(|| BitchainError::CorruptPack("bad pack filename".into()))?
                        .to_string();
                    let pack_id = Hash::from_hex(&pack_id_hex)?;

                    let idx_path = path.with_extension("idx");
                    let offsets = match packfile::read_index(&idx_path) {
                        Ok(idx) => idx,
                        Err(_) => {
                            // Self-heal: rebuild from the pack and persist.
                            let entries = packfile::rebuild_index_entries(&path)?;
                            packfile::write_index(&idx_path, &partition_id, &entries)?;
                            entries.into_iter().map(|(off, m)| (m.hash, off)).collect()
                        }
                    };

                    let idx = sealed.len();
                    for hash in offsets.keys() {
                        sealed_lookup.insert(*hash, idx);
                    }
                    sealed.push(SealedPack {
                        pack_id,
                        path,
                        offsets,
                    });
                }
            }
        }

        Ok(PartitionStore {
            root,
            partition_id,
            active,
            sealed,
            sealed_lookup,
        })
    }

    pub fn partition_root(&self) -> &Path {
        &self.root
    }

    pub fn packs_dir(&self) -> PathBuf {
        self.root.join("packs")
    }

    pub fn manifests_dir(&self) -> PathBuf {
        self.root.join("manifests")
    }

    /// Persist `manifest_json` under this partition's `manifests/` tree so
    /// `gc` can discover it as a root to keep alive.
    pub fn write_manifest(&self, name: &str, manifest_json: &str) -> Result<PathBuf> {
        let dir = self.manifests_dir();
        fs::create_dir_all(&dir)?;
        let path = dir.join(format!("{name}.json"));
        fs::write(&path, manifest_json)?;
        Ok(path)
    }

    pub fn list_manifests(&self) -> Result<Vec<PathBuf>> {
        let dir = self.manifests_dir();
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            if entry.path().extension().and_then(|e| e.to_str()) == Some("json") {
                out.push(entry.path());
            }
        }
        out.sort();
        Ok(out)
    }

    /// List every entry hash currently reachable in this partition — both
    /// sealed packs and the active pack.
    pub fn all_hashes(&self) -> Vec<Hash> {
        let mut out: Vec<Hash> = self.active.offsets.keys().copied().collect();
        for pack in &self.sealed {
            out.extend(pack.offsets.keys().copied());
        }
        out
    }

    pub fn sealed_pack_ids(&self) -> Vec<Hash> {
        self.sealed.iter().map(|p| p.pack_id).collect()
    }

    /// Number of entries written into the still-open active pack (not yet
    /// sealed into an immutable `.pack`). Exposed for `ls` summaries.
    pub fn active_entry_count(&self) -> u64 {
        self.active.entry_count
    }

    /// Verify every sealed pack's trailer + every entry's hash across the
    /// whole partition (sealed and active). Returns the number of entries
    /// checked.
    pub fn verify_all(&self) -> Result<u64> {
        let mut checked = 0u64;
        for pack in &self.sealed {
            packfile::verify_trailer(&pack.path)?;
            for offset in pack.offsets.values() {
                packfile::get_at(&pack.path, *offset)?;
                checked += 1;
            }
        }
        // Active pack has no trailer yet; verify each entry by hash.
        for hash in self.active.offsets.keys() {
            self.get(hash)?;
            checked += 1;
        }
        Ok(checked)
    }

    fn seal_active_if_over_cap(&mut self) -> Result<()> {
        if self.active.len < SEAL_CAP_BYTES {
            return Ok(());
        }
        let fresh_active_path = self.root.join("active").join("active.pack");
        let old = std::mem::replace(
            &mut self.active,
            // Placeholder; immediately overwritten below. `open_active` on
            // a not-yet-existing path is cheap (just a header write).
            PackWriter::open_active(
                &fresh_active_path.with_extension("pack.next"),
                self.partition_id,
            )?,
        );
        let (pack_id, path) = old.seal(&self.packs_dir())?;
        let offsets = packfile::read_index(&path.with_extension("idx"))?;
        let idx = self.sealed.len();
        for hash in offsets.keys() {
            self.sealed_lookup.insert(*hash, idx);
        }
        self.sealed.push(SealedPack {
            pack_id,
            path,
            offsets,
        });
        // Replace the throwaway placeholder with the real fresh active pack.
        let _ = fs::remove_file(fresh_active_path.with_extension("pack.next"));
        self.active = PackWriter::open_active(&fresh_active_path, self.partition_id)?;
        Ok(())
    }

    /// Force-seal the current active pack even if it hasn't hit the soft
    /// cap. Used by `gc` (which needs a clean set of sealed packs to
    /// rewrite) and is otherwise optional — the 128 MiB cap is the normal
    /// trigger.
    pub fn seal_active(&mut self) -> Result<Option<Hash>> {
        if self.active.entry_count == 0 {
            return Ok(None);
        }
        let fresh_active_path = self.root.join("active").join("active.pack");
        let old = std::mem::replace(
            &mut self.active,
            PackWriter::open_active(
                &fresh_active_path.with_extension("pack.sealing"),
                self.partition_id,
            )?,
        );
        let (pack_id, path) = old.seal(&self.packs_dir())?;
        let offsets = packfile::read_index(&path.with_extension("idx"))?;
        let idx = self.sealed.len();
        for hash in offsets.keys() {
            self.sealed_lookup.insert(*hash, idx);
        }
        self.sealed.push(SealedPack {
            pack_id,
            path,
            offsets,
        });
        let _ = fs::remove_file(fresh_active_path.with_extension("pack.sealing"));
        self.active = PackWriter::open_active(&fresh_active_path, self.partition_id)?;
        Ok(Some(pack_id))
    }
}

impl ReadBlock for PartitionStore {
    fn get(&self, hash: &Hash) -> Result<Vec<u8>> {
        if let Some(offset) = self.active.offsets.get(hash) {
            let (meta, payload) = self.active.read_payload_at(*offset)?;
            let decoded = packfile::decode(meta.codec, &payload)?;
            let actual = Hash::of(&decoded);
            if actual != *hash {
                return Err(BitchainError::HashMismatch {
                    expected: hash.to_hex(),
                    actual: actual.to_hex(),
                });
            }
            return Ok(decoded);
        }
        if let Some(&idx) = self.sealed_lookup.get(hash) {
            let pack = &self.sealed[idx];
            let offset = *pack.offsets.get(hash).ok_or_else(|| {
                BitchainError::NotFound(format!("{hash} indexed but missing in pack"))
            })?;
            return packfile::get_at(&pack.path, offset);
        }
        Err(BitchainError::NotFound(hash.to_hex()))
    }

    fn has(&self, hash: &Hash) -> bool {
        self.active.offsets.contains_key(hash) || self.sealed_lookup.contains_key(hash)
    }
}

impl WriteBlock for PartitionStore {
    fn put(&mut self, data: &[u8]) -> Result<Hash> {
        let hash = Hash::of(data);
        if self.has(&hash) {
            return Ok(hash); // per-partition dedup, keyed by hash only
        }
        let (codec, payload) = packfile::encode(data)?;
        self.active
            .append(hash, codec, data.len() as u64, &payload)?;
        self.seal_active_if_over_cap()?;
        Ok(hash)
    }
}

impl ImmutableStore for PartitionStore {
    fn partition_id(&self) -> PartitionId {
        self.partition_id
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn put_dedups_within_partition() {
        let dir = tempdir().unwrap();
        let partition_id = PartitionId::derive("dedup-test");
        let mut store = PartitionStore::open(dir.path(), partition_id).unwrap();

        let h1 = store.put(b"same bytes").unwrap();
        let h2 = store.put(b"same bytes").unwrap();
        assert_eq!(h1, h2);
        assert_eq!(
            store.active.entry_count, 1,
            "second put must be a dedup no-op"
        );
    }

    #[test]
    fn put_then_get_roundtrips() {
        let dir = tempdir().unwrap();
        let partition_id = PartitionId::derive("roundtrip-test");
        let mut store = PartitionStore::open(dir.path(), partition_id).unwrap();
        let hash = store.put(b"round trip me").unwrap();
        let back = store.get(&hash).unwrap();
        assert_eq!(back, b"round trip me");
    }

    #[test]
    fn seal_then_reopen_still_reads() {
        let dir = tempdir().unwrap();
        let partition_id = PartitionId::derive("seal-reopen-test");
        let hash;
        {
            let mut store = PartitionStore::open(dir.path(), partition_id).unwrap();
            hash = store.put(b"persist across reopen").unwrap();
            store.seal_active().unwrap();
        }
        let store2 = PartitionStore::open(dir.path(), partition_id).unwrap();
        let back = store2.get(&hash).unwrap();
        assert_eq!(back, b"persist across reopen");
    }

    #[test]
    fn partitions_are_isolated_directories() {
        let dir = tempdir().unwrap();
        let a = PartitionId::derive("partition-a");
        let b = PartitionId::derive("partition-b");
        let mut store_a = PartitionStore::open(dir.path(), a).unwrap();
        let mut store_b = PartitionStore::open(dir.path(), b).unwrap();

        let hash_a = store_a.put(b"only in a").unwrap();
        let _hash_b = store_b.put(b"only in b").unwrap();

        assert!(store_a.has(&hash_a));
        assert!(!store_b.has(&hash_a));
        assert_ne!(store_a.partition_root(), store_b.partition_root());
    }
}
