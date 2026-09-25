//! `bitchain init` -- explicitly provision a partition. storage-kit v3
//! split `PartitionStore::create` from `open` (`PartitionStore::open` no
//! longer provisions on first use, spec §3.7); this is the only bitchain
//! command that ever calls `create`, and the only place a partition
//! directory is ever created.
//!
//! DJ's ruling (`context/hot-decisions.md`, "One account, everything --
//! sixteen rulings"): an explicit `init` subcommand, not auto-provisioning
//! with defaults. The create/open split exists precisely so retention
//! class and encryption mode are decided deliberately, once, and never
//! changed -- auto-provisioning would have this CLI pick a retention class
//! on the user's behalf, which is exactly what `PartitionSpec` refuses to
//! let a constructor do. Every choice below is therefore **forced, not
//! defaulted**: `--subject-kind`, `--subject-id`, `--retention`, and
//! `--encryption` are all required arguments with no default value.
//!
//! The partition id is never accepted as a free-form string here (contrast
//! `super::resolve_partition`, which every other command uses for an
//! already-provisioned partition). It is always *derived* from
//! `(subject_kind, subject_id)` via `PartitionId::for_person`/
//! `for_household`/`for_service`, so `PartitionStore::create`'s derivation
//! check (`storage_kit::partition::verify_derivation`) is always
//! satisfiable from what this command has on hand -- unlike
//! `resolve_partition`, which mints a `PartitionId` from an arbitrary
//! string with no `Subject` behind it at all.

use bitchain::{
    EncryptionMode, PartitionId, PartitionSpec, PartitionStore, RetentionClass, Subject,
    SubjectKind,
};
use clap::ValueEnum;
use std::path::PathBuf;

/// Mirrors `storage_kit::SubjectKind` for `clap` parsing. Closed, forever,
/// at exactly three values -- see `storage_kit::partition`'s module docs
/// for the permanently-banned fourth-value list (`business`, `org`,
/// `team`, `project`, `client`, `division`: the segment field in a storage
/// costume).
#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum SubjectKindArg {
    Person,
    Household,
    Service,
}

impl From<SubjectKindArg> for SubjectKind {
    fn from(kind: SubjectKindArg) -> Self {
        match kind {
            SubjectKindArg::Person => SubjectKind::Person,
            SubjectKindArg::Household => SubjectKind::Household,
            SubjectKindArg::Service => SubjectKind::Service,
        }
    }
}

/// Mirrors `storage_kit::RetentionClass` for `clap` parsing. No `Default`
/// impl on purpose -- unlike `storage_kit::RetentionClass` itself (which
/// carries one so *callers that choose `standard`* have a canonical
/// spelling), this arg type must never let `init` run without the caller
/// naming a class.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum RetentionClassArg {
    Ephemeral,
    Standard,
    Durable,
}

impl From<RetentionClassArg> for RetentionClass {
    fn from(class: RetentionClassArg) -> Self {
        match class {
            RetentionClassArg::Ephemeral => RetentionClass::Ephemeral,
            RetentionClassArg::Standard => RetentionClass::Standard,
            RetentionClassArg::Durable => RetentionClass::Durable,
        }
    }
}

/// Mirrors `storage_kit::EncryptionMode` for `clap` parsing. Set once, at
/// `init` time, and never changeable after -- there is no `bitchain
/// set-encryption` command, by design; see `storage_kit::partition::EncryptionMode`.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum EncryptionModeArg {
    Plaintext,
    SealedRequired,
}

impl From<EncryptionModeArg> for EncryptionMode {
    fn from(mode: EncryptionModeArg) -> Self {
        match mode {
            EncryptionModeArg::Plaintext => EncryptionMode::Plaintext,
            EncryptionModeArg::SealedRequired => EncryptionMode::SealedRequired,
        }
    }
}

pub struct InitArgs {
    pub store_root: PathBuf,
    pub subject_kind: SubjectKindArg,
    pub subject_id: String,
    pub retention: RetentionClassArg,
    pub encryption: EncryptionModeArg,
    /// Free-text, stored only in the store-level, never-replicated local
    /// bindings file (`partitions.local.json`) -- never parsed for
    /// semantics.
    pub label: Option<String>,
}

/// Provision a new partition and return its hex id (the value every other
/// command's `--partition` flag accepts as a raw 32-hex-char id). Fails
/// with `PartitionAlreadyProvisioned` if this exact `(subject_kind,
/// subject_id)` pair was already `init`ed.
pub fn run(args: InitArgs) -> bitchain::Result<String> {
    let subject_kind: SubjectKind = args.subject_kind.into();
    let retention_class: RetentionClass = args.retention.into();
    let encryption_mode: EncryptionMode = args.encryption.into();

    let partition_id = match subject_kind {
        SubjectKind::Person => PartitionId::for_person(&args.subject_id),
        SubjectKind::Household => PartitionId::for_household(&args.subject_id),
        SubjectKind::Service => PartitionId::for_service(&args.subject_id),
    };

    let spec = PartitionSpec {
        subject: Subject {
            kind: subject_kind,
            id: args.subject_id.clone(),
        },
        retention_class,
        encryption_mode,
        label: args.label.clone(),
    };

    PartitionStore::create(&args.store_root, partition_id, spec)?;

    eprintln!(
        "provisioned partition {} (subject: {subject_kind:?}:{}, retention: {retention_class:?}, encryption: {encryption_mode:?}{})",
        partition_id.to_hex(),
        args.subject_id,
        args.label
            .as_deref()
            .map(|l| format!(", label: {l}"))
            .unwrap_or_default(),
    );
    eprintln!(
        "pass --partition {} to other commands to use this partition",
        partition_id.to_hex()
    );

    Ok(partition_id.to_hex())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn args(store_root: PathBuf, kind: SubjectKindArg, subject_id: &str) -> InitArgs {
        InitArgs {
            store_root,
            subject_kind: kind,
            subject_id: subject_id.to_string(),
            retention: RetentionClassArg::Standard,
            encryption: EncryptionModeArg::Plaintext,
            label: None,
        }
    }

    #[test]
    fn init_provisions_a_partition_that_open_can_then_read() {
        let dir = tempdir().unwrap();
        let printed = run(args(
            dir.path().to_path_buf(),
            SubjectKindArg::Household,
            "acme",
        ))
        .unwrap();

        assert_eq!(printed, PartitionId::for_household("acme").to_hex());
        // `open` must now succeed -- reproducing that is the entire point
        // of `init` existing: before this command, no code path in this
        // crate ever called `PartitionStore::create`, so `open` always
        // failed with `PartitionNotProvisioned` on first use.
        PartitionStore::open(dir.path(), PartitionId::for_household("acme")).unwrap();
    }

    #[test]
    fn init_derives_a_distinct_partition_per_subject_kind_for_the_same_id() {
        let dir = tempdir().unwrap();
        let person = run(args(
            dir.path().to_path_buf(),
            SubjectKindArg::Person,
            "shared-id",
        ))
        .unwrap();
        let household = run(args(
            dir.path().to_path_buf(),
            SubjectKindArg::Household,
            "shared-id",
        ))
        .unwrap();
        let service = run(args(
            dir.path().to_path_buf(),
            SubjectKindArg::Service,
            "shared-id",
        ))
        .unwrap();

        assert_ne!(person, household);
        assert_ne!(household, service);
        assert_ne!(person, service);
    }

    #[test]
    fn init_twice_for_the_same_subject_fails_closed() {
        let dir = tempdir().unwrap();
        run(args(
            dir.path().to_path_buf(),
            SubjectKindArg::Service,
            "svc-1",
        ))
        .unwrap();

        let err = run(args(
            dir.path().to_path_buf(),
            SubjectKindArg::Service,
            "svc-1",
        ))
        .unwrap_err();
        assert!(matches!(
            err,
            bitchain::BitchainError::PartitionAlreadyProvisioned(_)
        ));
    }

    /// The bug this command fixes: `resolve_partition` (used by every other
    /// command) mints a `PartitionId` straight from an arbitrary string
    /// with no `Subject` behind it. `init`'s subject-derived id must never
    /// coincide with that bare, unprefixed derivation for the same raw
    /// string -- if it did, `init`'s derivation check would be pointless.
    #[test]
    fn init_id_never_matches_the_bare_resolve_partition_derivation() {
        let dir = tempdir().unwrap();
        let printed = run(args(
            dir.path().to_path_buf(),
            SubjectKindArg::Household,
            "demo",
        ))
        .unwrap();

        let bare = super::super::resolve_partition("demo").unwrap();
        assert_ne!(printed, bare.to_hex());
    }
}
