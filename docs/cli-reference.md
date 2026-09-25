# CLI reference (format v2)

> **Pending relocation, not yet executed:** every command's *behavior* and
> *flags* documented below are verified against the shipped CLI as of
> 2026-08-23. The underlying lib layer these commands are thin wrappers
> over may relocate from this crate into `clusterzer0/storage-kit` per an
> escalated, not-yet-resolved architecture question — see `AGENT.md`'s
> "Open work" section. This session deliberately kept storage primitives
> inlined in `bitchain` rather than wait on that decision (a pragmatic
> "ship it, refactor later" call, not a resolution of the question).
> Nothing here changes for a CLI user if/when that move happens; it
> matters only if you're reaching for the library API directly.

**CLUS-20.** Every command below was verified against a real `cargo build
--release` of `slash-builder/bitchain` and a live round trip on
2026-08-23 — `pack` (a directory with a deliberate duplicate file) → `ls`
→ `verify` → `unpack` (byte-identical restore, confirmed via `diff -r`) →
`gc` → `push` → `pull` into a fresh store root → `unpack` from the pulled
store (byte-identical restore, confirmed again) — plus a dedicated
corruption test (flip one byte in a sealed `.pack`, confirm `verify`
catches it and exits non-zero). `cargo test` passes: 23/23 unit tests.
`cargo fmt --check` and `cargo clippy --all-targets` are both clean.
Every example command and its output below is a real transcript, with
only temp-directory paths shortened for readability.

## Global flag

```
bitchain --store <path> <command> ...
```

`--store` sets the store root for `pack`/`unpack`/`verify`/`ls`/`gc`.
Default: `~/.bitchain/store` (`cli::default_store_root()`). **`push` and
`pull` ignore `--store`** — they take their store roots as explicit
positional arguments instead (`<local_store>`/`<remote>`), since a
replication command inherently has two store roots, not one. There is no
config file, no account, no server dependency — every command below runs
fully offline against the local filesystem.

## Partition argument

Every command accepts a partition, either as `--partition <string>`
(`pack`, `unpack`, `verify`) or as an optional positional
(`ls [<partition>]`, `gc [<partition>]`), plus `--partition` on
`push`/`pull`. It accepts either a raw 32-hex-char partition id or any
other string, which is derived into a partition id as
`BLAKE3(string)[0:16]` (`resolve_partition` in `src/cli/mod.rs`).

**Defaults differ by command, on purpose:**
- `pack`/`verify` default to the partition `"default"` when `--partition`
  is omitted, so `bitchain pack x` / `bitchain verify` work with zero
  required flags.
- `unpack <hash>` defaults to that same `"default"` partition when
  `--partition` is omitted and `<hash>` looks like a manifest hash (64 hex
  chars) rather than a file path.
- `ls`/`gc`/`push`/`pull` treat "no partition given" as **"every
  partition under the store root"**, not `"default"` — these are
  whole-store operations by default, since a store commonly holds more
  than one partition.

**Format-v3 correction, 2026-09-24 (`storage-kit` 3.0.1):** the partition
string a command like `pack --partition <string>` resolves is only usable
once *some* partition has been provisioned at that id — `storage-kit` v3
split `PartitionStore::create` from `open`, and `open` no longer
provisions on first use. `resolve_partition`'s bare-string derivation
(`BLAKE3(string)[0:16]`, no domain-separator prefix) can never match a
partition `init` provisioned, since `init` always derives through a
`person:`/`household:`/`service:`-prefixed subject (see `## init` below)
— so in practice, every `--partition <string>` you pass to `pack`/
`unpack`/`verify`/`ls`/`gc`/`push`/`pull` after `init` should be the raw
32-hex-char id `init` printed, not a bare namespace string. Every
store-opening command fails closed with `partition not provisioned: <id>`
and names `bitchain init` in the error if you pass one that hasn't been
provisioned.

---

## `init`

```
bitchain [--store <path>] init --subject-kind <person|household|service>
  --subject-id <string> --retention <ephemeral|standard|durable>
  --encryption <plaintext|sealed-required> [--label <string>]
```

Provisions a new partition and prints its hex id to stdout. The **only**
bitchain command that ever calls `PartitionStore::create` — every other
command operates on a partition `init` already provisioned, and fails
with `partition not provisioned: <id>` if you skip this step. Every
choice is a required flag with no default, by DJ's ruling recorded in
`context/hot-decisions.md` ("One account, everything — sixteen rulings"):
the create/open split exists so retention class and encryption mode are
decided once, deliberately, and never auto-picked by the CLI.

- `--subject-kind`: `person`, `household`, or `service` — closed,
  forever, at exactly these three values. `service` is for a headless
  principal (e.g. CI) whose key cannot be a second wrap of a person's.
  Permanently out of scope: any segment value (`business`/`org`/`team`/
  `project`/`client`/`division`).
- `--subject-id`: hashed, with the subject kind as a domain-separator
  prefix, to derive the partition id
  (`PartitionId::for_person`/`for_household`/`for_service`) — not a raw
  `resolve_partition` string.
- `--retention`: `ephemeral` (GC may drop unreferenced content
  unprompted), `standard` (GC drops unreferenced content when asked to
  run), or `durable` (GC never drops content, referenced or not; only
  destroying the whole partition destroys its data). No default.
- `--encryption`: `plaintext` or `sealed-required`, set once and never
  changeable afterward — there is no setter. `plaintext` is the only
  value any command in this CLI can currently write to;
  `sealed-required` is reserved for when the key hierarchy lands.
- `--label`: free text, recorded only in the store-level, never-
  replicated `partitions.local.json` — never parsed for semantics.

**Real output:**

```
$ bitchain --store /tmp/bc-demo-store init --subject-kind household --subject-id demo-household --retention standard --encryption plaintext --label "demo household"
provisioned partition 82d5d64ceb79bd7f809db74d1ad29d61 (subject: Household:demo-household, retention: Standard, encryption: Plaintext, label: demo household)
pass --partition 82d5d64ceb79bd7f809db74d1ad29d61 to other commands to use this partition
82d5d64ceb79bd7f809db74d1ad29d61
```

Re-running `init` against the same `(subject-kind, subject-id)` fails
closed rather than silently reusing or altering the existing record:

```
$ bitchain --store /tmp/bc-demo-store init --subject-kind household --subject-id demo-household --retention standard --encryption plaintext
bitchain: partition already provisioned: 82d5d64ceb79bd7f809db74d1ad29d61
```

**Exit code:** `0` on success, printing only the partition's hex id to
stdout (progress/summary lines go to stderr, so
`partition=$(bitchain init ...)` captures exactly the id). `1` on
`PartitionAlreadyProvisioned` or any other failure.

---

## `pack`

```
bitchain [--store <path>] pack <path>
  [--partition <string>] [--identity <string>]
  [--fixed-block-size <bytes> | --cdc-min <u32> --cdc-avg <u32> --cdc-max <u32>]
```

Fragments a file, or every file under a directory (walked recursively via
`walkdir`, sorted by relative path for deterministic entry ordering), into
the store, seals the resulting pack(s), writes a manifest into the
partition's own `manifests/` directory keyed by the manifest's own content
hash, and prints that hash to stdout (progress lines go to stderr, so
`hash=$(bitchain pack ./x)` captures exactly the hash).

- `<path>`: a file or a directory.
- `--partition`: resolved per "Partition argument" above; default `default`.
- `--identity`: sets the whole pack's `Context` via `Context::derive`.
  Omitted means `Context::ANONYMOUS` (identity is content).
- `--fixed-block-size <bytes>`: use fixed-size chunking instead of
  FastCDC. Wins over `--cdc-*` if both are set.
- `--cdc-min` / `--cdc-avg` / `--cdc-max`: FastCDC parameters, default
  `4096` / `16384` / `65536`.

**`pack` always force-seals the active pack before returning** (unlike
earlier drafts of this CLI) — the manifest it just wrote is immediately
`push`-able with no separate `gc`/seal step required.

**Real output** (a directory containing `a.txt` and two byte-identical
copies of a 200 KB random file, `b.bin` and `b-dup.bin`):

```
$ bitchain --store /tmp/store pack ./src_dir
packed a.txt -> 83321ef49c28dcb63c6e3f46c9c0551f031200637d441a58d90dddf946c4b1a8 (17 bytes, depth 0)
packed b-dup.bin -> 31a1983d9b9b0998b32754c79f0a7b953274615a5808d1bd344f69b91c75f87a (200000 bytes, depth 1)
packed b.bin -> 31a1983d9b9b0998b32754c79f0a7b953274615a5808d1bd344f69b91c75f87a (200000 bytes, depth 1)
partition f78761cf8c3621e851097dff9a4b0463 — 3 file(s) packed
be98f1fdc2d802f199a5ab78f297ec1b510d6b92f926bbdda766e754dfd637b6
```

`b.bin` and `b-dup.bin` share the identical `root_hash` — dedup at pack
time, confirmed by `ls` reporting `total_entries=13` rather than double
that (200 KB fixed/FastCDC-chunked twice would be ~26 fragments, not 13,
if the duplicate weren't deduped).

**Exit code:** `0` on success, printing only the manifest hash to stdout.
On failure (bad input path, I/O error), prints `bitchain: ...` to stderr
and exits `1`.

---

## `unpack`

```
bitchain [--store <path>] unpack <hash> <output_path> [--partition <string>]
```

Resolves `<hash>` to a manifest — either directly as a path to a manifest
JSON file, or (if it looks like 64 hex characters) as
`<store>/<partition>/manifests/<hash>.json` — then reconstructs every
entry's fragment tree and writes it to `<output_path>/<entry.path>`.
`<output_path>` is always treated as a directory root, for both
single-file and multi-file (directory) packs, so the two cases
reconstruct identically.

**Correctness checks:** after reconstructing an entry's bytes, `unpack`
compares the reconstructed length against the manifest's recorded
`uncompressed_len` and fails with a descriptive error if they don't
match. Content-addressed identity is additionally enforced at the storage
layer on the way there — every block `ReadBlock::get` returns is hash-
checked against its own recorded hash before it's handed back, so a
corrupt fragment fails during reconstruction, not silently.

**Real output:**

```
$ bitchain --store /tmp/store unpack be98f1fdc2d802f199a5ab78f297ec1b510d6b92f926bbdda766e754dfd637b6 ./restored
restored ./restored/a.txt (17 bytes)
restored ./restored/b-dup.bin (200000 bytes)
restored ./restored/b.bin (200000 bytes)
3 file(s) restored
```

`diff -r ./src_dir ./restored` on the above produced no output —
byte-identical.

---

## `verify`

```
bitchain [--store <path>] verify [<hash>] [--partition <string>] [--all]
```

Two modes:

- **With `<hash>`:** resolves it to a manifest and checks just that one —
  re-derives the manifest's own content hash (must equal `<hash>`), and
  for every entry, reconstructs its bytes, then **re-chunks and re-hashes
  them from scratch** with the entry's recorded `chunking_profile`,
  comparing the resulting `(root_hash, root_type, depth)` against what the
  manifest recorded. This is a genuine determinism check, not a bare
  content-hash comparison — for a multi-chunk entry, `root_hash`
  addresses the *top list-node*, not `BLAKE3(reconstructed_bytes)`, so
  those two things are only equal for a single-chunk (`depth == 0`) entry.
- **Without `<hash>`:** runs the same per-manifest check for every
  manifest in the partition (or every partition, with `--all`), plus
  `PartitionStore::verify_all()` — every sealed pack's trailer
  (`pack_content_hash`) and every entry's own recorded hash, sealed and
  active.

**Two different failure-reporting shapes, by design:**
`PartitionStore::verify_all()` (the low-level pack/trailer integrity
check) is fail-fast — it stops and returns the first corruption it finds.
The per-manifest determinism check above it is **not** fail-fast — it
keeps going and reports every failing entry across every manifest checked
in one run, then prints a `PASS`/`FAIL` summary and every failure line.
`verify`'s process exit code is `1` if either check fails.

**Real output — pass** (immediately after the `pack` above):

```
$ bitchain --store /tmp/store verify
manifests: 1/1 passed | entries checked: 3 | pack entries checked: 13
PASS
```

**Real output — fail** (produced by flipping one byte inside a sealed
`.pack` file on disk, then re-running `verify`; caught by the pack-level
trailer check, not the entry-level one, since the flip landed in packfile
framing rather than surviving into a reconstructed entry):

```
$ bitchain --store /tmp/store verify
bitchain: corrupt pack: pack_content_hash mismatch in /tmp/store/.../packs/6b/6bf5a81b....pack: trailer says 6bf5a81be5293c47ea520a7ccc2d209037981f202325d8181e021dbcec837b92, computed 55edea36f5caaa5f7bcf8fc364acf3f19945ceea2771fc8d9d181908df41d8d2
```

Exit code `1`.

Verifying a partition that doesn't exist yet, or exists with zero
entries, is **not** an error — `PartitionStore::open` creates the
directory structure on first open, and an empty partition verifies
successfully with a `0/0 passed` report.

---

## `ls`

```
bitchain [--store <path>] ls [<partition>]
```

Summarizes one partition, or (with no `<partition>`) every partition
under the store root: sealed pack count, active (unsealed) entry count,
total distinct entries, manifest count, and the partition's root path.

**Real output:**

```
$ bitchain --store /tmp/store ls
f78761cf8c3621e851097dff9a4b0463  sealed_packs=1  active_entries=0  total_entries=13  manifests=1  root=/tmp/store/f78761cf8c3621e851097dff9a4b0463
```

---

## `gc`

```
bitchain [--store <path>] gc [<partition>] [--target-utilization <pct>]
```

Mark-and-sweep, scoped to exactly one partition at a time — **never
touches another partition's subtree** (verified by a dedicated test,
`gc_in_one_partition_never_touches_another`). With no `<partition>`,
every partition under the store root is considered independently.

**`--target-utilization` (default `80`):** before repacking a partition,
`gc` estimates its current utilization (`live_count / total_count * 100`,
via `bitchain::gc::utilization`, a read-only pass) and **skips the
repack entirely** if utilization is already at or above the target — a
repack that wouldn't reclaim much isn't worth the I/O. Pass `0` to force
a repack regardless of utilization.

**What an actual repack does, in order:** force-seal the active pack
(`store.seal_active()`, so the mark phase and the repack step both see a
stable, fully-indexed set) → mark the live set by walking every manifest
in `manifests/` and every fragment tree it roots → repack every live hash
into a staging area → atomically swap the staged `packs/`/`active/`
directories in for the real ones.

**v1: full repack, no incremental/generational GC.** Not implemented
today; a plausible v1.1 candidate.

**Real output** (nothing to reclaim, but demonstrating the
utilization-gated skip with `--target-utilization 0` to force a run
anyway):

```
$ bitchain --store /tmp/store gc --target-utilization 0
f78761cf8c3621e851097dff9a4b0463: utilization 100.0% (13/13) >= target 0.0% — skipped
```

---

## `push`

```
bitchain push <local_store> <remote> [--partition <string>] [--pack <hex-id>]...
```

Copies **sealed** `.pack`/`.idx` pairs, plus every manifest, for the named
partition(s) from `<local_store>` to `<remote>` — any filesystem path the
process can write to: a local directory, or a mounted network share /
`rclone`/`s3fs` mount. With no `--partition`, every partition present
under `<local_store>` is pushed. `--pack` (repeatable) restricts the pack
copy to specific pack ids; manifests always copy in full regardless.

**Scope limit, from the code's own doc comment (`src/cli/push.rs`):**
this reference CLI wires the local-filesystem backend end to end;
networked HTTP/S3 remotes are `storage-kit`'s job
(`S3Backend`/`HttpBackend`, implementing the same `ImmutableStore`
trait this crate already exposes) — a programmatic consumer reaches for
that crate directly, not this CLI, for those transports.
**`<local_store>`/`<remote>` are not S3 URIs or HTTP URLs in the current
CLI** — they are local (or locally-mounted) filesystem paths only.
Swapping the transport later is additive to `PushArgs`, not breaking.

**Only sealed packs move — but `pack` seals unconditionally now**, so in
practice a pack you just made is immediately pushable with no separate
`gc` step (a change from an earlier draft of this CLI, where sealing was
size-cap-only and a `push` right after `pack` could move zero packs).

**Real output:**

```
$ bitchain push /tmp/store /tmp/remote
f78761cf8c3621e851097dff9a4b0463: pushed 1 pack(s), 1 manifest(s) to /tmp/remote
```

---

## `pull`

```
bitchain pull <remote> <local_store> [--partition <string>] [--pack <hex-id>]...
```

The inverse of `push` — copies sealed pack/idx pairs and manifests from
`<remote>` into `<local_store>`. Same local-filesystem-only scope limit
as `push`; same "every partition present" default when `--partition` is
omitted.

**Real output, followed by a real cross-store restore proving the pulled
data is actually usable (not just copied bytes):**

```
$ bitchain pull /tmp/remote /tmp/fresh-store
f78761cf8c3621e851097dff9a4b0463: pulled 1 pack(s), 1 manifest(s) from /tmp/remote

$ bitchain --store /tmp/fresh-store unpack be98f1fdc2d802f199a5ab78f297ec1b510d6b92f926bbdda766e754dfd637b6 ./restored-from-pull
restored ./restored-from-pull/a.txt (17 bytes)
restored ./restored-from-pull/b-dup.bin (200000 bytes)
restored ./restored-from-pull/b.bin (200000 bytes)
3 file(s) restored
```

`diff -r` against the original source directory: byte-identical.

---

## Non-goals confirmed by reading the code, not assumed

- No `seal` command — sealing happens automatically at the end of every
  `pack`, or (for older content sitting in an active pack from prior CLI
  behavior) via `gc`. No standalone seal-only command exists.
- No `diff` command (flagged in `AGENT.md` as a plausible v1.1 addition,
  not required for the v1 MVP-completeness bar).
- No remote server state, subscription/licensing/quota, or account/auth
  management anywhere in this CLI — by design (`AGENT.md`: "CLI does NOT
  manage... Refraction's job").
- No `--legacy` flag for reading v1 (SHA-256, fixed-1MB) manifests — see
  `docs/manifest-format.md`'s "Versioning" section.
- No real S3/HTTP transport for `push`/`pull` in this crate today — see
  the scope note under `push` above.
