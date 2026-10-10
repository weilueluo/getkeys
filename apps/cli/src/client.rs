use anyhow::{Context, Result};
use getkeys_core::{App, commands::Command, endpoint, model::Snapshot};
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::path::{Path, PathBuf};

/// The vault, through the GetKeys app when it is running, else opened here.
pub struct Client {
    socket: String,
    data_dir: PathBuf,
    registry: Option<PathBuf>,
    direct: Option<App>,
}

impl Client {
    pub fn new(data_dir: PathBuf, registry: Option<PathBuf>) -> Self {
        Self {
            socket: endpoint::socket(&data_dir),
            data_dir,
            registry,
            direct: None,
        }
    }
    pub fn call(&mut self, command: Command) -> Result<Value> {
        if self.direct.is_none() {
            if let Some(value) = endpoint::call(&self.socket, &command)? {
                return Ok(value);
            }
            let registry = self.registry.clone().unwrap_or_else(registry);
            self.direct = Some(App::open(self.data_dir.clone(), registry)?);
        }
        self.direct
            .as_ref()
            .context("The vault is unavailable")?
            .dispatch(command)
    }
    pub fn get<T: DeserializeOwned>(&mut self, command: Command) -> Result<T> {
        Ok(serde_json::from_value(self.call(command)?)?)
    }
    pub fn snapshot(&mut self) -> Result<Snapshot> {
        self.get(Command::Snapshot)
    }
}

/// The provider catalog shipped with GetKeys, next to this program.
fn registry() -> PathBuf {
    let exe = std::env::current_exe().unwrap_or_default();
    let dir = exe.parent().unwrap_or(Path::new("."));
    let mut candidates = vec![
        dir.join("registry"),
        dir.join("../registry"),
        dir.join("../Resources/registry"),
        dir.join("../lib/GetKeys/registry"),
    ];
    if cfg!(debug_assertions) {
        candidates.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../registry/dist"));
    }
    candidates
        .into_iter()
        .find(|p| p.join("index.json").is_file())
        .unwrap_or_else(|| PathBuf::from("registry/dist"))
}
