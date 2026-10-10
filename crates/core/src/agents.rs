//! Sets up AI agent apps to use `getkeys mcp`.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use ts_rs::TS;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, TS, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentClient {
    ClaudeDesktop,
    ClaudeCode,
    Cursor,
    Vscode,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct AgentInfo {
    pub client: AgentClient,
    pub name: String,
    /// The client's MCP configuration file.
    pub path: String,
    /// Whether that file already starts GetKeys.
    pub configured: bool,
}

/// Name of the server entry GetKeys adds.
const SERVER: &str = "getkeys";

impl AgentClient {
    pub const ALL: [Self; 4] = [
        Self::ClaudeDesktop,
        Self::ClaudeCode,
        Self::Cursor,
        Self::Vscode,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::ClaudeDesktop => "Claude Desktop",
            Self::ClaudeCode => "Claude Code",
            Self::Cursor => "Cursor",
            Self::Vscode => "VS Code",
        }
    }
    pub fn config(self) -> Result<PathBuf> {
        let dirs = directories::BaseDirs::new().context("The home folder is unavailable")?;
        Ok(match self {
            Self::ClaudeDesktop => dirs.config_dir().join("Claude/claude_desktop_config.json"),
            Self::ClaudeCode => dirs.home_dir().join(".claude.json"),
            Self::Cursor => dirs.home_dir().join(".cursor/mcp.json"),
            Self::Vscode => dirs.config_dir().join("Code/User/mcp.json"),
        })
    }
    fn servers(self) -> &'static str {
        match self {
            Self::Vscode => "servers",
            _ => "mcpServers",
        }
    }
    fn entry(self, cli: &Path) -> Value {
        let command = cli.display().to_string();
        match self {
            Self::Vscode => json!({"type": "stdio", "command": command, "args": ["mcp"]}),
            _ => json!({"command": command, "args": ["mcp"]}),
        }
    }
}

fn read(path: &Path) -> Result<Value> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(json!({})),
        Err(e) => return Err(e).with_context(|| format!("Unable to read {}", path.display())),
    };
    if text.trim().is_empty() {
        return Ok(json!({}));
    }
    let value: Value = serde_json::from_str(&text).with_context(|| {
        format!(
            "{} isn't valid JSON. Fix it, then try again.",
            path.display()
        )
    })?;
    ensure!(value.is_object(), "{} isn't a JSON object", path.display());
    Ok(value)
}

/// Adds GetKeys to a client's MCP servers in `path`, keeping everything else in the file.
pub fn connect(client: AgentClient, path: &Path, cli: &Path) -> Result<()> {
    let mut config = read(path)?;
    let servers = config
        .as_object_mut()
        .context("Invalid configuration")?
        .entry(client.servers())
        .or_insert_with(|| json!({}));
    ensure!(
        servers.is_object(),
        "{} in {} isn't a JSON object",
        client.servers(),
        path.display()
    );
    servers[SERVER] = client.entry(cli);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, serde_json::to_string_pretty(&config)? + "\n")
        .with_context(|| format!("Unable to write {}", path.display()))
}

/// Removes GetKeys from a client's MCP servers in `path`, keeping everything else in the file.
pub fn disconnect(client: AgentClient, path: &Path) -> Result<()> {
    let mut config = read(path)?;
    let removed = config
        .get_mut(client.servers())
        .and_then(Value::as_object_mut)
        .and_then(|servers| servers.remove(SERVER));
    if removed.is_none() {
        return Ok(());
    }
    std::fs::write(path, serde_json::to_string_pretty(&config)? + "\n")
        .with_context(|| format!("Unable to write {}", path.display()))
}

pub fn configured(client: AgentClient, path: &Path) -> bool {
    read(path).is_ok_and(|c| c[client.servers()][SERVER].is_object())
}

pub fn status() -> Result<Vec<AgentInfo>> {
    AgentClient::ALL
        .iter()
        .map(|&client| {
            let path = client.config()?;
            Ok(AgentInfo {
                client,
                name: client.name().into(),
                configured: configured(client, &path),
                path: path.display().to_string(),
            })
        })
        .collect()
}

/// The `getkeys` command installed next to the running program.
pub fn cli_path() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("GETKEYS_CLI") {
        return Ok(path.into());
    }
    let exe = std::env::current_exe()?;
    let cli = exe.with_file_name(format!("getkeys{}", std::env::consts::EXE_SUFFIX));
    ensure!(
        cli.is_file(),
        "The getkeys command isn't installed next to GetKeys. Reinstall GetKeys to add it."
    );
    Ok(cli)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn connecting_keeps_the_rest_of_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/mcp.json");
        let cli = Path::new("C:/GetKeys/getkeys.exe");
        assert!(!configured(AgentClient::Vscode, &path));
        connect(AgentClient::Vscode, &path, cli).unwrap();
        assert!(configured(AgentClient::Vscode, &path));
        let file = |c| {
            serde_json::from_str::<Value>(&std::fs::read_to_string(&path).unwrap()).unwrap()[c]
                .clone()
        };
        assert_eq!(
            file("servers")["getkeys"],
            json!({"type": "stdio", "command": "C:/GetKeys/getkeys.exe", "args": ["mcp"]})
        );
        std::fs::write(
            &path,
            r#"{"theme":"dark","mcpServers":{"other":{"command":"x"}}}"#,
        )
        .unwrap();
        connect(AgentClient::Cursor, &path, cli).unwrap();
        assert_eq!(file("theme"), "dark");
        assert_eq!(file("mcpServers")["other"]["command"], "x");
        assert_eq!(file("mcpServers")["getkeys"]["args"], json!(["mcp"]));
        std::fs::write(&path, "{ not json").unwrap();
        assert!(connect(AgentClient::Cursor, &path, cli).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ not json");
    }
    #[test]
    fn disconnecting_keeps_the_rest_of_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mcp.json");
        disconnect(AgentClient::Cursor, &path).unwrap();
        assert!(!path.exists());
        std::fs::write(
            &path,
            r#"{"theme":"dark","mcpServers":{"other":{"command":"x"}}}"#,
        )
        .unwrap();
        connect(AgentClient::Cursor, &path, Path::new("getkeys")).unwrap();
        assert!(configured(AgentClient::Cursor, &path));
        disconnect(AgentClient::Cursor, &path).unwrap();
        assert!(!configured(AgentClient::Cursor, &path));
        let file: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(file["theme"], "dark");
        assert_eq!(file["mcpServers"]["other"]["command"], "x");
    }
}
