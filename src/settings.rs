use std::fs;
use std::io::Write;
use std::path::PathBuf;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::auth::Account;
use crate::platform;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Where versions, libraries, assets and Java runtimes live.
    pub data_dir: PathBuf,
    /// Game directory for plain versions launched without an instance. Empty = `data_dir`.
    /// Instances always use their own folder.
    pub game_dir: String,
    pub memory_mb: u32,
    /// Custom java binary. Empty = download Mojang's runtime automatically.
    pub java_path: String,
    pub jvm_args: String,
    pub ms_client_id: String,
    /// CurseForge API key (console.curseforge.com). `CURSEFORGE_API_KEY` overrides it.
    pub curseforge_api_key: String,
    pub accounts: Vec<Account>,
    pub selected_account: usize,
    /// Game window size; `None` = the game's default.
    pub window_width: Option<u32>,
    pub window_height: Option<u32>,
    pub fullscreen: bool,
    pub appearance: Appearance,
    pub on_launch: OnLaunch,
}

/// Color scheme of the launcher.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}

/// What the launcher window does once the game has started.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OnLaunch {
    #[default]
    KeepOpen,
    /// Minimize, and bring the launcher back when the game exits.
    Minimize,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            data_dir: platform::default_data_dir(),
            game_dir: String::new(),
            memory_mb: 4096,
            java_path: String::new(),
            jvm_args: String::new(),
            ms_client_id: String::new(),
            curseforge_api_key: String::new(),
            accounts: Vec::new(),
            selected_account: 0,
            window_width: None,
            window_height: None,
            fullscreen: false,
            appearance: Appearance::default(),
            on_launch: OnLaunch::default(),
        }
    }
}

fn settings_path() -> PathBuf {
    platform::default_data_dir().join("launcher.json")
}

impl Settings {
    pub fn load() -> Settings {
        fs::read(settings_path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    /// Saves settings; the file holds account tokens, so it is owner-readable only.
    pub fn save(&self) -> Result<()> {
        let path = settings_path();
        fs::create_dir_all(path.parent().unwrap())?;
        let mut options = fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
            options.mode(0o600);
            // `mode` only applies to new files; tighten files that already exist.
            if path.exists() {
                fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
            }
        }
        options.open(&path)?.write_all(&serde_json::to_vec_pretty(self)?)?;
        Ok(())
    }

    pub fn game_dir(&self) -> PathBuf {
        if self.game_dir.trim().is_empty() {
            self.data_dir.clone()
        } else {
            PathBuf::from(self.game_dir.trim())
        }
    }

    pub fn curseforge_key(&self) -> Option<String> {
        let key = std::env::var("CURSEFORGE_API_KEY").unwrap_or_else(|_| self.curseforge_api_key.clone());
        let key = key.trim();
        (!key.is_empty()).then(|| key.to_string())
    }

    /// Width and height when both are set.
    pub fn resolution(&self) -> Option<(u32, u32)> {
        self.window_width.zip(self.window_height)
    }

    pub fn account(&self) -> Option<&Account> {
        self.accounts.get(self.selected_account)
    }
}
