# bitchain

A **content-addressed binary storage** command-line tool and Rust library.
It splits files into immutable, content-addressed fragments, stores them in
Zstd-compressed packfiles on the local filesystem, and reconstructs them on
demand. Identical content is stored once per partition, and every stored byte
can be re-verified.

- Package: **`slashbuilder-bitchain`** (installed binary: `bitchain`; library: `bitchain`)
- Status: **alpha** (0.2.0). Storage format v2; the command surface and library API may still change before 1.0.
- Licence: Apache-2.0
- Minimum supported Rust: 1.88

Format v2: BLAKE3 addressing, FastCDC or fixed-size chunking, recursive
fragmentation, Zstd packfiles, `(hash, context)` manifest references and
per-partition isolation. The storage engine itself is the separate
[`slashbuilder-storage-kit`](https://crates.io/crates/slashbuilder-storage-kit)
crate; this crate is the command-line client, the manifest protocol and
garbage collection over it.

## What it is not

- Not a server, and not a sync service. Every command runs against the local
  filesystem only; `push` and `pull` copy between two directories (which may be
  a mounted network share). Networked S3 and HTTP backends exist in
  `slashbuilder-storage-kit` but are not wired into this CLI.
- Not an account, quota or authentication system.
- Not encrypted yet: `plaintext` is the only encryption mode any command can
  write to (`sealed-required` is reserved).
- Cannot read the retired v1 manifest format (SHA-256, flat block list).

## Install

```sh
cargo install slashbuilder-bitchain      # installs the `bitchain` binary
```

or from a checkout:

```sh
cargo build --release                    # target/release/bitchain
```

## Quick start

Partitions are provisioned once with `init`, which prints the partition id.
Every other command takes that id as `--partition`.

```sh
store=$(mktemp -d)

# 1. Provision a partition (retention and encryption are chosen once, never defaulted)
part=$(bitchain --store "$store" init --subject-kind household --subject-id demo \
         --retention standard --encryption plaintext 2>/dev/null)

# 2. Pack a file or directory; prints the manifest hash
hash=$(bitchain --store "$store" pack ./my-files --partition "$part" 2>/dev/null)

# 3. Check integrity, then restore byte-for-byte
bitchain --store "$store" verify --partition "$part"
bitchain --store "$store" unpack "$hash" ./restored --partition "$part"
diff -r ./my-files ./restored && echo identical
```

`--store <path>` (default `~/.bitchain/store`) is a global flag.

## Commands

```text
bitchain init   --subject-kind <person|household|service> --subject-id <s>
                --retention <ephemeral|standard|durable>
                --encryption <plaintext|sealed-required> [--label <s>]
bitchain pack   <path> [--partition <id>] [--identity <s>]
                [--fixed-block-size <n> | --cdc-min <n> --cdc-avg <n> --cdc-max <n>]
bitchain unpack <manifest-hash|path> <output-dir> [--partition <id>]
bitchain verify [<manifest-hash>] [--partition <id>] [--all]
bitchain ls     [<partition>]
bitchain gc     [<partition>] [--target-utilization <pct>]
bitchain push   <local-store> <remote-dir> [--partition <id>] [--pack <hex>]...
bitchain pull   <remote-dir> <local-store> [--partition <id>] [--pack <hex>]...
```

Every command, its flags, exit codes and real output are in
[`docs/cli-reference.md`](docs/cli-reference.md). `pull` into a fresh store
needs the same `init` run there first; see the note under `pull`.

## Library

```rust
use bitchain::{Manifest, PartitionStore};
```

The CLI is a thin client over the library, so every command is reachable
through the public API without a subprocess. The storage-engine types
(`addressing`, `fragment`, `store`) are re-exported from `storage_kit`.

## Documentation

- [`docs/concepts.md`](docs/concepts.md): content addressing, determinism, dedup, partitions, why packfiles.
- [`docs/manifest-format.md`](docs/manifest-format.md): the v2 manifest, field by field.
- [`bitchain-schema.json`](bitchain-schema.json): JSON Schema (draft-07) for the v2 manifest.
- [`docs/cli-reference.md`](docs/cli-reference.md): every command with real transcripts.

## Development

```sh
cargo build
cargo test
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
```

No system dependencies and no private registry: everything builds from
crates.io. See [CONTRIBUTING.md](CONTRIBUTING.md).

## Security

Report vulnerabilities privately; see [SECURITY.md](SECURITY.md).

## License

Source code under [Apache-2.0](LICENSE).

## About

bitchain is part of the **SlashBuilder Kit ecosystem**, a set of reusable
Rust and Dart libraries built around the BeOS-style "Kit" pattern. This tool
is a product composed from Kits: it uses `slashbuilder-storage-kit` for
storage.

Maintained by [SlashBuilder](https://slashbuilder.com) / Douglas Lockamy.
