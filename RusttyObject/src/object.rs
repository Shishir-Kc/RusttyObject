use mime_guess::from_path;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use walkdir::WalkDir;

pub const CONFIG_FILE_NAME: &str = "config.rustyobject";
const LEGACY_CONFIG_FILE_NAME: &str = ".rustyobject";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RustyConfig {
    pub version: u8,
    pub repository: String,
    pub branch: String,
    pub root: String,
    pub generated_at: u64,
    pub objects: Vec<ObjectEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObjectEntry {
    pub path: String,
    pub size: u64,
    pub sha256: String,
    pub content_type: String,
}

pub fn config_path(workspace: &Path) -> PathBuf {
    workspace.join(CONFIG_FILE_NAME)
}

pub fn build_config(
    workspace: &Path,
    repository: String,
    branch: String,
) -> io::Result<RustyConfig> {
    Ok(RustyConfig {
        version: 1,
        repository,
        branch,
        root: ".".to_string(),
        generated_at: now_unix(),
        objects: scan_workspace(workspace)?,
    })
}

pub fn scan_workspace(workspace: &Path) -> io::Result<Vec<ObjectEntry>> {
    let mut objects = Vec::new();
    let workspace = workspace.canonicalize()?;

    for entry in WalkDir::new(&workspace)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| !is_ignored_dir(entry.path(), &workspace))
    {
        let entry = entry.map_err(io::Error::other)?;
        if !entry.file_type().is_file()
            || entry
                .path()
                .file_name()
                .is_some_and(|name| name == CONFIG_FILE_NAME || name == LEGACY_CONFIG_FILE_NAME)
        {
            continue;
        }

        let bytes = fs::read(entry.path())?;
        let relative = entry
            .path()
            .strip_prefix(&workspace)
            .map_err(io::Error::other)?
            .to_string_lossy()
            .replace(std::path::MAIN_SEPARATOR, "/");
        let digest = format!("{:x}", Sha256::digest(&bytes));
        let content_type = from_path(entry.path())
            .first_or_octet_stream()
            .essence_str()
            .to_string();

        objects.push(ObjectEntry {
            path: relative,
            size: bytes.len() as u64,
            sha256: digest,
            content_type,
        });
    }

    objects.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(objects)
}

pub fn write_config(workspace: &Path, config: &RustyConfig) -> io::Result<()> {
    let json = serde_json::to_string_pretty(config).map_err(io::Error::other)?;
    fs::write(config_path(workspace), format!("{json}\n"))
}

pub fn read_config(workspace: &Path) -> io::Result<RustyConfig> {
    let path = if config_path(workspace).exists() {
        config_path(workspace)
    } else {
        workspace.join(LEGACY_CONFIG_FILE_NAME)
    };
    let contents = fs::read_to_string(path)?;
    serde_json::from_str(&contents).map_err(io::Error::other)
}

fn is_ignored_dir(path: &Path, workspace: &Path) -> bool {
    if path == workspace {
        return true;
    }

    path.file_name().is_some_and(|name| {
        matches!(
            name.to_str(),
            Some(".git" | "target" | "node_modules" | ".next" | "dist")
        )
    })
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}
