# Manifest format (v2)

> **Pending relocation, not yet executed:** the manifest shape and format
> version described here are locked and unaffected, but `src/manifest.rs`
> is slated (data-architect ruling, 2026-08-23) to relocate into
> `clusterzer0/storage-kit` alongside the rest of the lib layer — see
> `AGENT.md`'s "Open work" section. Source-file references below will
> need re-verifying once that executes.

**CLUS-20.** This document describes the bitchain manifest as it is actually
produced and read by the `bitchain` crate (`src/manifest.rs`), verified
against a live `cargo build` and a real `pack` → `unpack` round trip on
2026-08-23. It supersedes the format described in `bitchain-schema.json`
prior to this pass and in `README.md`'s "JSON Format" section — both
described the retired v1 (SHA-256, flat block list) shape. `bitchain-schema.json`
has been updated in the same pass as this document; see the note at the
end of this file.

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
for brevity) — every field below is verified against `src/manifest.rs`,
`src/fragment.rs`, and `src/addressing.rs`, not inferred from the ticket
brief.

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
`ingest`/`rebuild` CLI) is an open decision escalated to DJ per
`AGENT.md`'s "Migration" note — not decided, not defaulted, do not assume
either answer.

## Gap: no creation timestamp

**The manifest does not record a creation timestamp.** This was assumed in
the original CLUS-20 pragmatic-spec brief ("Version (v2), Creation
timestamp" as required manifest fields) but is absent from `Manifest` and
`ManifestEntry` in the shipped code — confirmed by reading `src/manifest.rs`
and by inspecting real `pack` output. This is a real gap between the
ticket's assumption and what shipped, not an oversight in this document.
No owner has ruled on whether a timestamp field should be added (schema
change — data-architect's call, since CLUS-20's manifest shape is locked
by data-architect) or is deliberately out of scope (a manifest's identity
is content-addressed and timestamp-independent by design, so adding one
would be observability, not identity). Flagging to data-architect; not
adding a field to code or docs on my own authority.

## `bitchain-schema.json`

The repo's `bitchain-schema.json` (referenced from `src/lib.rs`'s module
doc as "the JSON Schema for this shape; keep the two in lockstep by hand —
there is no schema-to-Rust codegen in this repo") described the retired
v1 format as of 2026-08-23 before this pass. It has been rewritten
alongside this document to match the shape above. There is still no
schema-to-Rust codegen — a future drift between `src/manifest.rs` and
`bitchain-schema.json` is a real risk that has to be caught by hand or by
adding a codegen/test step (software-developer's call, not a docs fix).
