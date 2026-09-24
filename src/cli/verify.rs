//! `bitchain verify [<hash>]` — re-hash content and confirm determinism.
//!
//! With a `<hash>`: resolve it to a manifest (same lookup `unpack` uses),
//! re-derive the manifest's own content hash, and for every entry:
//! reconstruct its bytes, then **re-chunk and re-hash them from scratch**
//! with the entry's recorded `chunking_profile` and compare the resulting
//! `(root_hash, root_type, depth)` against what the manifest recorded. This
//! is a determinism check, not a bare content-hash comparison — for a
//! multi-chunk entry, `root_hash` addresses the top list-node, not
//! `BLAKE3(reconstructed_bytes)`, so a straight hash-of-bytes comparison
//! would be comparing two different things for anything past a single
//! chunk.
//!
//! Without a `<hash>`: verify every sealed pack's trailer + every entry's
//! hash in the partition (or every partition, with `--all`), plus the same
//! per-manifest determinism check for every manifest found there.

use bitchain::fragment::{build_fragment_tree, read_fragment_tree};
use bitchain::{Hash, Manifest, PartitionId, PartitionStore};
use std::path::PathBuf;

pub struct VerifyArgs {
    pub store_root: PathBuf,
    pub hash: Option<String>,
    pub partition: String,
    pub all_partitions: bool,
}

#[derive(Default)]
pub struct VerifyReport {
    pub manifests_checked: usize,
    pub manifests_passed: usize,
    pub entries_checked: usize,
    pub pack_entries_checked: u64,
    pub failures: Vec<String>,
}

impl VerifyReport {
    pub fn passed(&self) -> bool {
        self.failures.is_empty()
    }
}

pub fn run(args: VerifyArgs) -> bitchain::Result<VerifyReport> {
    let mut report = VerifyReport::default();

    let partitions: Vec<PartitionId> = if let Some(hash) = &args.hash {
        Hash::from_hex(hash)?; // fail fast on a malformed hash
        if args.all_partitions {
            bitchain::store::list_partitions(&args.store_root)?
        } else {
            vec![super::resolve_partition(&args.partition)?]
        }
    } else if args.all_partitions {
        bitchain::store::list_partitions(&args.store_root)?
    } else {
        vec![super::resolve_partition(&args.partition)?]
    };

    for partition_id in partitions {
        // Mutable: re-verification re-chunks reconstructed bytes through
        // `build_fragment_tree`, which writes through `WriteBlock::put` —
        // always a dedup no-op here since the content already exists, but
        // the trait requires `&mut`.
        let mut store = PartitionStore::open(&args.store_root, partition_id)?;
        report.pack_entries_checked += store.verify_all()?;

        let manifest_paths: Vec<PathBuf> = if let Some(hash) = &args.hash {
            let path = store.manifests_dir().join(format!("{hash}.json"));
            if path.exists() {
                vec![path]
            } else {
                continue;
            }
        } else {
            store.list_manifests()?
        };

        for path in manifest_paths {
            let json = std::fs::read_to_string(&path)?;
            let manifest = Manifest::from_json(&json)?;
            report.manifests_checked += 1;

            let mut ok = true;
            let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
            let recomputed = manifest.content_hash()?.to_hex();
            if stem != recomputed {
                ok = false;
                report.failures.push(format!(
                    "{}: manifest content hash mismatch (filename {stem}, recomputed {recomputed})",
                    path.display()
                ));
            }

            for entry in &manifest.entries {
                report.entries_checked += 1;
                let root_hash = entry.root_hash()?;
                match read_fragment_tree(root_hash, entry.depth, &store) {
                    Ok(bytes) => {
                        if bytes.len() as u64 != entry.uncompressed_len {
                            ok = false;
                            report.failures.push(format!(
                                "{}: entry {} reconstructed length mismatch (expected {}, got {})",
                                path.display(),
                                entry.path,
                                entry.uncompressed_len,
                                bytes.len()
                            ));
                            continue;
                        }
                        // Determinism check: re-chunk + re-hash the
                        // reconstructed bytes with the same profile and
                        // confirm it reaches the identical address.
                        match build_fragment_tree(&bytes, &entry.chunking_profile, &mut store) {
                            Ok(rebuilt) => {
                                if rebuilt.root != root_hash
                                    || rebuilt.depth != entry.depth
                                    || rebuilt.root_type != entry.root_type
                                {
                                    ok = false;
                                    report.failures.push(format!(
                                        "{}: entry {} is not deterministic under its recorded chunking_profile \
                                         (recorded root {root_hash}, re-derived {})",
                                        path.display(),
                                        entry.path,
                                        rebuilt.root
                                    ));
                                }
                            }
                            Err(e) => {
                                ok = false;
                                report.failures.push(format!(
                                    "{}: entry {} failed to re-chunk for determinism check: {e}",
                                    path.display(),
                                    entry.path
                                ));
                            }
                        }
                    }
                    Err(e) => {
                        ok = false;
                        report.failures.push(format!(
                            "{}: entry {} unreadable: {e}",
                            path.display(),
                            entry.path
                        ));
                    }
                }
            }
            if ok {
                report.manifests_passed += 1;
            }
        }
    }

    Ok(report)
}
