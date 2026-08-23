//! Error type for the bitchain Kit (format v2).

use thiserror::Error;

#[derive(Debug, Error)]
pub enum BitchainError {
    #[error("hash mismatch: expected {expected}, got {actual}")]
    HashMismatch { expected: String, actual: String },

    #[error("not found: {0}")]
    NotFound(String),

    #[error("corrupt pack: {0}")]
    CorruptPack(String),

    #[error("corrupt manifest: {0}")]
    CorruptManifest(String),

    #[error("invalid address: {0}")]
    InvalidAddress(String),

    #[error("partition boundary violation: {0}")]
    PartitionBoundary(String),

    #[error("compression: {0}")]
    Compression(String),

    /// Network/transport failure talking to an S3-compatible remote —
    /// `push`/`pull` only. Not raised by any local-storage path.
    #[error("remote: {0}")]
    Remote(String),

    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("serde: {0}")]
    Serde(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, BitchainError>;
