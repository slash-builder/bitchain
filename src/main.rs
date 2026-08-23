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
    /// Ingest a file or directory, chunk + hash + compress it into the
    /// store, and print the manifest's root hash.
    Pack {
        /// File or directory to pack.
        path: PathBuf,

        /// Partition: either a 32-hex-char partition id, or a
        /// household/namespace string to derive one from.
        #[arg(long, default_value = DEFAULT_PARTITION)]
        partition: String,

        /// Logical identity for the whole pack. Defaults to
        /// `Context::ANONYMOUS` (identity is content).
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
            ExitCode::FAILURE
        }
    }
}

/// Returns `Ok(false)` (not an `Err`) for a clean run that nonetheless
/// reports failure, e.g. `verify` finding a hash mismatch.
fn run(command: Commands, store_root: PathBuf) -> bitchain::Result<bool> {
    match command {
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
