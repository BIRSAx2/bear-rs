use std::env;
use std::path::PathBuf;

use anyhow::{Result, anyhow};

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

#[cfg(test)]
mod tests {
    use super::expand_tilde;

    #[test]
    fn expands_tilde() {
        let expanded = expand_tilde("~/tmp").expect("tilde should expand");
        assert!(expanded.to_string_lossy().ends_with("/tmp"));
    }
}
