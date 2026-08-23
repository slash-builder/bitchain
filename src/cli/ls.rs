//! `bitchain ls [<partition>]` — summarize what's stored locally. With no
//! partition given, summarizes every partition found under the store root.

use bitchain::PartitionStore;
use std::path::PathBuf;

pub struct LsArgs {
    pub store_root: PathBuf,
    /// `None` means "every partition under the store root".
    pub partition: Option<String>,
}

pub fn run(args: LsArgs) -> bitchain::Result<()> {
    let partitions = match &args.partition {
        Some(p) => vec![super::resolve_partition(p)?],
        None => bitchain::store::list_partitions(&args.store_root)?,
    };

    if partitions.is_empty() {
        println!("(no partitions under {})", args.store_root.display());
        return Ok(());
    }

    for partition_id in partitions {
        let store = PartitionStore::open(&args.store_root, partition_id)?;
        let sealed = store.sealed_pack_ids().len();
        let manifests = store.list_manifests()?.len();
        println!(
            "{}  sealed_packs={sealed}  active_entries={}  total_entries={}  manifests={manifests}  root={}",
            partition_id.to_hex(),
            store.active_entry_count(),
            store.all_hashes().len(),
            store.partition_root().display(),
        );
    }
    Ok(())
}
