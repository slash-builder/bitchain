# Integration sketch: bitchain in Quickring Courier

**CLUS-20 deliverable #4 — integration boundary only, no code.** This
describes where bitchain's already-verified v2 primitives (see
`docs/concepts.md`, `docs/manifest-format.md`, `docs/cli-reference.md`)
are expected to meet Courier, per `AGENT.md`'s "Related work" section
(`QR-16`) and the Kit-model dependency graph
(`Storage Kit ← Refraction, Score, Maestro, Courier`,
`dlockamy/context/cross-repo-context.md`). Nothing in this document is a
new design decision — it names the boundary a spec reader needs, and
flags every open question to its actual owner rather than resolving it
here.

## What's actually settled, and by whom

- **bitchain is a Hearth client of nothing — it's a storage primitive
  Courier consumes, not a Hearth participant itself.** Courier is a
  Hearth client first (locked, `cross-repo-context.md`); bitchain doesn't
  speak Hearth, it just provides the out-of-band block store Courier's
  Hearth messages reference.
- **Out-of-band file sharing shape (locked at the strategic-note level,
  `2026-05-architecture-review.md` and `AGENT.md`'s "Deployed by" line):**
  a bitchain manifest is small JSON — it rides in-band as a Hearth
  message; the file's actual blocks move out of band through bitchain's
  store. This keeps large payloads off the fabric entirely.
- **`bitchain-sys` is the named Rust binding** Courier is expected to
  embed (`AGENT.md`: "the Rust binding (bitchain-sys) is gated on
  storage-kit's trait shape stabilizing, not on bitchain CLI
  completeness"). It does not exist yet — no crate, no code, no API
  surface has been written. Its shape is `software-developer` /
  `data-architect`'s call once `storage-kit`'s trait layer (the
  `ImmutableStore`/`ReadBlock`/`WriteBlock` contract this CLI's
  `PartitionStore` already implements) stabilizes there — **not this
  document's to design.**

## The boundary this document can state precisely

### Partition = household

`PartitionId` is already, mechanically, "a 16-byte physical storage
boundary derived from a namespace string" (`docs/concepts.md`). Mapping
Courier's household concept onto it is the obvious fit: one household,
one partition, derived from whatever stable household identifier
Identity Kit already assigns (family-graph id, most likely — Identity
Kit's model is the source of truth for what that identifier actually is,
not bitchain's). This document does not invent a derivation string —
that's a decision for whoever owns the household-identifier format
(Identity Kit / data-architect), made once, and then just handed to
`PartitionId::derive` as-is.

### Context = file identity

`Context` is already, mechanically, "a 16-byte logical identity, derived
once at ingest, immutable thereafter" (`docs/concepts.md`). For Courier's
use case, this is naturally "which shared file, shared by whom, in which
conversation" — the CLI's own `--identity` flag on `pack` demonstrates
the intended shape (`Context::derive("<identity>:<relative-path>")`)
without committing to what Courier's actual identity string looks like.
That string format is a Fabric Kit / data-architect decision (it likely
needs to reference a Hearth-level conversation or grant identifier, which
this document has no authority over), not something to lock here.

### What Courier would call, in principle

Not an API design — a statement of which already-verified operations
exist to be wrapped, and where they live:

- `pack` (fragment + write manifest) — for sending a file.
- `unpack` (reconstruct from manifest) — for receiving a file.
- `verify` — for confirming a received file's integrity before
  presenting it to the user.
- `gc` — for reclaiming space after a household deletes shared content
  locally, subject to whatever retention policy Courier's product design
  actually wants (not addressed here — that's a UX/product decision, not
  a storage one).

`bitchain-sys` would expose these as a library API against the
`ImmutableStore` trait rather than shelling out to the `bitchain` binary
— consistent with `AGENT.md`'s "Reusable library" invariant ("the CLI is
a thin reference client over a stable lib API"). The CLI documented in
`docs/cli-reference.md` is the reference implementation and conformance
target for that library API, not a dependency Courier would actually
process-spawn.

### Transport is explicitly not this document's concern

`push`/`pull` in the reference CLI are local-filesystem-to-local-
filesystem only (`docs/cli-reference.md`). Courier's actual block
transport — how a sender's device gets bytes to a receiver's device
across the open internet, NAT, intermittent connectivity, and so on — is
the **block-availability / transport contract** named in
`2026-05-architecture-review.md` as its own open item (QR-17, blocked on
CLUS-20). This document does not attempt to resolve QR-17; it only
confirms that bitchain's own `push`/`pull` are not that contract as
shipped, and that a networked backend (S3/HTTP) is `storage-kit`'s job
per `AGENT.md`, consumed directly rather than through this CLI.

## Open questions this document routes rather than answers

- **`bitchain-sys`'s actual API surface** — software-developer /
  data-architect, gated on `storage-kit`'s trait layer stabilizing there
  (it currently has not been touched for v2 at all — see the
  cross-org-dependency finding in `projects/clusterzer0.md`).
- **The household-identifier string `PartitionId::derive` should consume**
  — Identity Kit / data-architect.
- **The context-identity string format for shared files** — Fabric Kit /
  data-architect.
- **QR-17's block-availability / transport contract** — Messaging
  Architect (wire/transport seam) and data-architect (storage-side
  contract), not resolved here.
- **Retention/GC policy for household-deleted shared content** —
  product/UX decision, not a storage-primitive one.

None of these are blocking this spec's other three deliverables — they
block the next layer up, `bitchain-sys` itself, which is correctly not
yet started per `AGENT.md`.
