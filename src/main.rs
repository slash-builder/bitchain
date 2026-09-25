mod cli;

use bitchain::fragment::ChunkingProfile;
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::process::ExitCode;

const DEFAULT_PARTITION: &str = "default";
const DEFAULT_TARGET_UTILIZATION: f64 = 80.0;

#[derive(Parser, Debug)]
#[command(
    name = "bitchain",
    about = "Content-addressed binary storage — format v2 (BLAKE3, packfiles, partitions).",
    long_about = None
)]
struct Cli {
    /// Store root for pack/unpack/verify/ls/gc. Defaults to
    /// ~/.bitchain/store. (push/pull take their store roots as arguments.)
    #[arg(long, global = true)]
    store: Option<PathBuf>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Provision a new partition and print its hex id. The only place a
    /// partition is ever created -- every other command's `--partition`
    /// operates on a partition `init` already provisioned. Every choice is
    /// forced, not defaulted: retention class and encryption mode are set
    /// once, here, and never change (there is no `set-retention` or
    /// `set-encryption` command). Run this before `pack`.
    Init {
        /// Who this partition belongs to: a person, a household, or a
        /// headless service principal. Closed, forever, at exactly these
        /// three values -- a `business`/`org`/`team`/`project`/`client`/
        /// `division` value is a segment field wearing a storage-layer
        /// costume and is permanently out of scope for this flag.
        #[arg(long)]
        subject_kind: cli::init::SubjectKindArg,

        /// The id hashed (with the subject kind as a domain-separator
        /// prefix) to derive this partition's id -- an account id, a
        /// household id, or a service principal id, depending on
        /// `--subject-kind`.
        #[arg(long)]
        subject_id: String,

        /// GC behavior for this partition. A class is real only if it
        /// changes GC behaviour: `ephemeral` lets GC drop unreferenced
        /// content aggressively; `standard` drops it only when GC is asked
        /// to run; `durable` never drops referenced-or-not content (only
        /// destroying the whole partition destroys its data). No default --
        /// retention is decided once, at provisioning time, not retrofitted.
        #[arg(long)]
        retention: cli::init::RetentionClassArg,

        /// Set once, here, and never changeable afterward -- there is no
        /// setter. `plaintext` is the only value any command in this CLI
        /// can currently write to; `sealed-required` is reserved for when
        /// the key hierarchy lands and makes every write into this
        /// partition fail closed until it does.
        #[arg(long)]
        encryption: cli::init::EncryptionModeArg,

        /// Free-text, recorded only in the store-level, never-replicated
        /// local bindings file. Never parsed for semantics.
        #[arg(long)]
        label: Option<String>,
    },

    /// Ingest a file or directory, chunk + hash + compress it into the
    /// store, and print the manifest's root hash.
    Pack {
        /// File or directory to pack.
        path: PathBuf,

        /// Partition: either a 32-hex-char partition id, or a
        /// household/namespace string to derive one from.
        #[arg(long, default_value = DEFAULT_PARTITION)]
        partition: String,

        /// Logical identity for the whole pack, recorded in the manifest
        /// entry's `context` field. Defaults to `Context::ANONYMOUS`
        /// (identity is content). This labels the manifest reference only --
        /// it does NOT affect storage, addressing, or dedup. Two `pack`
        /// invocations of identical bytes under different `--identity`
        /// values still share one physical stored entry; that is correct
        /// and intentional, per the 2026-06 storage lock's dedup clause
        /// ("same bytes, different contexts = same payload deduplicated
        /// under one hash but distinct identities").
        #[arg(long)]
        identity: Option<String>,

        /// Use fixed-size chunking instead of FastCDC, with this block size.
        #[arg(long)]
        fixed_block_size: Option<u64>,

        /// FastCDC parameters (ignored if --fixed-block-size is set).
        #[arg(long, default_value_t = 4096)]
        cdc_min: u32,
        #[arg(long, default_value_t = 16384)]
        cdc_avg: u32,
        #[arg(long, default_value_t = 65536)]
        cdc_max: u32,
    },

    /// Retrieve and reconstruct the file(s) recorded under a manifest hash
    /// (as printed by `pack`) into `output_path`.
    Unpack {
        /// Manifest content hash printed by `pack` (or a path to a
        /// manifest JSON file).
        hash: String,

        /// Directory to reconstruct files into.
        output_path: PathBuf,

        /// Partition `hash` was packed into. Defaults to the same
        /// "default" partition `pack` uses when `--partition` is omitted.
        #[arg(long)]
        partition: Option<String>,
    },

    /// Re-hash content and confirm determinism. With a hash, verifies just
    /// that manifest; without one, verifies every manifest + pack in scope.
    Verify {
        /// Manifest hash to verify. Omit to verify everything in scope.
        hash: Option<String>,

        #[arg(long, default_value = DEFAULT_PARTITION)]
        partition: String,

        /// Verify every partition under the store root, not just one.
        #[arg(long)]
        all: bool,
    },

    /// List what's stored locally. Omit `partition` to summarize every
    /// partition under the store root.
    Ls { partition: Option<String> },

    /// Garbage-collect unreferenced content (mark-and-sweep). Omit
    /// `partition` to consider every partition under the store root.
    Gc {
        partition: Option<String>,

        /// Skip repacking a partition already at or above this
        /// live/total percentage.
        #[arg(long, default_value_t = DEFAULT_TARGET_UTILIZATION)]
        target_utilization: f64,
    },

    /// Replicate sealed pack(s) + manifests from `local_store` to `remote`.
    /// See `src/cli/push.rs` for the local-vs-S3 scope note.
    Push {
        local_store: PathBuf,
        remote: PathBuf,

        /// Partition to push. Default: every partition in `local_store`.
        #[arg(long)]
        partition: Option<String>,

        /// Specific pack ids (hex) to push. Default: all sealed packs.
        #[arg(long)]
        pack: Vec<String>,
    },

    /// Fetch sealed pack(s) + manifests from `remote` into `local_store`.
    Pull {
        remote: PathBuf,
        local_store: PathBuf,

        /// Partition to pull. Default: every partition `remote` has.
        #[arg(long)]
        partition: Option<String>,

        /// Specific pack ids (hex) to pull. Default: all sealed packs.
        #[arg(long)]
        pack: Vec<String>,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let store_root = cli.store.unwrap_or_else(cli::default_store_root);

    let result = run(cli.command, store_root);

    match result {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE, // ran fine, but e.g. verify found a mismatch
        Err(err) => {
            eprintln!("bitchain: {err}");
            // Every store-opening command (pack/unpack/verify/ls/gc) surfaces
            // `StorageError::PartitionNotProvisioned` unchanged from
            // storage-kit -- it names the partition id but not what to do
            // about it. Name `init` here, once, rather than teaching every
            // command's own error path about it separately.
            if let bitchain::BitchainError::PartitionNotProvisioned(_) = err {
                eprintln!(
                    "  no partition.json exists for this id -- provision it first with \
                     `bitchain init --subject-kind <person|household|service> --subject-id <id> \
                     --retention <ephemeral|standard|durable> --encryption <plaintext|sealed-required>`, \
                     then pass the hex id it prints as --partition to this command"
                );
            }
            ExitCode::FAILURE
        }
    }
}

/// Returns `Ok(false)` (not an `Err`) for a clean run that nonetheless
/// reports failure, e.g. `verify` finding a hash mismatch.
fn run(command: Commands, store_root: PathBuf) -> bitchain::Result<bool> {
    match command {
        Commands::Init {
            subject_kind,
            subject_id,
            retention,
            encryption,
            label,
        } => {
            let partition_id = cli::init::run(cli::init::InitArgs {
                store_root,
                subject_kind,
                subject_id,
                retention,
                encryption,
                label,
            })?;
            println!("{partition_id}");
            Ok(true)
        }

        Commands::Pack {
            path,
            partition,
            identity,
            fixed_block_size,
            cdc_min,
            cdc_avg,
            cdc_max,
        } => {
            let profile = match fixed_block_size {
                Some(block_size) => ChunkingProfile::Fixed { block_size },
                None => ChunkingProfile::FastCdc {
                    min_size: cdc_min,
                    avg_size: cdc_avg,
                    max_size: cdc_max,
                },
            };
            let hash = cli::pack::run(cli::pack::PackArgs {
                input: path,
                store_root,
                partition,
                identity,
                profile,
            })?;
            println!("{hash}");
            Ok(true)
        }

        Commands::Unpack {
            hash,
            output_path,
            partition,
        } => {
            let restored = cli::unpack::run(cli::unpack::UnpackArgs {
                manifest: hash,
                store_root,
                partition,
                output_dir: output_path,
            })?;
            eprintln!("{restored} file(s) restored");
            Ok(true)
        }

        Commands::Verify {
            hash,
            partition,
            all,
        } => {
            let report = cli::verify::run(cli::verify::VerifyArgs {
                store_root,
                hash,
                partition,
                all_partitions: all,
            })?;
            println!(
                "manifests: {}/{} passed | entries checked: {} | pack entries checked: {}",
                report.manifests_passed,
                report.manifests_checked,
                report.entries_checked,
                report.pack_entries_checked
            );
            for failure in &report.failures {
                println!("FAIL: {failure}");
            }
            if report.passed() {
                println!("PASS");
            } else {
                println!("FAIL");
            }
            Ok(report.passed())
        }

        Commands::Ls { partition } => {
            cli::ls::run(cli::ls::LsArgs {
                store_root,
                partition,
            })?;
            Ok(true)
        }

        Commands::Gc {
            partition,
            target_utilization,
        } => {
            cli::gc::run(cli::gc::GcArgs {
                store_root,
                partition,
                target_utilization,
            })?;
            Ok(true)
        }

        Commands::Push {
            local_store,
            remote,
            partition,
            pack,
        } => {
            cli::push::run(cli::push::PushArgs {
                store_root: local_store,
                partition,
                to: remote,
                pack_ids: pack,
            })?;
            Ok(true)
        }

        Commands::Pull {
            remote,
            local_store,
            partition,
            pack,
        } => {
            cli::pull::run(cli::pull::PullArgs {
                store_root: local_store,
                partition,
                from: remote,
                pack_ids: pack,
            })?;
            Ok(true)
        }
    }
}
