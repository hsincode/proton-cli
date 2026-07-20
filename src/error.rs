//! Shared error types for protoncli.

use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("invalid app id `{0}`: use lowercase letters, digits, and hyphens only (e.g. my-game)")]
    InvalidAppId(String),

    #[error("unknown locale `{0}` (known ids: ja, en)")]
    UnknownLocale(String),

    #[error("unknown graphics `{0}` (known: dxvk, wined3d)")]
    UnknownGraphics(String),

    #[error("unknown log level `{0}` (known: quiet, normal, verbose)")]
    UnknownLogLevel(String),

    #[error(
        "gamescope not found (install gamescope, or set [launch.gamescope].command; see `protoncli doctor`)"
    )]
    GamescopeNotFound,

    #[error("app `{0}` already exists")]
    AppExists(String),

    #[error("app `{0}` not found")]
    AppNotFound(String),

    #[error("refusing to delete shared default prefix (id=`default`)")]
    PrefixIsDefault,

    #[error(
        "prefix `{id}` is used by apps: {apps} (pass --force to delete anyway)"
    )]
    PrefixInUse { id: String, apps: String },

    #[error("refusing to delete absolute prefix path `{0}` (only managed relative ids)")]
    PrefixAbsolutePath(String),

    #[error("exe not found: {0}")]
    ExeNotFound(PathBuf),

    #[error(
        "umu-run not found (optional when Steam is installed; see `protoncli doctor`)"
    )]
    UmuNotFound,

    #[error(
        "no launch backend: install Steam (with SteamLinuxRuntime + Proton) or umu-launcher; see `protoncli doctor`"
    )]
    NoLaunchBackend,

    #[error("no Proton runtime found; install Steam Proton / dwproton or run `protoncli proton install`")]
    ProtonNotFound,

    #[error("Proton path does not look like a valid Proton install: {0}")]
    InvalidProtonPath(PathBuf),

    #[error("download failed: {0}")]
    Download(String),

    #[error("checksum mismatch for {path}: expected {expected}, got {actual}")]
    ChecksumMismatch {
        path: PathBuf,
        expected: String,
        actual: String,
    },

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    TomlDe(#[from] toml::de::Error),

    #[error(transparent)]
    TomlSer(#[from] toml::ser::Error),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
