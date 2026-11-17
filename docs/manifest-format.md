# Manifest format (v2)

This document describes the bitchain manifest as it is actually produced
and read by this crate (`src/manifest.rs`). `fragment.rs` and
`addressing.rs`, referenced below, are in the
[`slashbuilder-storage-kit`](https://crates.io/crates/slashbuilder-storage-kit)
crate. The format-v2 shape is documented here and in
`bitchain-schema.json`; the retired v1 (SHA-256, flat block list) shape is
not supported.

## What a manifest is

A manifest is the output of `bitchain pack`: one JSON document per pack
operation, recording the partition it was packed into and, per input path,
the root of that path's content-addressed fragment tree. The manifest is
the only thing `unpack` needs (plus the store holding the fragments) to
reconstruct the original bytes.

Manifests are also copied into the partition's own `manifests/` directory
at pack time (`store.write_manifest`), independent of where the CLI's
`--output` file is written — this is how `gc`'s mark phase discovers which
fragment trees are live.

## Shape

```json
{
  "format_version": 2,
  "partition_id": "7aed631198158a211b8778bdb2267829",
  "entries": [
    {
      "path": "a.txt",
      "context": "0c1b1bc9896253c19131abb26e3b1342",
      "root_hash": "88ddc1dfe8016046c29f23a44b25501c1125d64b51718ce4aa3eb8276b9d6ebc",
      "root_type": "leaf",
      "depth": 0,
      "uncompressed_len": 54,
      "chunking_profile": {
        "codec": "fast_cdc",
        "min_size": 4096,
        "avg_size": 16384,
        "max_size": 65536
      }
    }
  ]
}
```

This is a real, captured `pack` output (paths/hashes trimmed to one entry
for brevity) — every field below is verified against `src/manifest.rs` and the
storage-kit `fragment` and `addressing` modules.

## Top-level fields

| Field | Type | Description |
|---|---|---|
| `format_version` | integer | Always `2` for this format. `Manifest::from_json` rejects any other value — see "Versioning" below. |
| `partition_id` | string, 32 lowercase hex chars | The 16-byte partition this manifest belongs to, hex-encoded. Canonically `BLAKE3(household/namespace-string)[0:16]` (`PartitionId::derive`), but a manifest only ever stores the resolved hex id — it does not record which namespace string produced it. |
| `entries` | array | One entry per packed input path. |

## Entry fields

| Field | Type | Description |
|---|---|---|
| `path` | string | Relative path as packed; joined with `<output_path>` (a directory, always) to form the target path on `unpack`. For a directory input, this is the path relative to the packed directory root. |
| `context` | string, 32 lowercase hex chars | 16-byte logical identity, hex-encoded. `Context::ANONYMOUS` (all-zero) unless `pack --identity <id>` was supplied, in which case it's `BLAKE3("<id>")[0:16]` — the same context for every entry in that one `pack` invocation (identity is set per-pack, not derived per-entry-path). Two entries with the same content but different `context` are still deduplicated at the physical (hash-only) layer — `context` is a logical label, never part of the storage key. |
| `root_hash` | string, 64 lowercase hex chars | BLAKE3 hash (32 bytes, hex-encoded) of the fragment tree's root block. |
| `root_type` | string, `"leaf"` \| `"list"` | **Not** `"list-node"` — the actual serialized values are `leaf` and `list` (`RootType`, `#[serde(rename_all = "snake_case")]`). `leaf` means the content fit in a single chunk; `list` means one or more list-node fold levels sit above the leaves. |
| `depth` | integer | Number of list-node levels above the leaves (`0` for a bare leaf). Not part of the object's address — two manifests with identical `(root_hash, context, chunking_profile)` describe the same object regardless of `depth`, since `depth` is fully determined by the chunking profile and input length. It exists because reconstruction needs to know how many fold levels to walk, and recomputing that from scratch is wasted work `unpack` shouldn't have to redo. |
| `uncompressed_len` | integer (u64) | Total byte length of the original content. `unpack` checks the reconstructed byte count against this field and fails loudly on mismatch (see the CLI reference's `unpack` section). |
| `chunking_profile` | object | Recorded so a future `unpack` (or any other implementation) reconstructing the same bytes reaches the same root hash. See below. |

## `chunking_profile`

Tagged union (`#[serde(tag = "codec", rename_all = "snake_case")]`), two variants:

```json
{ "codec": "fixed", "block_size": 1048576 }
```
```json
{ "codec": "fast_cdc", "min_size": 4096, "avg_size": 16384, "max_size": 65536 }
```

`fast_cdc` is the default (`ChunkingProfile::default()`), with `min_size:
4096, avg_size: 16384, max_size: 65536` — these are also `pack`'s CLI
defaults (`--cdc-min`/`--cdc-avg`/`--cdc-max`). `fixed` is selected with
`pack --fixed-block-size <bytes>`, which suppresses the CDC flags entirely.

## Versioning

`format_version` is checked on every `Manifest::from_json` call
(`src/manifest.rs`). Any value other than `2` is rejected with
`BitchainError::CorruptManifest` and the message explicitly notes "legacy
v1 read is not implemented" — **there is no `--legacy` read path in this
crate today**. Whether one is ever added (reading the retired SHA-256 /
fixed-1MB-block / flat-JSON manifest format written by the pre-rewrite
`ingest`/`rebuild` CLI) is undecided; do not assume either answer.

## Gap: no creation timestamp

**The manifest does not record a creation timestamp.** `Manifest` and
`ManifestEntry` have no such field, as `src/manifest.rs` and real `pack`
output show. Whether to add one (a schema change) or to leave it out is
undecided: a manifest's identity is content-addressed and
timestamp-independent by design, so a timestamp would be observability, not
identity.

## `bitchain-schema.json`

The repo's `bitchain-schema.json` is the JSON Schema (draft-07) for the
shape above. There is no schema-to-Rust codegen: the two are kept in
lockstep by hand, and a unit test validates the schema's example manifest.
Drift between `src/manifest.rs` and `bitchain-schema.json` is a real risk
to watch for in review.
