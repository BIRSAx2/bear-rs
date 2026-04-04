use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;

pub const DEFAULT_DATABASE_PATH: &str =
    "~/Library/Group Containers/9K33E3U3T4.net.shinyfrog.bear/Application Data/database.sqlite";

pub fn expand_tilde(path: &str) -> Result<PathBuf> {
    if let Some(rest) = path.strip_prefix("~/") {
        let home = env::var_os("HOME").ok_or_else(|| anyhow!("$HOME is not set"))?;
        return Ok(PathBuf::from(home).join(rest));
    }

    Ok(PathBuf::from(path))
}

pub fn app_support_dir() -> Result<PathBuf> {
    let home = env::var_os("HOME").ok_or_else(|| anyhow!("$HOME is not set"))?;
    Ok(PathBuf::from(home)
        .join("Library")
        .join("Application Support")
        .join("bear-cli"))
}

pub fn token_path() -> Result<PathBuf> {
    Ok(app_support_dir()?.join("token"))
}

pub fn save_token(token: &str) -> Result<()> {
    let dir = app_support_dir()?;
    fs::create_dir_all(&dir).with_context(|| format!("failed to create {}", dir.display()))?;
    fs::write(token_path()?, format!("{token}\n")).context("failed to write token file")?;
    Ok(())
}

pub fn load_token() -> Result<Option<String>> {
    let path = token_path()?;
    match fs::read_to_string(&path) {
        Ok(contents) => Ok(Some(contents.trim().to_string())),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err).with_context(|| format!("failed to read {}", path.display())),
    }
}

pub fn encode_file(path: &Path) -> Result<String> {
    let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    Ok(BASE64_STANDARD.encode(bytes))
}

#[cfg(test)]
mod tests {
    use super::expand_tilde;

    #[test]
    fn expands_tilde() {
        let expanded = expand_tilde("~/tmp").expect("tilde should expand");
        assert!(expanded.to_string_lossy().ends_with("/tmp"));
    }
}
