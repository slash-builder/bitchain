//! `bitchain gc [<partition>] [--target-utilization N]` — mark-and-sweep,
//! skipping the (expensive) repack for any partition already at or above
//! `target_utilization` percent live/total, since a repack wouldn't
//! reclaim enough to be worth the I/O. With no partition given, every
//! partition under the store root is considered.

use bitchain::PartitionId;
use std::path::PathBuf;

pub struct GcArgs {
    pub store_root: PathBuf,
    /// `None` means "every partition under the store root".
    pub partition: Option<String>,
    /// Percent (0-100). A partition already at or above this utilization is
    /// left alone.
    pub target_utilization: f64,
}

pub fn run(args: GcArgs) -> bitchain::Result<()> {
    let partitions: Vec<PartitionId> = match &args.partition {
        Some(p) => vec![super::resolve_partition(p)?],
        None => bitchain::store::list_partitions(&args.store_root)?,
    };

    if partitions.is_empty() {
        println!("(no partitions under {})", args.store_root.display());
        return Ok(());
    }

    for partition_id in partitions {
        let (live, total, pct) = bitchain::gc::utilization(&args.store_root, partition_id)?;
        if pct >= args.target_utilization {
            println!(
                "{}: utilization {pct:.1}% ({live}/{total}) >= target {:.1}% — skipped",
                partition_id.to_hex(),
                args.target_utilization
            );
            continue;
        }
        let report = bitchain::gc::collect(&args.store_root, partition_id)?;
        println!(
            "{}: utilization was {pct:.1}% ({live}/{total}) — reclaimed {} of {} entries, {} live remain",
            partition_id.to_hex(),
            report.reclaimed_count,
            total,
            report.live_count,
        );
    }
    Ok(())
}
