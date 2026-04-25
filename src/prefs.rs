use std::path::PathBuf;

use anyhow::Result;

use crate::db::group_container_path;
use crate::model::TagPosition;

const PREFS_RELATIVE: &str = "Library/Preferences/9K33E3U3T4.net.shinyfrog.bear.plist";

/// Bear user preferences relevant to the CLI.
#[derive(Debug)]
pub struct BearPrefs {
    pub tag_position: TagPosition,
    pub app_locking_enabled: bool,
}

impl Default for BearPrefs {
    fn default() -> Self {
        BearPrefs {
            tag_position: TagPosition::Bottom,
            app_locking_enabled: false,
        }
    }
}

/// Absolute path to the Bear preferences plist.
pub fn prefs_path() -> Result<PathBuf> {
    Ok(group_container_path()?.join(PREFS_RELATIVE))
}

/// Load Bear preferences from the shared group container plist.
/// Falls back to defaults if the file is missing or a key is absent.
pub fn load_prefs() -> Result<BearPrefs> {
    let path = prefs_path()?;
    if !path.exists() {
        return Ok(BearPrefs::default());
    }

    let dict: plist::Dictionary = plist::from_file(&path)?;

    let tag_position = match dict.get("SFGCTagPosition").and_then(|v| v.as_string()) {
        Some("SFTagPositionTop") => TagPosition::Top,
        _ => TagPosition::Bottom,
    };

    let app_locking_enabled = dict
        .get("applicationLockingEnabled")
        .and_then(|v| v.as_boolean())
        .unwrap_or(false);

    Ok(BearPrefs {
        tag_position,
        app_locking_enabled,
    })
}

/// Check the app lock and bail with the native error message if enabled.
pub fn check_app_lock() -> Result<()> {
    if load_prefs()?.app_locking_enabled {
        anyhow::bail!("Bear's app lock is enabled; disable it in Bear's settings to use the CLI.");
    }
    Ok(())
}
