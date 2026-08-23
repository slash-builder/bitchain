//! CLI command implementations. `main.rs` owns argument parsing (`clap`)
//! and dispatch only; each command's actual behavior lives in its own
//! module here, built entirely on the public `bitchain` library API.

pub mod gc;
pub mod ls;
pub mod pack;
pub mod pull;
pub mod push;
pub mod transfer;
pub mod unpack;
pub mod verify;

use bitchain::PartitionId;
use std::path::PathBuf;

/// Default store root: `~/.bitchain/store`.
pub fn default_store_root() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".bitchain").join("store")
}

/// Resolve a partition from either a raw 32-hex-char id or a namespace
/// string to derive it from (`BLAKE3(namespace)[0:16]`).
pub fn resolve_partition(raw: &str) -> bitchain::Result<PartitionId> {
    if raw.len() == 32 && raw.chars().all(|c| c.is_ascii_hexdigit()) {
        PartitionId::from_hex(raw)
    } else {
        Ok(PartitionId::derive(raw))
    }
}
