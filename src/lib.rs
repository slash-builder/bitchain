//! # bitchain (format v2)
//!
//! Content-addressed binary storage: BLAKE3 `(hash, context)` addressing,
//! recursive fragmentation (fixed 1024-child fanout), Zstd-compressed
//! packfiles, and per-partition dedup + GC. See `docs/concepts.md` for the
//! engineering contract and `bitchain-schema.json` for the manifest wire
//! format.
//!
//! Layering. The storage ENGINE lives in `slashbuilder-storage-kit`
//! (imported as `storage_kit`) and is re-exported here, not reimplemented.
//! This crate is the thin protocol + CLI layer over it.
//!
//! Re-exported from `storage-kit`:
//! - [`addressing`] — `Hash`, `PartitionId`. `Context` and `Address` are
//!   retired from storage-kit entirely as of format v3: the engine addresses by
//!   `(partition_id, content_id)` alone and no longer carries a logical-
//!   identity concept for this crate to re-export or route around. This
//!   crate's own [`context::Context`] (below) is the only `Context` that
//!   exists anywhere in this dependency graph now.
//! - [`fragment`] — chunking + the leaf/list-node recursive tree
//! - [`store`] — the `ImmutableStore` trait and the reference
//!   local-filesystem packfile implementation ([`store::PartitionStore`])
//!
//! Owned by this crate:
//! - [`context`] — the `Context` type: a manifest reference's logical
//!   identity. `storage-kit`
//!   addresses purely by `(partition_id, content_id)` and knows nothing of
//!   logical identity; `Context` is a property of a reference, and
//!   `ManifestEntry` is where it belongs. See `src/context.rs` module docs.
//! - [`manifest`] — the v2 manifest format
//! - [`gc`] — mark-and-sweep, partition-scoped
//!
//! The CLI (`src/cli/`, `src/main.rs`) is a thin client over this library —
//! every command it exposes is reachable through the public API here
//! without going through a subprocess.
//!
//! **v1 is not read by this crate.** Format v2 is the primary surface; a
//! `--legacy` v1 read path is deliberately not implemented.

// The engine modules are storage-kit's, re-exported under their original
// paths so `crate::addressing::…` etc. keep resolving.
pub use storage_kit::{addressing, fragment, store};

/// Partition provisioning types, re-exported from `storage-kit` (format v3).
/// `PartitionSpec` is
/// what `bitchain init` (`src/cli/init.rs`) builds and passes to
/// `PartitionStore::create` -- there is no default `RetentionClass` or
/// `EncryptionMode` anywhere in this crate either, on purpose.
pub use storage_kit::{EncryptionMode, PartitionSpec, RetentionClass, Subject, SubjectKind};

/// Error type, re-exported from `storage-kit`.
///
/// `StorageError` there is the union of this crate's old `BitchainError` and
/// storage-kit's own backend-facing error, merged when the engine moved into
/// `storage-kit`. `BitchainError` is kept as an alias so existing callers and
/// downstream users don't break; the only variant that changed name is
/// `Remote(String)` → `Backend(String)`, which had no callers here.
pub mod error {
    pub use storage_kit::error::{Result, StorageError, StorageError as BitchainError};
}

pub mod context;
pub mod gc;
pub mod manifest;

pub use addressing::{Hash, PartitionId};
pub use context::Context;
pub use error::{BitchainError, Result};
pub use fragment::{ChunkingProfile, FragmentTree, RootType};
pub use gc::GcReport;
pub use manifest::{Manifest, ManifestEntry};
// `ReadBlock`/`WriteBlock` are deliberately NOT re-exported: storage-kit
// collapsed both into the single `ImmutableStore` trait -- same methods,
// same signatures, same implementors, no behavior change (its own
// store/mod.rs documents the collapse).
pub use store::{ImmutableStore, PartitionStore};

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
