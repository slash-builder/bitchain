# bitchain — Agent Context
# clusterzer0/bitchain
#
# Paste this file into your AI's system prompt to work on this repo.
# ─────────────────────────────────────────────────────────────────────────────

You are working on **bitchain** — the **Storage Kit**, a Rust library and CLI
for content-addressed binary storage, created by Douglas Lockamy (DJ) at Lockamy
Studios.

Content-addressing is the core invariant — a block's hash is its identity.
Local-first, offline-first design. Never suggest changes that compromise these.

---

## What bitchain is

**The Storage Kit** — a reusable Rust library + CLI for ingesting files into
immutable, content-addressed block stores and reconstructing them on demand.

Think of it as:
- **The reference implementation** of the bitchain specification (blocks, manifests, verification)
- **A standalone toolkit** (library + CLI) for systems that need content-addressed binary storage
- **Deployed by:** Quickring Courier (out-of-band file sharing), Refraction (managed bitchain server), Thunderhead devices (block cache + local storage)

Files are split into fixed-size SHA-256 blocks; a JSON manifest records their URIs and checksums. Any system with access to the block sources can reconstruct the original file. Blocks are stored locally by default; S3 / HTTPS / Refraction are backend options, never required.

**Primary use cases:**
- Distributed file sharing (Quickring Courier app)
- Managed binary artifact versioning (Refraction SaaS)
- On-device block storage (Thunderhead NAS, Anvil)
- Standalone CLI for local archival

---

## Storage Kit positioning

**Storage Kit is a reusable component**, not a full application. It lives in the
clusterzer0 / Developer Tools business unit and is consumed by:
- **Quickring Courier** (file sharing; Quickring org)
- **Refraction** (managed server; clusterzer0 org)
- **Thunderhead** (on-device storage; Thunderhead org)
- **Future language bindings** (Go, Python, TypeScript via FFI or gRPC)

The Rust `lib` crate is the load-bearing API. The CLI is a thin reference client.

---

## Locked engineering — local-first invariant

Storage Kit **does not require internet or backend connectivity to function.**

- **Default backend:** local filesystem (no network)
- **Optional backends:** S3, HTTPS file mirrors, Refraction HTTP API
- **Guarantees:** works offline; blocks are stored-then-verified; no state on remote servers is required for reconstruction

When Refraction or S3 are unavailable, bitchain continues to serve blocks from
the local cache. Offline-first is not a nice-to-have — it is the invariant.

---

## Core data model

```
Block:
  hash: SHA-256 of content (hex string) — the block's identity and only naming key
  data: raw bytes (stored locally at --output-dir by default)

Manifest (JSON):
  version: "1.0"
  files: [
    {
      path: "original/file/path.bin",
      blocks: [
        { hash: "<sha256>", uris: ["s3://...", "https://...", "file://..."] }
      ]
    }
  ]
```

A manifest reconstructs files by:
1. For each file, iterate its blocks in order
2. Try each URI in sequence until one succeeds
3. Verify hash of fetched bytes
4. Concatenate blocks to restore original file

Deduplication is automatic — identical blocks across files share storage.

---

## CLI scope (locked)

The CLI is a **reference client**, not the only interface. Its scope:

### Ingest phase
```
bitchain ingest --input <path> --output <manifest.json> [--output-dir <dir>] [--uri-base <uri>]
```
- Split a file/directory into blocks
- Write blocks locally (default: `--output-dir ./blocks`)
- Emit a manifest JSON with `file://` URIs (or `s3://`, `https://` if `--uri-base` given)
- Dry-run mode for validation

### Rebuild phase
```
bitchain rebuild --bitchain <manifest.json> --output-dir <dir>
```
- Fetch blocks from the manifest's URIs (in order; first success wins)
- Verify each block's hash
- Concatenate into the original file structure
- Restore under `--output-dir`

### Utility
```
bitchain show <manifest.json>          # Pretty-print manifest
bitchain validate <manifest.json>      # Schema validation
bitchain help                          # Command reference
```

CLI does NOT manage:
- Remote server state
- Subscription / licensing / quotas
- Asset versioning beyond the manifest format
- User accounts or authentication (Refraction's job)

---

## UI scope (not in this repo)

Bitchain is a **library and CLI**, not a GUI application. The library exposes
a stable Rust API for embedding in other tools. Language bindings (Dart, Go,
Python) via FFI or gRPC are external projects.

- **Quickring Courier** uses the library via Flutter/Rust FFI bridge
- **Refraction** uses the library internally; exposes HTTP API
- **Thunderhead** uses the library for on-device storage

---

## Stack

- Language: Rust (edition 2021)
- CLI framework: `clap` v4 with derive macros
- Async runtime: `tokio` (full features)
- Serialization: `serde` + `serde_json`
- Hashing: `sha2` v0.10
- S3: `aws-sdk-s3` v1 + `aws-config` v1
- HTTP: `reqwest` v0.11 with JSON feature
- License: **Apache-2.0**

---

## Architecture

```
bitchain/
├── src/
│   ├── lib.rs           — public API (block, manifest, store traits)
│   ├── main.rs          — CLI entry, clap subcommands
│   ├── block.rs         — block splitting, hashing, I/O
│   ├── manifest.rs      — manifest JSON schema, serialization
│   ├── store/
│   │   ├── mod.rs       — Store trait (abstract)
│   │   ├── local.rs     — local filesystem backend
│   │   ├── s3.rs        — S3 backend (optional feature)
│   │   └── http.rs      — HTTPS + Refraction backend
│   └── config.rs        — config file, env vars, AWS region
├── Cargo.toml
├── bitchain-schema.json — JSON Schema for manifests
└── Jenkinsfile          — CI: fmt + clippy + test + publish
```

---

## API surface (lib.rs)

Public types and traits exposed for embedding:
- `Block` — hash, bytes, metadata
- `Manifest` — file list, block list, URIs
- `Store` trait — abstract backend (local, s3, http, custom)
- `ingest()` — split file, produce manifest
- `rebuild()` — fetch blocks, verify, reconstruct

The CLI is built on top of this API; third-party code links the library directly.

---

## CI/CD

Jenkins pipeline: Pre-flight → `cargo fmt --check` + `cargo clippy` + `cargo test`
+ `cargo build --release` → publish to `nexus.softsurve.com/repository/cargo-hosted/`.

Jenkins credential: `nexus-credentials`.
Cargo registry config at deploy time — not committed to repo.

---

## Related work

- **Refraction** (`clusterzer0/refraction`) — self-hosted management server consuming Storage Kit
- **Quickring Courier** (Quickring org) — consumer file-sharing app consuming Storage Kit via FFI
- **Thunderhead** (Thunderhead Systems) — on-device block storage using Storage Kit
- **The bitchain specification** — the formal reference (`spec/`) — neutral of implementation language

---

## Jira

Project: **CLUS** (clusterzer0) — fairmerce.atlassian.net
- **CLUS-1** Storage Kit (was bitchain CLI)
- **CLUS-6** Refresh `AGENT.md` (this file)
- **CLUS-20** Define the bitchain spec (reference docs + conformance suite)
- **CLUS-21** Monorepo + lib/cli split

---

## Key invariants

1. **Content-addressed** — hash is the block's identity; no content is stored twice
2. **Offline-first** — works without internet; backends are optional
3. **Reusable library** — embedded by other projects; stable API; not a monolith
4. **Specification-first** — the manifest format and block protocol are language-neutral; Rust is the reference
5. **Minimal CLI** — reference client; not the only interface
