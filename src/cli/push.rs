//! `bitchain push <local_store> <remote>` — replicate sealed pack(s) +
//! manifests to another store root ("remote").
//!
//! **Scope note:** v2's reference CLI wires the local-filesystem backend
//! end to end (`local_store`/`remote` are any paths this process can write
//! to, including a mounted network share, `rclone`/`s3fs` mount, etc.) —
//! `pack`/`unpack`/`verify` are the blocking gate per CLUS-20's
//! re-forecast; a networked HTTP/S3 remote is `storage-kit`'s job
//! (`S3Backend`/`HttpBackend`, implementing the same `ImmutableStore`
//! contract this crate already exposes). Programmatic consumers who need a
//! real S3 remote today should reach for that crate directly rather than
//! this CLI; swapping this command's transport later is additive, not a
//! breaking change to `PushArgs`.

use bitchain::PartitionId;
use std::path::PathBuf;

pub struct PushArgs {
    pub store_root: PathBuf,
    /// `None` means "every partition under the store root".
    pub partition: Option<String>,
    /// Destination store root.
    pub to: PathBuf,
    /// Specific pack ids (hex) to push. Default: all sealed packs.
    pub pack_ids: Vec<String>,
}

pub fn run(args: PushArgs) -> bitchain::Result<()> {
    let partitions: Vec<PartitionId> = match &args.partition {
        Some(p) => vec![super::resolve_partition(p)?],
        None => bitchain::store::list_partitions(&args.store_root)?,
    };

    if partitions.is_empty() {
        println!(
            "(nothing to push — no partitions under {})",
            args.store_root.display()
        );
        return Ok(());
    }

    for partition_id in partitions {
        let n =
            super::transfer::copy_packs(&args.store_root, &args.to, partition_id, &args.pack_ids)?;
        let manifests = super::transfer::copy_manifests(&args.store_root, &args.to, partition_id)?;
        println!(
            "{}: pushed {n} pack(s), {manifests} manifest(s) to {}",
            partition_id.to_hex(),
            args.to.display()
        );
    }
    Ok(())
}
