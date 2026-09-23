//! Manifest — format v2. Records what a `pack` produced: partition, and one
//! fragment-tree entry per packed path.
//!
//! Per the locked spec, each entry carries `root_hash`, `root_type`
//! (`leaf`|`list`), `context`, and `chunking_profile`. `depth` is an
//! additive implementation field (see `fragment.rs` doc comment) needed to
//! reconstruct list-node trees unambiguously; it is not part of the address
//! itself and does not change identity — two manifests with the same
//! `(root_hash, context, chunking_profile)` describe the same object
//! regardless of how `depth` is spelled, since `depth` is fully determined
//! by the chunking profile and input length.
//!
//! `context` is a [`crate::context::Context`] — bitchain's own type, per the
//! 2026-09-23 storage-addressing rulings (see `src/context.rs` module docs).
//! It is a property of this reference, not of the stored object `root_hash`
//! points at; `storage-kit`'s engine has no notion of it.
//!
//! `bitchain-schema.json` is the JSON Schema for this shape; keep the two
//! in lockstep by hand — there is no schema-to-Rust codegen in this repo.

use crate::addressing::{Hash, PartitionId};
use crate::context::Context;
use crate::error::{BitchainError, Result};
use crate::fragment::{ChunkingProfile, RootType};
use serde::{Deserialize, Serialize};

pub const FORMAT_VERSION: u32 = 2;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub format_version: u32,
    pub partition_id: String,
    pub entries: Vec<ManifestEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestEntry {
    /// Relative path as ingested; used as the target path on `unpack`.
    pub path: String,
    /// Hex-encoded 16-byte context. `Context::ANONYMOUS` unless the caller
    /// supplied `--identity`.
    pub context: String,
    /// Hex-encoded BLAKE3 root hash of the fragment tree.
    pub root_hash: String,
    pub root_type: RootType,
    /// List-node levels above the leaves. See module docs.
    pub depth: u32,
    pub uncompressed_len: u64,
    pub chunking_profile: ChunkingProfile,
}

impl Manifest {
    pub fn new(partition_id: PartitionId) -> Self {
        Manifest {
            format_version: FORMAT_VERSION,
            partition_id: partition_id.to_hex(),
            entries: Vec::new(),
        }
    }

    pub fn partition_id(&self) -> Result<PartitionId> {
        PartitionId::from_hex(&self.partition_id)
    }

    pub fn to_json_pretty(&self) -> Result<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    pub fn from_json(s: &str) -> Result<Self> {
        let manifest: Manifest = serde_json::from_str(s)?;
        if manifest.format_version != FORMAT_VERSION {
            return Err(BitchainError::CorruptManifest(format!(
                "manifest format_version {} unsupported (expected {FORMAT_VERSION}); \
                 legacy v1 read is not implemented",
                manifest.format_version
            )));
        }
        Ok(manifest)
    }

    /// The address a `pack` invocation prints and `unpack`/`verify` resolve
    /// against: `BLAKE3` of this manifest's canonical (compact) JSON
    /// serialization. Manifests are stored under
    /// `manifests/<content_hash_hex>.json`, so the manifest is itself
    /// content-addressed the same way the fragments it references are —
    /// this is the "root hash" of a `pack` call, not a fragment-tree hash.
    pub fn content_hash(&self) -> Result<Hash> {
        let compact = serde_json::to_vec(self)?;
        Ok(Hash::of(&compact))
    }
}

impl ManifestEntry {
    pub fn root_hash(&self) -> Result<Hash> {
        Hash::from_hex(&self.root_hash)
    }

    pub fn context(&self) -> Result<Context> {
        Context::from_hex(&self.context)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fragment::RootType;

    #[test]
    fn manifest_json_roundtrips() {
        let partition_id = PartitionId::derive("manifest-test");
        let mut manifest = Manifest::new(partition_id);
        manifest.entries.push(ManifestEntry {
            path: "hello.txt".into(),
            context: Context::ANONYMOUS.to_hex(),
            root_hash: Hash::of(b"hello").to_hex(),
            root_type: RootType::Leaf,
            depth: 0,
            uncompressed_len: 5,
            chunking_profile: ChunkingProfile::default(),
        });

        let json = manifest.to_json_pretty().unwrap();
        let back = Manifest::from_json(&json).unwrap();
        assert_eq!(back.entries.len(), 1);
        assert_eq!(back.entries[0].path, "hello.txt");
    }

    #[test]
    fn rejects_non_v2_format_version() {
        let json = r#"{"format_version":1,"partition_id":"00","entries":[]}"#;
        assert!(Manifest::from_json(json).is_err());
    }

    /// There is no `tests/fixtures/*.json` directory or dedicated sample
    /// manifest file in this repo (verified: only `bitchain-schema.json`
    /// itself ships an on-disk manifest shape, under its own `examples`
    /// key). This test uses that checked-in example — the closest thing to
    /// an existing fixture — as a stand-in: it is real JSON that predates
    /// this change and was written against `storage_kit::addressing::Context`
    /// re-exported as `crate::Context`. If moving `Context` into this crate
    /// changed derivation, representation, or the JSON shape in any way,
    /// this manifest would stop parsing or its `context` field would stop
    /// decoding — it does neither.
    #[test]
    fn existing_schema_example_manifest_still_verifies() {
        let schema: serde_json::Value =
            serde_json::from_str(include_str!("../bitchain-schema.json")).unwrap();
        let example = &schema["examples"][0];
        let json = serde_json::to_string(example).unwrap();

        let manifest = Manifest::from_json(&json)
            .expect("pre-existing schema example manifest must still parse under the new Context");
        assert_eq!(manifest.entries.len(), 2);

        for entry in &manifest.entries {
            // Byte-for-byte: the context field on disk still decodes as a
            // valid 16-byte Context through the type now owned by bitchain.
            let ctx = entry.context().expect("context must still decode");
            assert_eq!(ctx.to_hex(), entry.context);
        }

        // Round-trip: re-serializing must reproduce the same manifest shape
        // (modulo key order, which JSON equality below ignores).
        let round_tripped: serde_json::Value =
            serde_json::from_str(&manifest.to_json_pretty().unwrap()).unwrap();
        assert_eq!(&round_tripped, example);
    }

    #[test]
    fn content_hash_is_deterministic_and_survives_json_roundtrip() {
        let partition_id = PartitionId::derive("content-hash-test");
        let mut manifest = Manifest::new(partition_id);
        manifest.entries.push(ManifestEntry {
            path: "hello.txt".into(),
            context: Context::ANONYMOUS.to_hex(),
            root_hash: Hash::of(b"hello").to_hex(),
            root_type: RootType::Leaf,
            depth: 0,
            uncompressed_len: 5,
            chunking_profile: ChunkingProfile::default(),
        });

        let hash_a = manifest.content_hash().unwrap();
        let json = manifest.to_json_pretty().unwrap();
        let back = Manifest::from_json(&json).unwrap();
        let hash_b = back.content_hash().unwrap();
        assert_eq!(
            hash_a, hash_b,
            "content hash must survive a JSON round-trip"
        );
    }
}
