//! dwproton download, install, and PROTONPATH resolution.

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use indicatif::{ProgressBar, ProgressStyle};
use serde::Deserialize;
use sha2::{Digest, Sha512};

use crate::config::ProtonConfig;
use crate::error::{Error, Result};
use crate::paths::Paths;

const DWPROTON_API_LATEST: &str =
    "https://dawn.wine/api/v1/repos/dawn-winery/dwproton/releases/latest";
const DWPROTON_API_TAG: &str = "https://dawn.wine/api/v1/repos/dawn-winery/dwproton/releases/tags";
const ARCH: &str = "x86_64";

#[derive(Debug, Deserialize)]
struct Release {
    tag_name: String,
    #[serde(default)]
    assets: Vec<Asset>,
}

#[derive(Debug, Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

/// A discovered Proton install directory suitable for PROTONPATH.
#[derive(Debug, Clone)]
pub struct ProtonInstall {
    pub name: String,
    pub path: PathBuf,
    pub source: &'static str,
}

pub fn list_installs(paths: &Paths) -> Result<Vec<ProtonInstall>> {
    let mut out = Vec::new();

    if paths.runtimes_dir.is_dir() {
        collect_from_dir(&paths.runtimes_dir, "managed", &mut out)?;
    }
    for steam_dir in steam_compat_dirs() {
        if steam_dir.is_dir() {
            collect_from_dir(&steam_dir, "steam", &mut out)?;
        }
    }

    out.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.source.cmp(b.source)));
    Ok(out)
}

fn collect_from_dir(dir: &Path, source: &'static str, out: &mut Vec<ProtonInstall>) -> Result<()> {
    // Prefer canonical paths so ~/.steam/steam and ~/.local/share/Steam (often the same
    // tree via symlinks) do not list the same Proton thrice.
    let dir = fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    for entry in fs::read_dir(&dir)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let path = fs::canonicalize(&path).unwrap_or(path);
        if is_valid_proton_dir(&path) {
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string();
            if out.iter().any(|p| p.path == path) {
                continue;
            }
            out.push(ProtonInstall { name, path, source });
        }
    }
    Ok(())
}

/// True if `path` looks like a Proton compatibility tool root (PROTONPATH).
pub fn is_valid_proton_dir(path: &Path) -> bool {
    path.join("proton").is_file() || path.join("compatibilitytool.vdf").is_file()
}

fn steam_compat_dirs() -> Vec<PathBuf> {
    let home = directories::BaseDirs::new().map(|b| b.home_dir().to_path_buf());
    let Some(home) = home else {
        return Vec::new();
    };
    vec![
        home.join(".steam/steam/compatibilitytools.d"),
        home.join(".local/share/Steam/compatibilitytools.d"),
        home.join(".steam/root/compatibilitytools.d"),
    ]
}

/// Resolve PROTONPATH for a logical name or absolute path.
pub fn resolve_proton_path(
    paths: &Paths,
    cfg: &ProtonConfig,
    requested: &str,
) -> Result<PathBuf> {
    if let Some(explicit) = &cfg.explicit_path {
        if is_valid_proton_dir(explicit) {
            return Ok(explicit.clone());
        }
        return Err(Error::InvalidProtonPath(explicit.clone()));
    }

    let req = if requested == "dwproton" || requested.is_empty() {
        cfg.default.as_str()
    } else {
        requested
    };

    // Absolute path override
    let as_path = PathBuf::from(req);
    if as_path.is_absolute() {
        if is_valid_proton_dir(&as_path) {
            return Ok(as_path);
        }
        return Err(Error::InvalidProtonPath(as_path));
    }

    let installs = list_installs(paths)?;

    // Exact directory name match
    if let Some(found) = installs.iter().find(|i| i.name == req) {
        return Ok(found.path.clone());
    }

    // Logical "dwproton" → newest dwproton-* preferring configured source
    if req == "dwproton" {
        let mut candidates: Vec<&ProtonInstall> = installs
            .iter()
            .filter(|i| i.name.starts_with("dwproton"))
            .collect();
        if candidates.is_empty() {
            return Err(Error::ProtonNotFound);
        }
        // prefer managed or steam
        let prefer = cfg.prefer.as_str();
        candidates.sort_by(|a, b| {
            let rank = |s: &str| match (prefer, s) {
                ("steam", "steam") => 0,
                ("managed", "managed") => 0,
                _ => 1,
            };
            rank(a.source)
                .cmp(&rank(b.source))
                .then_with(|| b.name.cmp(&a.name)) // newest-ish by name
        });
        return Ok(candidates[0].path.clone());
    }

    // Prefix match (e.g. GE-Proton)
    if let Some(found) = installs.iter().rev().find(|i| i.name.starts_with(req)) {
        return Ok(found.path.clone());
    }

    Err(Error::ProtonNotFound)
}

pub fn install_dwproton(paths: &Paths, version: Option<&str>, force: bool) -> Result<PathBuf> {
    paths.ensure_dirs()?;
    let client = reqwest::blocking::Client::builder()
        .user_agent(concat!("protoncli/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| Error::Download(e.to_string()))?;

    let release: Release = if let Some(tag) = version {
        let url = format!("{DWPROTON_API_TAG}/{tag}");
        println!("==> Fetching release {tag}");
        client
            .get(&url)
            .send()
            .and_then(|r| r.error_for_status())
            .and_then(|r| r.json())
            .map_err(|e| Error::Download(format!("release metadata: {e}")))?
    } else {
        println!("==> Fetching latest dwproton release");
        client
            .get(DWPROTON_API_LATEST)
            .send()
            .and_then(|r| r.error_for_status())
            .and_then(|r| r.json())
            .map_err(|e| Error::Download(format!("latest release: {e}")))?
    };

    let tag = release.tag_name;
    let asset_name = format!("{tag}-{ARCH}.tar.xz");
    let sum_name = format!("{tag}-{ARCH}.sha512sum");
    let dir_name = format!("{tag}-{ARCH}");
    let target = paths.runtimes_dir.join(&dir_name);

    if target.is_dir() && is_valid_proton_dir(&target) && !force {
        println!("==> Already installed: {}", target.display());
        return Ok(target);
    }

    let tarball_url = release
        .assets
        .iter()
        .find(|a| a.name == asset_name)
        .map(|a| a.browser_download_url.clone())
        .unwrap_or_else(|| {
            format!(
                "https://dawn.wine/dawn-winery/dwproton/releases/download/{tag}/{asset_name}"
            )
        });
    let sum_url = release
        .assets
        .iter()
        .find(|a| a.name == sum_name)
        .map(|a| a.browser_download_url.clone())
        .unwrap_or_else(|| {
            format!("https://dawn.wine/dawn-winery/dwproton/releases/download/{tag}/{sum_name}")
        });

    println!("==> Version:  {tag}");
    println!("==> Tarball:  {tarball_url}");

    let tmp = paths.runtimes_dir.join(format!(".tmp-{dir_name}"));
    if tmp.exists() {
        fs::remove_dir_all(&tmp)?;
    }
    fs::create_dir_all(&tmp)?;

    let tarball_path = tmp.join(&asset_name);
    let sum_path = tmp.join(&sum_name);

    download_file(&client, &sum_url, &sum_path)?;
    download_file(&client, &tarball_url, &tarball_path)?;

    let expected = parse_sha512sum(&fs::read_to_string(&sum_path)?)?;
    let actual = sha512_file(&tarball_path)?;
    if !expected.eq_ignore_ascii_case(&actual) {
        return Err(Error::ChecksumMismatch {
            path: tarball_path,
            expected,
            actual,
        });
    }
    println!("==> Checksum OK");

    // Extract into tmp then rename into place (atomic-ish).
    let extract_root = tmp.join("extract");
    fs::create_dir_all(&extract_root)?;
    extract_tar_xz(&tarball_path, &extract_root)?;

    // Tarball usually contains a top-level dir_name/.
    let extracted = find_extracted_proton(&extract_root, &dir_name)?;

    if target.exists() {
        fs::remove_dir_all(&target)?;
    }
    fs::rename(&extracted, &target).or_else(|_| {
        // Cross-device rename fallback
        copy_dir_all(&extracted, &target)?;
        fs::remove_dir_all(&extracted)?;
        Ok::<(), Error>(())
    })?;

    fs::remove_dir_all(&tmp)?;

    if !is_valid_proton_dir(&target) {
        return Err(Error::InvalidProtonPath(target));
    }

    println!("==> Installed: {}", target.display());
    Ok(target)
}

fn download_file(
    client: &reqwest::blocking::Client,
    url: &str,
    dest: &Path,
) -> Result<()> {
    let response = client
        .get(url)
        .send()
        .and_then(|r| r.error_for_status())
        .map_err(|e| Error::Download(e.to_string()))?;

    let total = response.content_length();
    let pb = if let Some(len) = total {
        let pb = ProgressBar::new(len);
        pb.set_style(
            ProgressStyle::with_template(
                "{msg} [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta})",
            )
            .unwrap()
            .progress_chars("=>-"),
        );
        pb.set_message(
            dest.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("download")
                .to_string(),
        );
        pb
    } else {
        ProgressBar::new_spinner()
    };

    let mut file = File::create(dest)?;
    let mut reader = response;
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = reader
            .read(&mut buf)
            .map_err(|e| Error::Download(e.to_string()))?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])?;
        pb.inc(n as u64);
    }
    pb.finish_and_clear();
    Ok(())
}

fn parse_sha512sum(text: &str) -> Result<String> {
    // Format: "<hex>  filename" or "<hex> *filename"
    let line = text
        .lines()
        .find(|l| !l.trim().is_empty())
        .ok_or_else(|| Error::Download("empty sha512sum file".into()))?;
    let hex = line
        .split_whitespace()
        .next()
        .ok_or_else(|| Error::Download("malformed sha512sum".into()))?;
    if hex.len() != 128 {
        return Err(Error::Download(format!(
            "unexpected sha512 length {}",
            hex.len()
        )));
    }
    Ok(hex.to_string())
}

fn sha512_file(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha512::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

fn extract_tar_xz(tarball: &Path, dest: &Path) -> Result<()> {
    let file = File::open(tarball)?;
    let decompressor = xz2::read::XzDecoder::new(file);
    let mut archive = tar::Archive::new(decompressor);
    archive.unpack(dest)?;
    Ok(())
}

fn find_extracted_proton(extract_root: &Path, dir_name: &str) -> Result<PathBuf> {
    let direct = extract_root.join(dir_name);
    if is_valid_proton_dir(&direct) {
        return Ok(direct);
    }
    // Single top-level directory
    let mut dirs = fs::read_dir(extract_root)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect::<Vec<_>>();
    if dirs.len() == 1 && is_valid_proton_dir(&dirs[0]) {
        return Ok(dirs.remove(0));
    }
    // Nested search one level
    for d in fs::read_dir(extract_root)? {
        let d = d?.path();
        if is_valid_proton_dir(&d) {
            return Ok(d);
        }
    }
    Err(Error::Other(anyhow::anyhow!(
        "could not find Proton directory inside tarball (expected {dir_name})"
    )))
}

fn copy_dir_all(src: &Path, dst: &Path) -> io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let to = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_all(&entry.path(), &to)?;
        } else {
            fs::copy(entry.path(), to)?;
        }
    }
    Ok(())
}
