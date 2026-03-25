mod adapter;
mod backend;
mod builder;
mod i18n;
mod manifest;
#[cfg(any(not(windows), test))]
mod mszip;
mod progress;
mod state;
mod version;

rust_i18n::i18n!("locales", fallback = "en");

use std::path::PathBuf;
use std::process::ExitCode;
use std::{env, io::Write};

use clap::{Parser, Subcommand, ValueEnum};
use env_logger::{Builder, Target};
use i18n::Messages;
use log::{LevelFilter, error};

#[derive(Debug, Parser)]
#[command(
    author,
    version,
    about = "Static WinGet source builder for third-party repositories"
)]
struct Cli {
    #[arg(
        long,
        global = true,
        env = "WINGET_SOURCE_BUILDER_LANG",
        default_value = "en",
        value_name = "locale",
        help = "Locale tag for user-facing messages. Add more locales by adding files under locales/."
    )]
    lang: String,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Build(BuildArgs),
    QueueValidation(QueueValidationArgs),
    BuildIndex(BuildArgs),
}

#[derive(Debug, Clone, Parser)]
pub(crate) struct BuildArgs {
    #[arg(long)]
    pub(crate) repo: PathBuf,

    #[arg(long)]
    pub(crate) state: PathBuf,

    #[arg(long)]
    pub(crate) out: PathBuf,

    #[arg(long, default_value = "v2")]
    pub(crate) format: CatalogFormat,

    #[arg(long, default_value = "wingetutil")]
    pub(crate) backend: BackendKind,
}

#[derive(Debug, Clone, Parser)]
pub(crate) struct QueueValidationArgs {
    #[arg(long)]
    pub(crate) repo: PathBuf,

    #[arg(long)]
    pub(crate) state: PathBuf,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, ValueEnum)]
pub(crate) enum CatalogFormat {
    V1,
    V2,
}

impl CatalogFormat {
    pub(crate) const fn package_file_name(self) -> &'static str {
        match self {
            Self::V1 => "source.msix",
            Self::V2 => "source2.msix",
        }
    }

    pub(crate) const fn wingetutil_schema_version(self) -> (u32, u32) {
        match self {
            Self::V1 => (1, u32::MAX),
            Self::V2 => (2, 0),
        }
    }

    pub(crate) const fn uses_package_sidecars(self) -> bool {
        matches!(self, Self::V2)
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, ValueEnum)]
pub(crate) enum BackendKind {
    Wingetutil,
    Rust,
}

fn main() -> ExitCode {
    init_logging();
    let cli = Cli::parse();
    let messages = Messages::new(cli.lang);

    let result = match cli.command {
        Command::Build(args) => builder::run_build(args, messages.clone()),
        Command::QueueValidation(args) => builder::run_queue_validation(args, messages.clone()),
        Command::BuildIndex(args) => builder::run_build_index(args, messages.clone()),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            error!("{}", messages.build_failed(&error));
            ExitCode::FAILURE
        }
    }
}

fn init_logging() {
    let mut builder = Builder::new();
    if let Ok(filters) = env::var("WINGET_SOURCE_BUILDER_LOG").or_else(|_| env::var("RUST_LOG")) {
        builder.parse_filters(&filters);
    } else {
        builder.filter_level(LevelFilter::Info);
    }

    builder
        .target(Target::Stdout)
        .format(|buf, record| writeln!(buf, "[{}] {}", record.level(), record.args()))
        .init();
}
