//! Conversions between filesystem paths and `file:` URIs for LSP communication.

use anyhow::{anyhow, Result};
use std::path::{Path, PathBuf};
use url::Url;

pub fn path_to_uri(path: &Path) -> Result<String> {
    if path.to_str().is_none() {
        return Err(anyhow!(
            "{} is not valid UTF-8; rust-analyzer only handles UTF-8 paths",
            path.display()
        ));
    }
    if !path.is_absolute() {
        return Err(anyhow!("{} is not an absolute path", path.display()));
    }
    Url::from_file_path(path)
        .map(|u| u.to_string())
        .map_err(|()| anyhow!("{} cannot be represented as a file URI", path.display()))
}

pub fn uri_to_path(uri: &str) -> Option<PathBuf> {
    let url = Url::parse(uri).ok()?;
    if url.scheme() != "file" {
        return None;
    }
    url.to_file_path().ok()
}

/// Returns a canonical absolute path, avoiding the `\\?\` extended form on Windows.
pub fn absolute(path: &Path) -> PathBuf {
    if let Ok(canonical) = dunce::canonicalize(path) {
        return canonical;
    }
    if path.is_absolute() {
        return path.to_path_buf();
    }
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(path)
}

/// Normalizes a URI for use as a map key, settling on one spelling of the Windows drive letter.
pub fn normalize(uri: &str) -> String {
    let Ok(url) = Url::parse(uri) else {
        return uri.to_string();
    };
    if url.scheme() != "file" {
        return uri.to_string();
    }

    let Some((drive, drive_len)) = drive_letter(url.path()) else {
        return url.to_string();
    };

    let mut url = url;
    let normalized = format!(
        "/{}:{}",
        drive.to_ascii_uppercase(),
        &url.path()[drive_len..]
    );
    url.set_path(&normalized);
    url.to_string()
}

fn drive_letter(path: &str) -> Option<(char, usize)> {
    let letter = path.strip_prefix('/')?.chars().next()?;
    if !letter.is_ascii_alphabetic() {
        return None;
    }
    let rest = &path[2..];
    let colon_len = if rest.starts_with(':') || rest.starts_with('|') {
        1
    } else if rest.starts_with("%3A") || rest.starts_with("%3a") {
        3
    } else {
        return None;
    };
    let drive_len = 2 + colon_len;
    match path[drive_len..].chars().next() {
        Some('/') | None => Some((letter, drive_len)),
        _ => None,
    }
}
