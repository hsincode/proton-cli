//! Shared error types for proton-cli.

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

    #[error("app `{0}` already exists")]
    AppExists(String),

    #[error("app `{0}` not found")]
    AppNotFound(String),

    #[error("exe not found: {0}")]
    ExeNotFound(PathBuf),

    #[error(
        "umu-run not found (optional when Steam is installed; see `proton-cli doctor`)"
    )]
    UmuNotFound,

    #[error(
        "no launch backend: install Steam (with SteamLinuxRuntime + Proton) or umu-launcher; see `proton-cli doctor`"
    )]
    NoLaunchBackend,

    #[error("no Proton runtime found; install Steam Proton / dwproton or run `proton-cli proton install`")]
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
