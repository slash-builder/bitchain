# Core concepts (format v2)

> **Pending relocation, not yet executed:** data-architect ruled
> 2026-08-23 that the lib layer described below (`addressing.rs`,
> `fragment.rs`, `store/`) is slated to move into `clusterzer0/storage-kit`
> verbatim, with this crate depending on it rather than reimplementing the
> format directly. See `AGENT.md`'s "Open work" section for the full
> ruling. The invariants below don't change; the file paths will once the
> move executes — re-verify against `AGENT.md` before trusting a path
> reference in this document.

**CLUS-20.** These are the invariants the shipped code actually enforces
— each claim below is backed by a specific source file and, where it
matters, a test that exercises it. Locked by data-architect; see
`AGENT.md` for the full engineering contract this document explains.

## Content addressing via BLAKE3

Physical storage is keyed by **content hash alone** — `Hash::of(bytes)`
(`src/addressing.rs`) is BLAKE3 over the raw bytes, and that hash is the
only thing dedup and the packfile layer ever look at. A block's address
*is* its content; there is no separate "block ID."

BLAKE3 is a replacement for the retired v1 design's SHA-256, chosen for
the Lore-alignment lock (`2026-06-bitchain-storage-aligned-with-lore.md`)
— not a constitutional choice. `AGENT.md` is explicit that the actual
invariant is content-addressed determinism; BLAKE3 is this version's
implementation of it, and a future hash-function change would be a format
version bump, not a redesign.

## `(hash, context)` addressing

A block's physical key is its hash. Its **logical identity** — "this is
`photo.jpg` for household X" as opposed to "this is 4KB of anonymous
bytes that happen to match" — is a separate 16-byte `Context`, layered on
top (`Address { hash, context }`).

`Context` is assigned once, at ingest, from
`BLAKE3(logical-identity-string)[0:16]` (`Context::derive`), and is
immutable — a new logical identity is a new context, never a mutation of
an old one. Fragmentation-internal nodes (leaves and list-nodes produced
while chunking a file — see below) have no logical identity of their own
and always use the reserved all-zero `Context::ANONYMOUS`.

This split matters in practice: two different logical objects that
happen to hash to the same bytes are still distinguishable by context at
the manifest layer, while storage itself still dedups on hash alone — you
get both "know what this file logically is" and "never store the same
bytes twice," without one compromising the other.

## Determinism via left-to-right tree construction

`build_fragment_tree` (`src/fragment.rs`) splits content into leaves via a
configurable `ChunkingProfile` (fixed-size or FastCDC), then folds leaves
into **list-nodes** at a fixed `FANOUT = 1024` — a determinism lock, not a
tunable — until a single root remains. A list-node's body is nothing but
its children's `hash (32B) | uncompressed_len (8B)` records, back to
back, in order.

The build is always left-to-right and depth-uniform (every leaf ends up
at the same depth for a given input), so identical bytes chunked under an
identical profile always produce an identical tree — proven directly by
`identical_bytes_same_profile_yield_identical_root_hash`, which builds the
same input into two independent stores and asserts the roots, depths,
and root types match exactly.

`RootType` is `leaf` (content fit in a single chunk, no fold levels) or
`list` (one or more fold levels above the leaves) — these are the two
literal serialized values; the manifest's `depth` field records how many
list-node levels sit above the leaves, needed because reconstruction
would otherwise have to guess how many levels to walk.

## Dedup via identical content

Because storage keys on hash alone, two logically distinct entries that
happen to be byte-identical automatically share one physical copy —
confirmed directly: packing two identical 200 KB files into the same
partition produces two manifest entries with different `path`s and
`context`s but the identical `root_hash`, and `put_dedups_within_partition`
covers the same behavior as a unit test. Dedup happens naturally as a
consequence of content addressing; there is no separate dedup pass or
dedup index to maintain.

Dedup is **not** cross-partition. Partitions are isolated subtrees on
disk (see below); the same bytes packed into two different partitions are
stored twice, once per partition, by design.

## Partition isolation (household scope)

A `PartitionId` is a 16-byte physical storage boundary — canonically
`BLAKE3(household/namespace-string)[0:16]` (`PartitionId::derive`). Every
`PartitionStore` owns exactly one partition's on-disk subtree
(`<store_root>/<partition_id_hex>/...`) and never reads or writes outside
it. No pack file spans more than one partition; GC, dedup, and the
sealed-pack index are all scoped per partition — confirmed by
`partitions_are_isolated_directories` and
`gc_in_one_partition_never_touches_another`.

This is the mechanism, not yet a policy statement about what a
"household" is at the product layer — see `docs/integration-quickring.md`
for how Courier is expected to map its own concept of a household onto
this primitive.

## Why packfiles (append-only, local-first)

Each partition keeps one **active** pack (`active/active.pack`, append-only,
no trailer, crash-safe by construction — a sequential scan on reopen
simply stops at the last complete entry) plus zero or more **sealed**
packs (`packs/<xx>/<pack_id>.pack`, immutable once written, named by
their own content hash). An entry inside a pack is
`hash (32B) | codec (1B) | flags (1B) | uncompressed_len (8B) |
stored_len (8B) | payload`; a sealed pack's trailer records `entry_count`
and a `pack_content_hash` — the BLAKE3 hash of every byte before the
trailer — which `verify` re-derives and compares.

Each pack also has a derived `.idx` file (git-pack-style: rebuildable
from the `.pack` alone, never treated as a source of truth — losing an
`.idx` is a `verify`/rebuild-away, not data loss; `PartitionStore::open`
self-heals a missing or unreadable index automatically).

An active pack seals once it crosses a **soft cap**
(`SEAL_CAP_BYTES = 128 MiB`), or on demand when `gc` runs (which
force-seals so its mark-and-sweep sees a stable, fully-indexed set). This
is why `push`/`pull` — which only ever transfer *sealed* pack files — can
report "0 packs" moved even when a partition holds real, verifiable
content: nothing has crossed the seal boundary yet. See
`docs/cli-reference.md`'s `push`/`pull`/`gc` sections for the exact
behavior this produces.

The whole design is **append-only and local-first by construction** — no
command in this CLI requires a server, an account, or network access;
every example in `docs/cli-reference.md` runs against nothing but the
local filesystem.

## Compression (Zstd, hash-of-uncompressed)

Payloads are Zstd-compressed at level 3 where it helps, falling back to
raw storage otherwise (`Codec::Raw` / `Codec::Zstd`, `src/store/packfile.rs`).
Critically, **the content hash is always computed over the uncompressed
bytes** — dedup and identity survive a future change in compression
strategy or level, since the address never depends on how (or whether)
the payload happens to be compressed on disk.

## What's deliberately out of scope for v1

Documented here rather than silently omitted, per `AGENT.md`'s own
framing of the CLI's boundaries:

- **No remote server state, subscriptions, licensing, quotas, user
  accounts, or authentication.** That's explicitly Refraction's job, not
  this CLI's.
- **No networked HTTP/S3 backend wired into the CLI.** `push`/`pull` are
  local-filesystem-to-local-filesystem only in the reference CLI;
  networked transports are `storage-kit`'s job via the same
  `ImmutableStore` trait, for programmatic consumers to reach directly.
- **No legacy (v1, SHA-256, fixed-1MB, flat-JSON) manifest read path.**
  Open decision, escalated to DJ — not decided, not defaulted.
- **No creation timestamp on the manifest.** See
  `docs/manifest-format.md`'s "Gap" section.
- **No continue-on-error verification.** `verify` is strict fail-fast, not
  a full-scan corruption report. See `docs/cli-reference.md`'s `verify`
  section.
