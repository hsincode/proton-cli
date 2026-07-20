//! clap command definitions.

use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "protoncli",
    version,
    about = "Run Windows games/apps via umu + Proton (dwproton by default)",
    long_about = "Thin CLI orchestrator around umu-run and Proton.\n\
                  Run with no subcommand for a minimal setup TUI.\n\
                  Does not bundle Proton; install a runtime with `protoncli proton install`.\n\
                  Requires umu-launcher on PATH for Steam-equivalent launches.\n\
                  Wine prefixes live under ~/.protoncli (default: ~/.protoncli/default)."
)]
pub struct Cli {
    /// Subcommand. When omitted, launches the setup TUI.
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Register an application and optionally create a .desktop shortcut
    Add {
        /// Display name
        #[arg(long)]
        name: String,
        /// Path to the Windows .exe
        #[arg(long)]
        exe: PathBuf,
        /// App id (slug). Default: derived from --name
        #[arg(long)]
        id: Option<String>,
        /// Arguments passed to the exe
        #[arg(long = "arg", value_name = "ARG")]
        args: Vec<String>,
        /// Working directory (default: parent of exe)
        #[arg(long)]
        cwd: Option<PathBuf>,
        /// Wineprefix id or absolute path.
        /// Relative ids: `default` → ~/.protoncli/default, others → ~/.protoncli/prefixes/<id>.
        /// Default: config.default_prefix (usually `default`).
        #[arg(long)]
        prefix: Option<String>,
        /// Proton logical name or path (default: dwproton)
        #[arg(long, default_value = "dwproton")]
        proton: String,
        /// umu GAMEID for protonfixes
        #[arg(long)]
        game_id: Option<String>,
        /// umu STORE (egs, gog, none, …)
        #[arg(long)]
        store: Option<String>,
        /// Do not create a .desktop shortcut
        #[arg(long)]
        no_desktop: bool,
        /// Per-app launch locale id (`ja`, `en`). Omit to inherit global `config.locale`.
        #[arg(long)]
        locale: Option<String>,
        /// Per-app D3D backend (`dxvk`, `wined3d`). Omit to inherit `config.launch.graphics`.
        #[arg(long)]
        graphics: Option<String>,
    },

    /// Launch a registered app (used by .desktop shortcuts)
    Launch {
        /// App id
        id: String,
    },

    /// Run an exe once without registering
    Run {
        /// Path to the Windows .exe
        exe: PathBuf,
        /// Extra args after `--`
        #[arg(last = true)]
        args: Vec<String>,
        /// Wineprefix id or absolute path (default: config.default_prefix)
        #[arg(long)]
        prefix: Option<String>,
        /// Proton logical name or path
        #[arg(long, default_value = "dwproton")]
        proton: String,
        /// umu GAMEID
        #[arg(long, default_value = "umu-default")]
        game_id: String,
        /// Working directory
        #[arg(long)]
        cwd: Option<PathBuf>,
        /// One-shot locale id (`ja`, `en`). Default: global `config.locale`.
        #[arg(long)]
        locale: Option<String>,
        /// One-shot D3D backend (`dxvk`, `wined3d`). Default: `config.launch.graphics`.
        #[arg(long)]
        graphics: Option<String>,
        /// Terminal noise: `quiet` | `normal` | `verbose`. Default: `config.launch.log_level`.
        #[arg(long, value_name = "LEVEL")]
        log_level: Option<String>,
        /// Keep host `VK_ICD_FILENAMES` / `VK_DRIVER_FILES` (default: strip like Steam/Lutris).
        #[arg(long)]
        no_sanitize_vulkan: bool,
    },

    /// List registered apps
    List,

    /// Remove a registered app
    Remove {
        id: String,
        /// Keep the wine prefix directory
        #[arg(long)]
        keep_prefix: bool,
        /// Keep the .desktop shortcut
        #[arg(long)]
        keep_desktop: bool,
    },

    /// Manage application menu shortcuts
    Desktop {
        #[command(subcommand)]
        command: DesktopCmd,
    },

    /// Manage Proton runtimes (dwproton)
    Proton {
        #[command(subcommand)]
        command: ProtonCmd,
    },

    /// Manage wine prefixes under ~/.protoncli
    Prefix {
        #[command(subcommand)]
        command: PrefixCmd,
    },

    /// Check umu, Proton, PATH, and layout
    Doctor,

    /// Show paths for an app (prefix, exe, desktop)
    Path {
        id: String,
    },

    /// Open the setup TUI (same as running with no args)
    Tui,
}

#[derive(Debug, Subcommand)]
pub enum DesktopCmd {
    /// Create/refresh the .desktop file for an app
    Install { id: String },
    /// Remove the .desktop file for an app
    Uninstall { id: String },
}

#[derive(Debug, Subcommand)]
pub enum ProtonCmd {
    /// Download and install dwproton into the managed runtimes dir
    Install {
        /// Release tag (default: latest)
        #[arg(long)]
        version: Option<String>,
        /// Reinstall even if present
        #[arg(long)]
        force: bool,
    },
    /// List discovered Proton installs
    List,
}

#[derive(Debug, Subcommand)]
pub enum PrefixCmd {
    /// List known prefixes under ~/.protoncli
    List,
    /// Create an empty prefix directory
    Create {
        /// Prefix id (`default` or a slug → ~/.protoncli/prefixes/<id>)
        id: String,
    },
    /// Print resolved path for a prefix id
    Path {
        id: String,
    },
}
