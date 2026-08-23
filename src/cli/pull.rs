//! `bitchain pull <remote> <local_store>` — fetch sealed pack(s) + manifests
//! from another store root ("remote"). See `push.rs` for the scope note on
//! why this is store-root-to-store-root in this reference CLI.

use bitchain::PartitionId;
use std::path::PathBuf;

pub struct PullArgs {
    pub store_root: PathBuf,
    /// `None` means "every partition the remote has".
    pub partition: Option<String>,
    /// Source store root.
    pub from: PathBuf,
    /// Specific pack ids (hex) to pull. Default: all sealed packs.
    pub pack_ids: Vec<String>,
}

pub fn run(args: PullArgs) -> bitchain::Result<()> {
    let partitions: Vec<PartitionId> = match &args.partition {
        Some(p) => vec![super::resolve_partition(p)?],
        None => bitchain::store::list_partitions(&args.from)?,
    };

    if partitions.is_empty() {
        println!(
            "(nothing to pull — no partitions under {})",
            args.from.display()
        );
        return Ok(());
    }

    for partition_id in partitions {
        let n = super::transfer::copy_packs(
            &args.from,
            &args.store_root,
            partition_id,
            &args.pack_ids,
        )?;
        let manifests =
            super::transfer::copy_manifests(&args.from, &args.store_root, partition_id)?;
        println!(
            "{}: pulled {n} pack(s), {manifests} manifest(s) from {}",
            partition_id.to_hex(),
            args.from.display()
        );
    }
    Ok(())
}
