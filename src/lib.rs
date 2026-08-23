//! # bitchain — Storage Kit (format v2)
//!
//! Content-addressed binary storage: BLAKE3 `(hash, context)` addressing,
//! recursive fragmentation (fixed 1024-child fanout), Zstd-compressed
//! packfiles, and per-partition dedup + GC. Locked by data-architect
//! (CLUS-20); see `AGENT.md` for the full engineering contract and
//! `bitchain-schema.json` for the manifest wire format.
//!
//! Layering:
//! - [`addressing`] — `Hash`, `Context`, `PartitionId`, `Address`
//! - [`fragment`] — chunking + the leaf/list-node recursive tree
//! - [`store`] — the `ImmutableStore`/`ReadBlock`/`WriteBlock` traits and
//!   the reference local-filesystem packfile implementation
//!   ([`store::PartitionStore`])
//! - [`manifest`] — the v2 manifest format
//! - [`gc`] — mark-and-sweep, partition-scoped
//!
//! The CLI (`src/cli/`, `src/main.rs`) is a thin client over this library —
//! every command it exposes is reachable through the public API here
//! without going through a subprocess.
//!
//! **v1 is not read by this crate.** Format v2 is the primary surface; a
//! `--legacy` v1 read path is deferred (explicit blocking constraint from
//! CLUS-20), not silently unsupported by omission.

pub mod addressing;
pub mod error;
pub mod fragment;
pub mod gc;
pub mod manifest;
pub mod store;

pub use addressing::{Address, Context, Hash, PartitionId};
pub use error::{BitchainError, Result};
pub use fragment::{ChunkingProfile, FragmentTree, RootType};
pub use gc::GcReport;
pub use manifest::{Manifest, ManifestEntry};
pub use store::{ImmutableStore, PartitionStore, ReadBlock, WriteBlock};

/// Storage format version this crate implements. See `manifest::FORMAT_VERSION`
/// and `store::packfile::FORMAT_VERSION` for the same constant scoped to each
/// wire artifact — all three are required to stay in lockstep.
pub const FORMAT_VERSION: u32 = 2;

/// Crate version, sourced from `Cargo.toml`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_version_is_two() {
        assert_eq!(FORMAT_VERSION, 2);
    }

    #[test]
    fn version_is_set() {
        assert!(!VERSION.is_empty());
    }
}
