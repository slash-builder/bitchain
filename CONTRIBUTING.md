# Contributing to bitchain

Thanks for your interest. Contributions are welcome: bug reports, feature requests, documentation improvements and code patches.

## Quick reference

| Item | Value |
|---|---|
| Licence | Apache-2.0 |
| Sign-off | `git commit -s` (Developer Certificate of Origin) |
| Required approver | A code owner: see [`.github/CODEOWNERS`](.github/CODEOWNERS) |
| CI | GitHub Actions, on every pull request |
| Conduct | [Contributor Covenant 2.1](CODE_OF_CONDUCT.md); reports to `conduct@slashbuilder.com` |
| Security | [SECURITY.md](SECURITY.md); never a public issue |

## Before you start

Open an issue first for substantial changes: new commands, flag changes, new dependencies, anything that touches the
manifest shape. Small changes (typos, doc fixes, small bug fixes) can go straight to a pull request.

### Invariants worth a design discussion

- **The manifest format** (`src/manifest.rs`, `bitchain-schema.json`, `docs/manifest-format.md`). The three move
  together, by hand; there is no code generation. A change to one without the others is a bug.
- **Determinism.** Identical bytes under an identical chunking profile must produce an identical tree and root hash.
- **Partitions are isolated.** Dedup and GC never cross a partition boundary.
- **The CLI stays a thin client over the library.** A new command should be reachable through the public API.
- **No server, account or network concerns** in this CLI. Networked backends belong in `slashbuilder-storage-kit`.

## Local development

```sh
cargo build
cargo test
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo run -- --help          # try the CLI
```

Requires Rust 1.88 or newer. No other system dependencies, and no private registry: all dependencies come from
crates.io.

## Code style

- `cargo fmt --all` must produce no diff.
- `cargo clippy --all-targets --all-features -- -D warnings` must pass with zero warnings.
- A behaviour change comes with a test.

## Continuous integration

Every pull request runs formatting, clippy, docs, tests on Linux (x86-64 and arm64), macOS and Windows, the
minimum-supported-Rust check, `cargo-deny` (advisories, licences, sources), `cargo-semver-checks`, a hygiene check, and
a DCO check. The same pipeline runs weekly to catch new advisories.

Releases are automated with [release-plz](https://release-plz.dev): pushes to `main` open or update a release pull request that bumps the version and changelog from Conventional Commit titles (`feat:`, `fix:`, a `!` for a breaking change) and checks the bump with `cargo-semver-checks`. Merging that pull request releases the crate: a tag `vX.Y.Z` equal to the crate version, published to crates.io by trusted publishing from a protected environment.

## Pull request process

1. Fork the repository and create a branch from `main`.
2. Make your changes and sign off each commit with `git commit -s`.
3. Run the commands above and confirm they pass.
4. Open a pull request against `main` and address review feedback. A code owner must approve before merge.
5. The maintainer merges (squash by default).

## Sign your commits (DCO)

This project uses the [Developer Certificate of Origin 1.1](https://developercertificate.org/).
By adding a `Signed-off-by` line you certify that you wrote the change or have the right to submit it
under the project's licence (Apache-2.0).

```sh
git commit -s -m "your message"      # adds: Signed-off-by: Your Name <you@example.com>
git rebase --signoff main            # add the line to commits you already made
```

Use your real name and a working email address. A check on every pull request fails if any
non-merge commit is missing the line.

Under section 5 of the Apache License 2.0, a contribution you submit is licensed under the same
terms as the project unless you state otherwise. There is no separate contributor agreement.

## Code of Conduct

This project follows the [Contributor Covenant](CODE_OF_CONDUCT.md). Be kind, assume good intent, and report problems to **conduct@slashbuilder.com**.

## About

bitchain is part of the **SlashBuilder Kit ecosystem**. See the [README](README.md) for the project overview.
