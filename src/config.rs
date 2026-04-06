use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;

const BEAR_GROUP_CONTAINER_SUFFIX: &str = ".net.shinyfrog.bear";
const BEAR_DATABASE_SUFFIX: &str = "Application Data/database.sqlite";

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

pub fn resolve_database_path(override_path: Option<&str>) -> Result<PathBuf> {
    if let Some(path) = override_path {
        return expand_tilde(path);
    }

    let home = env::var_os("HOME").ok_or_else(|| anyhow!("$HOME is not set"))?;
    let group_containers = PathBuf::from(home).join("Library").join("Group Containers");

    find_bear_database_in(&group_containers)
}

fn find_bear_database_in(group_containers: &Path) -> Result<PathBuf> {
    let entries = fs::read_dir(group_containers).with_context(|| {
        format!(
            "failed to read Bear group containers from {}",
            group_containers.display()
        )
    })?;

    let mut matches = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with(BEAR_GROUP_CONTAINER_SUFFIX))
        })
        .map(|path| path.join(BEAR_DATABASE_SUFFIX))
        .filter(|path| path.is_file())
        .collect::<Vec<_>>();

    matches.sort();

    matches.into_iter().next().ok_or_else(|| {
        anyhow!(
            "could not locate Bear database under {} matching *{}",
            group_containers.display(),
            BEAR_GROUP_CONTAINER_SUFFIX
        )
    })
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
    use std::fs;

    use super::{expand_tilde, find_bear_database_in};

    #[test]
    fn expands_tilde() {
        let expanded = expand_tilde("~/tmp").expect("tilde should expand");
        assert!(expanded.to_string_lossy().ends_with("/tmp"));
    }

    #[test]
    fn finds_database_dynamically() {
        let base =
            std::env::temp_dir().join(format!("bear-cli-config-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let group = base.join("ABC123.net.shinyfrog.bear");
        let database = group.join("Application Data").join("database.sqlite");
        fs::create_dir_all(database.parent().expect("database parent"))
            .expect("test directories should be created");
        fs::write(&database, b"").expect("database file should be created");

        let found = find_bear_database_in(&base).expect("database should be discovered");
        assert_eq!(found, database);

        let _ = fs::remove_dir_all(&base);
    }
}
