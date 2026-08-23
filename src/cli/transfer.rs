//! Shared copy logic for `push`/`pull`. Copies are always scoped to a
//! single partition on both sides — the partition boundary is a filesystem
//! subtree, and this never reaches across it.

use bitchain::PartitionId;
use std::fs;
use std::path::Path;

/// Copy sealed `.pack`/`.idx` pairs for `partition_id` from
/// `<src_root>/<partition>/packs/` to `<dst_root>/<partition>/packs/`.
/// `only` restricts the copy to those pack-id hex strings; empty means
/// "everything sealed".
pub fn copy_packs(
    src_root: &Path,
    dst_root: &Path,
    partition_id: PartitionId,
    only: &[String],
) -> bitchain::Result<usize> {
    let src_packs = src_root.join(partition_id.to_hex()).join("packs");
    let dst_packs = dst_root.join(partition_id.to_hex()).join("packs");
    if !src_packs.exists() {
        return Ok(0);
    }

    let mut copied = 0usize;
    for shard_entry in fs::read_dir(&src_packs)? {
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
            let pack_id = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .to_string();
            if !only.is_empty() && !only.contains(&pack_id) {
                continue;
            }

            let dest_shard_dir = dst_packs.join(PartitionId::shard_prefix(&pack_id));
            fs::create_dir_all(&dest_shard_dir)?;
            fs::copy(&path, dest_shard_dir.join(format!("{pack_id}.pack")))?;
            let idx_path = path.with_extension("idx");
            if idx_path.exists() {
                fs::copy(&idx_path, dest_shard_dir.join(format!("{pack_id}.idx")))?;
            }
            copied += 1;
        }
    }
    Ok(copied)
}

/// Copy every manifest for `partition_id` from `<src_root>/<partition>/manifests/`
/// to `<dst_root>/<partition>/manifests/`, so a `pull`ed store can resolve
/// `unpack <hash>` the same way the source could, and `gc` on the
/// destination sees the same live roots.
pub fn copy_manifests(
    src_root: &Path,
    dst_root: &Path,
    partition_id: PartitionId,
) -> bitchain::Result<usize> {
    let src_manifests = src_root.join(partition_id.to_hex()).join("manifests");
    if !src_manifests.exists() {
        return Ok(0);
    }
    let dst_manifests = dst_root.join(partition_id.to_hex()).join("manifests");
    fs::create_dir_all(&dst_manifests)?;

    let mut copied = 0usize;
    for entry in fs::read_dir(&src_manifests)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        if let Some(name) = path.file_name() {
            fs::copy(&path, dst_manifests.join(name))?;
            copied += 1;
        }
    }
    Ok(copied)
}
