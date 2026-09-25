# bitchain — Agent Context
# slash-builder/bitchain
#
# Paste this file into your AI's system prompt to work on this repo.
# ─────────────────────────────────────────────────────────────────────────────

You are working on **bitchain** — the reference CLI product built on
**Storage Kit**, a Rust library + CLI for content-addressed binary storage,
created by Douglas Lockamy (DJ) at Lockamy Studios.

**Content-addressed determinism is the core invariant** — the same bytes in
the same partition resolve to the same address, and stored data is immutable
once written. Local-first, offline-first design. Never suggest changes that
compromise either invariant. **Correction, 2026-09-23** (see "Addressing"
below): the address is `(partition_id, content_id)`, not `(hash, context)` —
`context` is a property of a manifest reference, not of the address itself.

---

## Repo home (moved 2026-08-23)

bitchain now lives at **`github.com/slash-builder/bitchain`** — transferred
out of `clusterzer0` as part of the SlashBuilder OSS-org restructuring.
SlashBuilder is the studio's open-source organization (peer to Hearth and
BenixOS); bitchain is stewarded there as an open protocol + reference
tooling. **Correction, 2026-09-23: `storage-kit` did move, and the
"remains at `clusterzer0/storage-kit`" claim below is stale.** The
`clusterzer0` org was dissolved 2026-08-23 — both repos are `slash-builder`
siblings now (`github.com/slash-builder/storage-kit`; confirmed by that
repo's own `Cargo.toml` `repository` field and by this crate's `Cargo.toml`
dependency comment: "Both are same-org siblings under slash-builder now").
This is no longer a cross-org dependency requiring active tracking for the
reason stated below — that reason no longer applies, though the Nexus
registry dependency itself (`storage-kit = { version = "2.0", registry =
"lockamy" }`) is unaffected and still real.
<details><summary>Original claim, kept for provenance, not deleted</summary>

"`storage-kit` did not move — it remains at `clusterzer0/storage-kit` (the
developer-tools BU). bitchain (SlashBuilder) depending on storage-kit
(clusterzer0) is a deliberate cross-org dependency, not a mistake — see
'Open work' below for why this needs active tracking."
</details>

License: **Apache-2.0** (confirmed correct in `Cargo.toml` as of the
2026-08-23 repositioning commit — the prior `MIT` mismatch is resolved).

---

## What bitchain is

**The reference CLI** for content-addressed binary storage — splits files
into immutable, content-addressed fragments and reconstructs them on demand.
Think of it as:

- **A standalone toolkit** (library + CLI) for systems that need
  content-addressed binary storage, usable with zero knowledge of Quickring
  or any other studio product.
- **Deployed by:** Quickring Courier (out-of-band file sharing, QR-16),
  Refraction (managed bitchain server), Thunderhead devices (block cache +
  local storage).
- **Local-first, always.** Works with zero servers and zero accounts.
  Refraction is an optional backend, never a dependency.

---

## Storage engineering — format v2, locked (Lore-aligned)

**Locked 2026-06** via `softsurve/sol/weekly/2026-06-bitchain-storage-aligned-with-lore.md`,
confirmed and formalized by data-architect. Supersedes the old SHA-256 /
fixed-1MB-block / flat-JSON-manifest design entirely — this is not an
incremental change, it's a substitution of every low-level storage
primitive:

| Layer | Format v2 |
|---|---|
| Hash function | **BLAKE3** |
| Chunking | **FastCDC + fixed-size hybrid** — content-defined by default, fixed-size where canonical addressing matters; application chooses per content type |
| Compression | **Zstd** (hash is of *uncompressed* content — dedup survives compression choice) |
| On-disk format | **Packfile** — append-only files holding many fragments, mmappable index, fanned out across the filesystem |
| Addressing | **`(partition_id, content_id)`** — the storage engine's whole address, 16-byte partition id + 32-byte BLAKE3 content id; `storage-kit` deduplicates on `content_id` alone and knows nothing of logical identity |
| Access boundary | **Partitions** — 16-byte opaque identifier; storage subsystem enforces per-partition isolation |
| Large-file handling | **Recursive fragmentation** — an oversized fragment list is itself fragmented as a content-addressed object, flagged as a list, fetched lazily |

**Superseded, 2026-09-23: `(hash, context)` addressing.** The 2026-06 lock
named `(hash, context)` as the format's addressing pillar — 32-byte BLAKE3
hash + 16-byte opaque context, "the address is a pair, not a bare digest."
Two independent rulings retire that at the engine layer and amend the lock,
not overturn it:

- `wiki/decisions/storage-addressing-hash-context-ruling-2026-09-23.md` (data-architect)
- `wiki/decisions/storage-addressing-hash-context-ruling-security-2026-09-23.md` (security-engineer)

Both read the lock's own clause — "same bytes, different contexts = same
payload deduplicated under one hash but distinct identities" — as *requiring*
hash-only (now partition-scoped, keyed) dedup, not merely tolerating it.
`Context` was never keyed into `ImmutableStore`; it is a property of a
**reference**, not of a stored object, and `bitchain`'s own
`ManifestEntry { path, context, root_hash }` is where it has always actually
lived (see `src/context.rs`, which now owns the type). Do not resurrect a
`context`/`Address` composite key in `storage-kit` on the strength of the
old table row above — it is superseded, and re-adding it would violate the
lock's dedup clause and reopen a cross-partition correlator the rulings both
name explicitly (§3.1/§2.1 of the two documents above).

**Do not describe this as "SHA-256 immutability."** That framing is
superseded. The invariant that actually holds is content-addressed
determinism (above) — BLAKE3 is this version's implementation of it, not a
constitutional choice.

**What is unchanged:** the multi-URI peer-to-peer trust model. A bitchain
manifest carries, per fragment, a list of candidate URIs
(`s3://`, `https://`, `file://`, eventually `peer://`); consumers try each
in order. This is bitchain's distinctive contribution over Lore (which
explicitly does not solve peer discovery) and is preserved unchanged by the
v2 storage rewrite.

**Migration:** legacy manifests (SHA-256, fixed-size 1 MB, flat block list)
are a different format. Whether the v2 CLI reads them (`--legacy` flag or
similar) or is v2-only is an **open decision, escalated to DJ** — this
touches the hosted tier's existing data and is not a call this repo can
default silently. Do not assume either answer in code or docs.

---

## CLI scope (v2, shipped — updated 2026-08-23)

The CLI is a **reference client**, not the only interface. All seven
commands below are implemented, compiling, and round-trip tested as of
2026-08-23 — see `docs/cli-reference.md` for every command's real flags
and a live transcript, not the placeholder positional-arg sketch this
section used to carry:

```
bitchain pack    --input <path> --partition <string> [...]   # split, hash, chunk, compress → packfile + manifest
bitchain unpack  --manifest <path> --output-dir <path>        # fetch, decompress, reassemble; length-checked
bitchain verify  --partition <string>                         # walk sealed packs + entries, re-hash, confirm integrity
bitchain push    --partition <string> --to <path>              # copy sealed pack(s) to another *local* store root
bitchain pull    --partition <string> --from <path>            # copy sealed pack(s) in from another *local* store root
bitchain ls      --partition <string>                          # list a partition's sealed packs + manifests
bitchain gc      --partition <string>                          # mark-and-sweep; also the only way to force-seal
```

**Added 2026-09-24: `bitchain init`.** storage-kit 3.0.1 split
`PartitionStore::create` from `open` (`open` no longer provisions on
first use), and this crate had no command anywhere that ever called
`create` — every store-opening command failed closed with `partition not
provisioned: <id>` from the very first use. Fixed with an explicit `init`
subcommand (DJ's ruling, `context/hot-decisions.md`, "One account,
everything — sixteen rulings"), not auto-provisioning with defaults:

```
bitchain init --subject-kind <person|household|service> --subject-id <string> \
  --retention <ephemeral|standard|durable> --encryption <plaintext|sealed-required> [--label <string>]
```

All four choices are required flags with no default — see `src/cli/init.rs`
and `docs/cli-reference.md`'s `## init` section for the full contract.
Every other store-opening command's `partition not provisioned` error now
names `bitchain init` in its message (`src/main.rs`'s single error-reporting
point, not duplicated per command).

This is a **redesign, not a rename** of the old `ingest` / `rebuild` /
`show` / `validate` surface — `push`/`pull`/`ls`/`gc` are new primitives
that didn't exist in v1 (git-shaped, not single-shot ingest/rebuild).
**Scope note found during verification, not in the original target
sketch: `push`/`pull` are local-filesystem-to-local-filesystem only in
the shipped v1 CLI — no S3 backend is wired into the CLI itself.**
Networked HTTP/S3 remotes are `storage-kit`'s job (`S3Backend`/
`HttpBackend` against the same `ImmutableStore` trait), for programmatic
consumers to use directly — see `docs/cli-reference.md`'s `push`/`pull`
sections. `diff` is still a plausible v1.1 addition; still not shipped.
There is also no standalone `seal` command — sealing is implicit (size
cap) or forced via `gc` — see `docs/concepts.md`.

CLI does NOT manage: remote server state, subscription/licensing/quotas,
user accounts or authentication (Refraction's job).

---

## Open work (as of 2026-08-23 — verify before trusting any of this is done)

- **RESOLVED 2026-08-23: the crate compiles and the v1→v2 CLI rewrite is
  wired end to end.** An earlier same-day note here (superseded, not
  deleted — see below) said the crate did not compile because
  `src/main.rs` still implemented the old `ingest`/`rebuild` surface
  against a removed dependency set. That has been replaced: `src/lib.rs`,
  `src/addressing.rs`, `src/fragment.rs`, `src/error.rs`, `src/gc.rs`,
  `src/manifest.rs`, `src/store/` (`mod.rs` + `packfile.rs`), and
  `src/cli/` (`pack.rs`, `unpack.rs`, `verify.rs`, `ls.rs`, `gc.rs`,
  `push.rs`, `pull.rs`, `transfer.rs`, `mod.rs`) are all present, and
  `src/main.rs` now declares `mod cli;` and dispatches every subcommand
  through it. Verified independently (software-developer, CLUS-20 pass,
  2026-08-23): `cargo build`/`cargo fmt --check`/`cargo clippy
  --all-targets` are all clean, `cargo test` passes 23/23, and a live
  `pack` → `ls` → `verify` → `unpack` → `gc` → `push` → `pull` → `unpack`
  round trip against real files (including a >1-chunk 5 MB file, forcing
  a list-node level, and a directory with a deduplicated file pair)
  produced byte-identical output at every restore, plus a corruption test
  (flip one byte in a sealed pack) that `verify` correctly caught and
  exited non-zero on. That same pass also fixed a real bug in `verify`'s
  entry check (it compared a raw content hash against a multi-chunk
  entry's list-node root hash — a type mismatch that always failed
  depth>0 entries even on a byte-correct reconstruction) and fixed
  several CLI modules that had regressed to an earlier arg shape
  (`gc`/`unpack`/`push`/`transfer`) after a same-day concurrent edit.
  Full detail and every command's real transcript: `docs/cli-reference.md`,
  `docs/concepts.md`, `docs/manifest-format.md`.
  **Superseded note, kept for provenance, not deleted:** "Rewrite is in
  progress and, as of this writing, the crate does not compile.
  `Cargo.toml` was updated to the v2 dependency set (`blake3`, `zstd`,
  `fastcdc`; `sha2` removed) but `src/main.rs` still imports
  `sha2::{Digest, Sha256}` and implements the old `ingest`/`rebuild`
  command surface end to end. `src/lib.rs`, `src/block.rs`,
  `src/manifest.rs`, and `src/store/` have all been removed and not yet
  replaced."
- **RULED 2026-08-23 (data-architect, via `projects/clusterzer0.md` in the
  shared context repo — not yet executed here): the v2 lib layer
  (`src/addressing.rs`, `src/fragment.rs`, `src/store/`) is slated to
  relocate into `storage-kit`, not stay in this crate long-term.** The
  question below ("does bitchain depend on storage-kit, or reimplement
  it?") is answered: neither, as shipped — `bitchain` currently
  reimplements the format directly, and the ruling is that this is a
  *location* defect, not a *shape* defect (the v2 engine here is a
  faithful, non-stub implementation of the 2026-06 storage-alignment
  lock; `storage-kit` is still on v1/`sha2`). The lib-layer modules get
  moved into `storage-kit` verbatim; `bitchain`'s CLI then depends on
  `storage-kit` and keeps only its command surface + the multi-URI/
  manifest protocol layer. Two reconciliations noted in the ruling: pick
  one trait-naming convention (the lock said `ImmutableStore`/
  `MutableStore`; the shipped code says `ReadBlock`/`WriteBlock`), and
  `storage-kit`'s v1 `sha2` scaffold is superseded by the relocated v2
  modules, not merged with them. Routed to software-developer for
  execution (sequencing is bitchain-pm's, CLUS-21/CLUS-1). **Not done as
  of this writing** — `Cargo.toml` still pulls `blake3`/`fastcdc`/`zstd`
  directly with no `storage-kit` reference, and `docs/concepts.md`/
  `docs/cli-reference.md`/`docs/manifest-format.md` (CLUS-20) document the
  code at its current, pre-relocation location. Re-verify those docs'
  file-path references once this executes.
  **DONE as of 2026-09-21 (correction, 2026-09-23 — the "not done" note
  above and the "RESOLVED 2026-08-23" bullet's `src/store/` listing are both
  stale).** `src/lib.rs`'s own header now reads: "the storage ENGINE lives
  in `storage-kit` and is re-exported here, not reimplemented" and "[u]ntil
  2026-09-21 this crate carried its own drifted COPIES of these files."
  Verified directly against the tree on this branch: `src/addressing.rs`,
  `src/fragment.rs`, and `src/store/` **do not exist in this crate**;
  `Cargo.toml` pulls `storage-kit = { version = "2.0", registry = "lockamy"
  }` from Nexus, and `bitchain::{addressing, fragment, store}` are
  `pub use storage_kit::{...}` re-exports. **`src/store/` is not bitchain's
  own** — do not describe it as such in future doc passes; this crate now
  owns only `src/manifest.rs`, `src/gc.rs`, `src/context.rs` (new,
  2026-09-23 — see below), and `src/cli/`. `docs/concepts.md`,
  `docs/cli-reference.md`, and `docs/manifest-format.md` still describe the
  pre-relocation file layout in places and are not corrected by this pass —
  flagged, not fixed, since doc-path cleanup here is out of this change's
  scope.
  <details><summary>Original open question, kept for provenance</summary>

  Architecture question, not yet answered: does bitchain depend on
  `storage-kit` as a library crate, or is the v2 storage engine being
  implemented directly inside the `bitchain` crate? `bitchain`'s
  `Cargo.toml` pulls `blake3`/`fastcdc`/`zstd` as **direct** dependencies
  with no `storage-kit` path/git dependency anywhere. The Kit model lock
  and the 2026-06 storage-alignment weekly both say bitchain "switches the
  storage primitives via storage-kit" — i.e., storage-kit is supposed to be
  the lib layer bitchain links against, not logic bitchain reimplements.
  `storage-kit`'s own `Cargo.toml` hasn't been touched for v2 at all yet
  (still `sha2`, no `blake3`/`fastcdc`/`zstd`/`memmap2`). Escalated to
  software-developer and data-architect: confirm the lib/CLI split before
  more v2 code lands in either repo, or CLUS-21 (monorepo + lib/cli
  split) silently regresses into a fork.
  </details>
- **`Context` retired from `storage-kit`, owned by `bitchain` (2026-09-23).**
  Two independent rulings — see the "Addressing" correction above — retire
  `Context`/`Address` from the storage engine and move the logical-identity
  half of the 2026-06 lock down to the reference layer, where
  `ManifestEntry` already carried it. `src/context.rs` (new) is now the
  canonical `Context` type: same derivation
  (`BLAKE3(logical-identity-string)[0:16]`), same representation, same
  `ANONYMOUS` sentinel, moved down the dependency edge rather than
  reimplemented differently. `storage-kit`'s own internal `Context`/`Address`
  (used by `FragmentTree::root`, always `ANONYMOUS` for fragmentation
  internal nodes) are **not yet removed from `storage-kit` itself** — that
  is additive, in-scope-for-a-later-PR work on `storage-kit`, not this repo.
  The JSON manifest schema (`bitchain-schema.json`) is unchanged; existing
  manifests still verify.
- **Backwards-compat (legacy manifest read) — escalated to DJ.** See above.
- **Local git remote hygiene:** confirm any local clone or CI credential
  still pointing at `clusterzer0/bitchain.git` is updated —
  `git@github.com:slash-builder/bitchain.git` is now canonical (GitHub's
  transfer redirect covers the old URL for now, but don't rely on it
  long-term).

---

## Stack

- Language: Rust (edition 2021)
- CLI framework: `clap` v4 with derive macros
- Hashing: `blake3` (replaces `sha2`)
- Chunking: `fastcdc`
- Compression: `zstd`
- Serialization: `serde` + `serde_json`
- License: **Apache-2.0**

Full CI/deploy stack (Jenkins → Nexus) unchanged by the org move; see
`Jenkinsfile`.

---

## Related work

- **`slash-builder/storage-kit`** — the library layer bitchain consumes
  (same-org sibling since the `clusterzer0` org dissolution, 2026-08-23; see
  the "Repo home" correction above — this was `clusterzer0/storage-kit`).
- **`clusterzer0/refraction`** — self-hosted/managed storage backend
  consuming the same storage primitives. (Not independently verified by this
  pass — only `storage-kit`'s location was confirmed against its own
  `Cargo.toml`. If `refraction` also moved, this line is stale too.)
- **Quickring Courier / QR-16** — out-of-band file sharing; the Rust
  binding (`bitchain-sys`) is gated on storage-kit's trait shape
  stabilizing, not on bitchain CLI completeness.
- **`epicgames/lore`** — third-party reference clone (not owned), cloned
  for engineering-alignment reading only. No tickets filed against it, no
  ownership implied.

---

## Jira

Project: **CLUS** (clusterzer0) — fairmerce.atlassian.net. (Jira project
key is unaffected by the GitHub org move.)

- **CLUS-1** Storage Kit
- **CLUS-20** bitchain spec — **timeline re-estimate delivered 2026-08-23**;
  do not quote any older date as live. See `projects/clusterzer0.md`.
- **CLUS-21** Monorepo + lib/cli split — status now depends on resolving
  the storage-kit dependency question above.
- **CLUS-22** storage-kit stability / conformance suite.
- **QR-16** Quickring's bitchain integration — consumer of CLUS-20, gated
  on implementation timeline now, not on design decisions.

---

## Key invariants

1. **Content-addressed determinism** — same bytes + same partition → same
   address; stored data is immutable once written. BLAKE3 is this version's
   hash function, not a constitutional choice. **Correction, 2026-09-23:**
   the address is `(partition_id, content_id)`, not `(hash, context)` — see
   "Addressing" above. `context` is a property of the manifest reference
   that names an object, not of the object's storage address.
2. **Offline-first** — works without internet; every backend beyond local
   filesystem is optional.
3. **Reusable library** — the CLI is a thin reference client over a stable
   lib API. (The storage-kit dependency resolution this line used to say was
   "pending" is done — see the CLUS-21 correction under "Open work" above.)
4. **Specification-first** — the manifest/address format is language-neutral;
   Rust is the reference implementation.
5. **Local-first is a gate, not a preference** — no command may require a
   Refraction server to function.
