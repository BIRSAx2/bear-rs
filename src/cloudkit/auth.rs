use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};

use crate::config::app_support_dir;

const KEYCHAIN_SERVICE: &str = "bear-cli";
const KEYCHAIN_ACCOUNT: &str = "ckWebAuthToken";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthConfig {
    pub ck_web_auth_token: String,
}

impl AuthConfig {
    /// Load from Keychain first, fall back to the config file.
    pub fn load() -> Result<Self> {
        if let Ok(token) = keychain_get() {
            return Ok(Self {
                ck_web_auth_token: token,
            });
        }
        Self::load_from_file()
    }

    /// Save to both Keychain and the config file.
    pub fn save(&self) -> Result<()> {
        let _ = keychain_set(&self.ck_web_auth_token); // best-effort
        self.save_to_file()
    }

    fn config_path() -> Result<PathBuf> {
        Ok(app_support_dir()?.join("auth.json"))
    }

    fn load_from_file() -> Result<Self> {
        let path = Self::config_path()?;
        let contents = fs::read_to_string(&path).with_context(|| {
            format!(
                "auth token not found — run `bear auth <token>` first (checked {})",
                path.display()
            )
        })?;
        serde_json::from_str(&contents).context("invalid auth config")
    }

    fn save_to_file(&self) -> Result<()> {
        let path = Self::config_path()?;
        fs::create_dir_all(path.parent().unwrap())?;
        let json = serde_json::to_string_pretty(self)?;
        // Atomic write
        let tmp = path.with_extension("tmp");
        fs::write(&tmp, json)?;
        fs::rename(&tmp, &path)?;
        Ok(())
    }
}

fn keychain_get() -> Result<String> {
    let output = std::process::Command::new("security")
        .args([
            "find-generic-password",
            "-s",
            KEYCHAIN_SERVICE,
            "-a",
            KEYCHAIN_ACCOUNT,
            "-w",
        ])
        .output()?;
    if !output.status.success() {
        return Err(anyhow!("keychain lookup failed"));
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_string())
}

fn keychain_set(token: &str) -> Result<()> {
    // Delete existing entry first (ignore errors)
    let _ = std::process::Command::new("security")
        .args([
            "delete-generic-password",
            "-s",
            KEYCHAIN_SERVICE,
            "-a",
            KEYCHAIN_ACCOUNT,
        ])
        .output();

    let status = std::process::Command::new("security")
        .args([
            "add-generic-password",
            "-s",
            KEYCHAIN_SERVICE,
            "-a",
            KEYCHAIN_ACCOUNT,
            "-w",
            token,
        ])
        .status()?;
    if !status.success() {
        return Err(anyhow!("failed to write token to Keychain"));
    }
    Ok(())
}
