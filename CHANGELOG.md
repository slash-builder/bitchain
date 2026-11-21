# Changelog

All notable changes to **bitchain** are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.1](https://github.com/slash-builder/bitchain/compare/v0.2.0...v0.2.1) - 2026-11-21

### Other

- *(deps)* Bump thiserror from 1.0.69 to 2.0.21 ([#2](https://github.com/slash-builder/bitchain/pull/2))

## [0.2.0]

First release on crates.io, as `slashbuilder-bitchain`. The installed binary and the library are both still named
`bitchain`.

### Added

- `bitchain init`: explicit partition provisioning with required `--subject-kind`, `--subject-id`, `--retention` and
  `--encryption` flags and no defaults. It is the only command that creates a partition.
- `Context`, the logical identity of a manifest reference, is now owned by this crate (same derivation and encoding as
  before, so existing manifests keep their meaning).
- Garbage collection preserves each entry's metadata byte for byte when it repacks.
- `pack` seals the active pack before returning, so its output is immediately `push`-able.

### Changed

- **Package renamed** to `slashbuilder-bitchain`; `[lib] name` and the binary name are unchanged.
- Storage format v2 is built on `slashbuilder-storage-kit` 0.2 from crates.io (format v3 engine: addressing is
  `(partition_id, content_id)`; `PartitionStore::create` is separate from `open`). Every store-opening command now
  fails with `partition not provisioned` for a partition that `init` has not created.
- Minimum supported Rust version is 1.88 (set by `slashbuilder-storage-kit`).
- The storage engine moved out of this crate into `slashbuilder-storage-kit` and is re-exported.
- Documentation rewritten to describe the shipped behaviour; the manifest schema now describes format v2.

### Removed

- The retired v1 format (SHA-256, flat `files[].blocks[]` manifests) and its `ingest`, `rebuild`, `show` and `validate`
  commands. There is no `--legacy` read path.
- The `docs/integration-quickring.md` planning sketch.

[Unreleased]: https://github.com/slash-builder/bitchain/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/slash-builder/bitchain/commits/main
