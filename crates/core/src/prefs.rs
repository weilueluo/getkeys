use crate::App;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use ts_rs::TS;

/// Settings kept next to the vault. They hold no secrets.
#[derive(Clone, Debug, Serialize, Deserialize, TS, PartialEq)]
#[serde(default)]
pub struct Prefs {
    /// Agents may use keys through `getkeys mcp`.
    pub mcp_enabled: bool,
}
impl Default for Prefs {
    fn default() -> Self {
        Self { mcp_enabled: true }
    }
}

impl App {
    fn prefs_path(&self) -> PathBuf {
        self.vault.path().with_file_name("prefs.json")
    }
    pub fn prefs(&self) -> Result<Prefs> {
        match std::fs::read(self.prefs_path()) {
            Ok(bytes) => serde_json::from_slice(&bytes).context("Settings are damaged"),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Prefs::default()),
            Err(e) => Err(e).context("Unable to read settings"),
        }
    }
    pub fn set_prefs(&self, prefs: Prefs) -> Result<Prefs> {
        std::fs::write(self.prefs_path(), serde_json::to_vec_pretty(&prefs)?)
            .context("Unable to save settings")?;
        self.vault.event(
            "settings.changed",
            if prefs.mcp_enabled {
                "mcp on"
            } else {
                "mcp off"
            },
        )?;
        Ok(prefs)
    }
}
