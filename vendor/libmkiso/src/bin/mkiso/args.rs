use clap::{Args, Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(
    name = "mkiso",
    version = option_env!("MKISO_BUILD_TAG").unwrap_or(env!("CARGO_PKG_VERSION")),
    about = "Plan, create and inspect optical media"
)]
pub struct Cli {
    #[arg(long, global = true)]
    pub json: bool,
    #[arg(long, global = true)]
    pub report: Option<PathBuf>,
    #[arg(long, global = true, value_enum, default_value = "auto")]
    pub progress: ProgressMode,
    #[arg(long, global = true)]
    pub replace: bool,
    #[arg(long, global = true)]
    pub reproducible: bool,
    #[arg(long, global = true)]
    pub timestamp: Option<String>,
    #[arg(long, global = true, default_value = "1", value_parser = clap::value_parser!(u16).range(1..))]
    pub jobs: u16,
    #[command(subcommand)]
    pub command: Command,
}
#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ProgressMode {
    Auto,
    Always,
    Never,
}
#[derive(Debug, Subcommand)]
pub enum Command {
    Create(Create),
    Build {
        manifest: PathBuf,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    Plan {
        manifest: PathBuf,
    },
    Inspect {
        image: PathBuf,
    },
    Verify {
        image: PathBuf,
        #[arg(long)]
        manifest: Option<PathBuf>,
    },
    Extract {
        image: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    Repack {
        image: PathBuf,
        #[arg(long)]
        overlay: PathBuf,
        #[arg(long)]
        output: PathBuf,
        #[arg(long)]
        boot: Option<String>,
    },
    Boot {
        #[command(subcommand)]
        command: BootCommand,
    },
    Multiboot {
        #[command(subcommand)]
        command: MultibootCommand,
    },
    Usb {
        #[command(subcommand)]
        command: UsbCommand,
    },
    Disk {
        #[command(subcommand)]
        command: DiskCommand,
    },
}
#[derive(Debug, Args)]
pub struct Create {
    pub root: PathBuf,
    #[arg(long)]
    pub output: PathBuf,
    #[arg(long, default_value = "MKISO")]
    pub label: String,
    #[arg(long, default_value = "iso9660")]
    pub filesystem: String,
    #[arg(long)]
    pub joliet: bool,
    #[arg(long)]
    pub rock_ridge: bool,
    #[arg(long)]
    pub profile: Option<String>,
    #[arg(long)]
    pub arch: Option<String>,
    #[arg(long)]
    pub boot: Vec<String>,
    #[arg(long)]
    pub boot_assets: Option<PathBuf>,
    #[arg(long)]
    pub media: Option<String>,
    #[arg(long)]
    pub partition_table: Option<String>,
}
#[derive(Debug, Subcommand)]
pub enum BootCommand {
    Prepare {
        #[arg(long)]
        loader: String,
        #[arg(long)]
        arch: String,
        #[arg(long)]
        kernel: PathBuf,
        #[arg(long)]
        initrd: Vec<PathBuf>,
        #[arg(long)]
        cmdline: Option<String>,
        #[arg(long)]
        assets: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
}
#[derive(Debug, Subcommand)]
pub enum MultibootCommand {
    Init {
        manifest: PathBuf,
    },
    Add {
        manifest: PathBuf,
        image: PathBuf,
        #[arg(long)]
        id: String,
        #[arg(long)]
        title: String,
    },
    Plan {
        manifest: PathBuf,
        #[arg(long, default_value = "usb")]
        target: String,
    },
    Build {
        manifest: PathBuf,
        #[arg(long)]
        target: String,
        #[arg(long)]
        output: PathBuf,
        #[arg(long)]
        size: Option<String>,
        #[arg(long)]
        partition_table: Option<String>,
        #[arg(long)]
        data_filesystem: Option<String>,
        #[arg(long)]
        backend: Option<String>,
    },
    Verify {
        image: PathBuf,
        #[arg(long)]
        manifest: Option<PathBuf>,
    },
    Test {
        image: PathBuf,
        #[arg(long)]
        firmware: String,
        #[arg(long)]
        evidence_dir: PathBuf,
        #[arg(long, default_value = "menu")]
        entry: String,
        #[arg(long, default_value = "ventoy")]
        backend: String,
        #[arg(long)]
        bios: Option<PathBuf>,
        #[arg(long)]
        ovmf_code: Option<PathBuf>,
        #[arg(long)]
        ovmf_vars: Option<PathBuf>,
        #[arg(long)]
        swtpm: Option<PathBuf>,
        #[arg(long)]
        key: Vec<String>,
        #[arg(long, default_value = "90", value_parser = clap::value_parser!(u64).range(1..=3600))]
        timeout: u64,
        #[arg(long, default_value = "15")]
        select_after: u64,
        #[arg(long, default_value = "4096", value_parser = clap::value_parser!(u32).range(128..=65536))]
        memory: u32,
        #[arg(long, default_value = "kvm", value_parser = ["kvm", "tcg"])]
        accel: String,
    },
}
#[derive(Debug, Subcommand)]
pub enum UsbCommand {
    Build {
        root: PathBuf,
        #[arg(long)]
        profile: String,
        #[arg(long)]
        output: PathBuf,
        #[arg(long)]
        filesystem: Option<String>,
        #[arg(long)]
        split_wim_size: Option<String>,
    },
}
#[derive(Debug, Subcommand)]
pub enum DiskCommand {
    Inspect {
        device: PathBuf,
    },
    Write {
        image: PathBuf,
        #[arg(long)]
        device: PathBuf,
        #[arg(long)]
        erase: bool,
        #[arg(long)]
        expect_device_id: Option<String>,
        #[arg(long)]
        verify: Option<String>,
    },
}
