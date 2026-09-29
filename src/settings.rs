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
    pub accounts: Vec<Account>,
    pub selected_account: usize,
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
            accounts: Vec::new(),
            selected_account: 0,
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

    pub fn account(&self) -> Option<&Account> {
        self.accounts.get(self.selected_account)
    }
}
