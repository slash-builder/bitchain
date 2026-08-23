//! `bitchain unpack <hash> <output_path>` — resolve `<hash>` to a manifest
//! (content-addressed the same way fragments are: `manifests/<hash>.json`
//! under a partition), then walk each entry's fragment tree back into
//! bytes under `<output_path>` (always treated as a directory: entries land
//! at `<output_path>/<entry.path>`, so single- and multi-file manifests
//! reconstruct the same way).

use bitchain::fragment::read_fragment_tree;
use bitchain::{Manifest, PartitionStore};
use std::path::PathBuf;

pub struct UnpackArgs {
    /// The manifest's content hash (64 hex chars), as printed by `pack`.
    /// A path to a manifest JSON file is also accepted, for working
    /// directly off a manifest you already have on hand.
    pub manifest: String,
    pub store_root: PathBuf,
    /// Defaults to the same "default" partition `pack` uses when
    /// `--partition` is omitted.
    pub partition: Option<String>,
    pub output_dir: PathBuf,
}

fn load_manifest_json(args: &UnpackArgs) -> bitchain::Result<String> {
    let looks_like_hash =
        args.manifest.len() == 64 && args.manifest.chars().all(|c| c.is_ascii_hexdigit());
    if looks_like_hash {
        // Same implicit "default" partition `pack` uses when `--partition`
        // is omitted, so `bitchain pack x && bitchain unpack <hash> out`
        // works with zero required flags.
        let partition_raw = args.partition.as_deref().unwrap_or("default");
        let partition_id = super::resolve_partition(partition_raw)?;
        let path = args
            .store_root
            .join(partition_id.to_hex())
            .join("manifests")
            .join(format!("{}.json", args.manifest));
        std::fs::read_to_string(&path).map_err(|e| {
            bitchain::BitchainError::NotFound(format!(
                "manifest {} not found in partition {} ({}): {e}",
                args.manifest,
                partition_id.to_hex(),
                path.display()
            ))
        })
    } else {
        Ok(std::fs::read_to_string(&args.manifest)?)
    }
}

/// Reconstruct every entry in the manifest resolved from `args.manifest`,
/// returning the number of files written.
pub fn run(args: UnpackArgs) -> bitchain::Result<usize> {
    let json = load_manifest_json(&args)?;
    let manifest = Manifest::from_json(&json)?;
    let partition_id = manifest.partition_id()?;
    let store = PartitionStore::open(&args.store_root, partition_id)?;

    std::fs::create_dir_all(&args.output_dir)?;
    let mut restored = 0usize;
    for entry in &manifest.entries {
        let root_hash = entry.root_hash()?;
        let data = read_fragment_tree(root_hash, entry.depth, &store)?;
        if data.len() as u64 != entry.uncompressed_len {
            return Err(bitchain::BitchainError::CorruptManifest(format!(
                "reconstructed length mismatch for {}: expected {}, got {}",
                entry.path,
                entry.uncompressed_len,
                data.len()
            )));
        }

        let target = args.output_dir.join(&entry.path);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&target, &data)?;
        eprintln!("restored {} ({} bytes)", target.display(), data.len());
        restored += 1;
    }
    Ok(restored)
}
