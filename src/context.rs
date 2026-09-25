//! `Context` — the logical identity of a manifest reference.
//!
//! Owned by `bitchain`, not `storage-kit`, per the 2026-09-23 storage
//! addressing rulings (data-architect and security-engineer, ruling
//! independently on the same escalation, both citing DJ's 2026-06 storage
//! lock):
//!
//! - `wiki/decisions/storage-addressing-hash-context-ruling-2026-09-23.md`
//! - `wiki/decisions/storage-addressing-hash-context-ruling-security-2026-09-23.md`
//!
//! The address of a stored object is `(partition_id, content_id)` —
//! `storage-kit`'s `ImmutableStore` knows nothing of logical identity and
//! deduplicates on content alone, full stop. `Context` was never a property
//! of a *stored object*; it is a property of a *reference*, and
//! [`crate::manifest::ManifestEntry`] — `{ path, context, root_hash }` — is
//! where that property has always actually lived, since `--identity`
//! shipped. This module makes bitchain's ownership of it explicit: bitchain
//! now defines `Context` itself instead of re-exporting `storage-kit`'s copy.
//!
//! Moving the type down the dependency edge does not change what it means or
//! how it is computed. Derivation, representation, and the `ANONYMOUS`
//! sentinel are byte-for-byte identical to the
//! `storage_kit::addressing::Context` this replaces for manifest purposes —
//! same `BLAKE3(logical-identity-string)[0:16]`, same 16 bytes, same hex
//! encoding — so no manifest already on disk changes meaning, and the JSON
//! manifest schema (`bitchain-schema.json`) does not change.
//!
//! `storage-kit` still carries its own internal `Context`/`Address` types
//! (used by `FragmentTree::root`, always `ANONYMOUS` for fragmentation
//! internal nodes per its own `fragment.rs`). This module does not touch or
//! remove those — retiring them from `storage-kit` is a separate,
//! not-yet-executed change to that crate, out of scope here.

use crate::error::{BitchainError, Result};
use std::fmt;

/// Length in bytes of a context.
pub const CONTEXT_LEN: usize = 16;

/// A 16-byte, partition-scoped logical identity for a manifest reference.
/// Canonical construction is `BLAKE3(logical-identity-string)[0:16]`. The
/// all-zero context is reserved: it means "anonymous — identity is
/// content," and is the value every `pack` entry gets unless the caller
/// supplied `--identity`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Context([u8; CONTEXT_LEN]);

impl Context {
    /// Reserved anonymous context: `0x00..00`. Identity is content.
    pub const ANONYMOUS: Context = Context([0u8; CONTEXT_LEN]);

    /// Derive a canonical context from a logical-identity string:
    /// `BLAKE3(logical-identity-string)[0:16]`.
    pub fn derive(logical_identity: &str) -> Self {
        let full = blake3::hash(logical_identity.as_bytes());
        let mut out = [0u8; CONTEXT_LEN];
        out.copy_from_slice(&full.as_bytes()[..CONTEXT_LEN]);
        Context(out)
    }

    pub fn from_bytes(bytes: [u8; CONTEXT_LEN]) -> Self {
        Context(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; CONTEXT_LEN] {
        &self.0
    }

    pub fn is_anonymous(&self) -> bool {
        self.0 == [0u8; CONTEXT_LEN]
    }

    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }

    pub fn from_hex(s: &str) -> Result<Self> {
        let bytes = hex::decode(s)
            .map_err(|e| BitchainError::InvalidAddress(format!("bad context hex: {e}")))?;
        let arr: [u8; CONTEXT_LEN] = bytes
            .try_into()
            .map_err(|_| BitchainError::InvalidAddress("context must be 16 bytes".into()))?;
        Ok(Context(arr))
    }
}

impl fmt::Debug for Context {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_anonymous() {
            write!(f, "Context(ANONYMOUS)")
        } else {
            write!(f, "Context({})", self.to_hex())
        }
    }
}

impl Default for Context {
    fn default() -> Self {
        Context::ANONYMOUS
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Byte-for-byte parity with the `storage_kit::addressing::Context`
    /// derivation this type replaces for manifest purposes. If this ever
    /// fails, existing manifests stop verifying.
    #[test]
    fn derivation_matches_storage_kits_prior_formula() {
        let ours = Context::derive("household:acme/user:dj");
        let expected_hex = {
            let full = blake3::hash(b"household:acme/user:dj");
            hex::encode(&full.as_bytes()[..CONTEXT_LEN])
        };
        assert_eq!(ours.to_hex(), expected_hex);
    }

    #[test]
    fn context_derivation_is_deterministic() {
        let a = Context::derive("household:acme/user:dj");
        let b = Context::derive("household:acme/user:dj");
        let c = Context::derive("household:acme/user:someone-else");
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert!(!a.is_anonymous());
    }

    #[test]
    fn anonymous_context_is_all_zero() {
        assert!(Context::ANONYMOUS.is_anonymous());
        assert_eq!(Context::ANONYMOUS.as_bytes(), &[0u8; CONTEXT_LEN]);
    }

    #[test]
    fn anonymous_hex_matches_the_value_recorded_in_every_existing_manifest() {
        // `bitchain-schema.json`'s own docs describe ANONYMOUS as
        // "all-zero"; this is the literal on-disk hex value every manifest
        // written without `--identity` carries in its `context` field.
        assert_eq!(Context::ANONYMOUS.to_hex(), "0".repeat(32));
    }

    #[test]
    fn hex_roundtrips() {
        let ctx = Context::derive("roundtrip-me");
        let hex = ctx.to_hex();
        let back = Context::from_hex(&hex).unwrap();
        assert_eq!(ctx, back);
    }

    #[test]
    fn from_hex_rejects_wrong_length() {
        assert!(Context::from_hex("ab").is_err());
    }
}
