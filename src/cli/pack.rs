//! `bitchain pack <path>` — split, hash, chunk, compress into the local
//! store, then print the manifest's content hash (the "root hash" a caller
//! passes to `unpack`/`verify`).

use bitchain::fragment::build_fragment_tree;
use bitchain::{ChunkingProfile, Context, Manifest, ManifestEntry, PartitionStore};
use std::path::PathBuf;

pub struct PackArgs {
    pub input: PathBuf,
    pub store_root: PathBuf,
    pub partition: String,
    /// Logical identity for the whole pack; `Context::ANONYMOUS` if unset.
    pub identity: Option<String>,
    pub profile: ChunkingProfile,
}

/// Ingest `args.input` (a file or a directory, walked recursively) into the
/// store and return the manifest's content hash as hex.
pub fn run(args: PackArgs) -> bitchain::Result<String> {
    let partition_id = super::resolve_partition(&args.partition)?;
    let mut store = PartitionStore::open(&args.store_root, partition_id)?;
    let context = match &args.identity {
        Some(identity) => Context::derive(identity),
        None => Context::ANONYMOUS,
    };

    let mut files: Vec<(PathBuf, String)> = Vec::new();
    if args.input.is_file() {
        let name = args
            .input
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("file")
            .to_string();
        files.push((args.input.clone(), name));
    } else if args.input.is_dir() {
        for entry in walkdir::WalkDir::new(&args.input) {
            let entry = entry.map_err(|e| {
                bitchain::BitchainError::NotFound(format!("walking {}: {e}", args.input.display()))
            })?;
            if !entry.file_type().is_file() {
                continue;
            }
            let rel = entry
                .path()
                .strip_prefix(&args.input)
                .unwrap_or(entry.path())
                .to_string_lossy()
                .replace('\\', "/");
            files.push((entry.path().to_path_buf(), rel));
        }
        // Deterministic entry order regardless of directory-walk order, so
        // the same directory contents always yield the same manifest hash.
        files.sort_by(|a, b| a.1.cmp(&b.1));
    } else {
        return Err(bitchain::BitchainError::NotFound(format!(
            "{} is neither a file nor a directory",
            args.input.display()
        )));
    }
    if files.is_empty() {
        return Err(bitchain::BitchainError::NotFound(format!(
            "nothing to pack under {}",
            args.input.display()
        )));
    }

    let mut manifest = Manifest::new(partition_id);
    for (path, relative_path) in &files {
        let data = std::fs::read(path)?;
        let tree = build_fragment_tree(&data, &args.profile, &mut store)?;
        manifest.entries.push(ManifestEntry {
            path: relative_path.clone(),
            context: context.to_hex(),
            root_hash: tree.root.hash.to_hex(),
            root_type: tree.root_type,
            depth: tree.depth,
            uncompressed_len: tree.uncompressed_len,
            chunking_profile: args.profile.clone(),
        });
        eprintln!(
            "packed {relative_path} -> {} ({} bytes, depth {})",
            tree.root.hash, tree.uncompressed_len, tree.depth
        );
    }

    // Sealing here (rather than leaving content in the active pack) means
    // the very next `push` sees fully-formed, immutable packs.
    store.seal_active()?;

    let content_hash = manifest.content_hash()?;
    let json = manifest.to_json_pretty()?;
    store.write_manifest(&content_hash.to_hex(), &json)?;

    eprintln!(
        "partition {} — {} file(s) packed",
        partition_id.to_hex(),
        files.len()
    );
    Ok(content_hash.to_hex())
}
