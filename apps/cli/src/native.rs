//! `getkeys native-host`: saves keys the GetKeys browser extension captures, over Chrome native
//! messaging (a 4-byte little-endian length, then UTF-8 JSON, on stdin and stdout).
use crate::client::Client;
use anyhow::{Context, Result, bail, ensure};
use getkeys_core::{
    commands::Command,
    model::{CredentialView, CustomField, Snapshot},
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::{ErrorKind, Read, Write},
    path::Path,
};

pub const HOST: &str = "dev.getkeys.host";
pub const ORIGIN: &str = "chrome-extension://mhdeafblmlpikemoliokafdegcdhoeed/";
const LIMIT: usize = 1024 * 1024;

#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Browser {
    Chrome,
    Edge,
    All,
}

pub fn serve(client: &mut Client, origin: &str) -> Result<()> {
    ensure!(
        origin == ORIGIN,
        "GetKeys only answers its own browser extension, not {origin}"
    );
    let mut stdin = std::io::stdin().lock();
    let mut stdout = std::io::stdout().lock();
    loop {
        let mut length = [0u8; 4];
        match stdin.read_exact(&mut length) {
            Err(e) if e.kind() == ErrorKind::UnexpectedEof => return Ok(()),
            result => result?,
        }
        let length = u32::from_le_bytes(length) as usize;
        ensure!(length <= LIMIT, "The message is too large");
        let mut message = vec![0; length];
        stdin.read_exact(&mut message)?;
        let reply = match serde_json::from_slice::<Value>(&message) {
            Ok(m) => handle(client, &m)
                .unwrap_or_else(|e| json!({"ok": false, "error": format!("{e:#}")})),
            Err(_) => json!({"ok": false, "error": "The message isn't valid JSON"}),
        };
        let bytes = serde_json::to_vec(&reply)?;
        stdout.write_all(&(bytes.len() as u32).to_le_bytes())?;
        stdout.write_all(&bytes)?;
        stdout.flush()?;
    }
}

fn handle(client: &mut Client, m: &Value) -> Result<Value> {
    Ok(match m["type"].as_str() {
        Some("ping") => json!({"ok": true, "version": env!("CARGO_PKG_VERSION")}),
        Some("patterns") => {
            let snapshot = client.snapshot()?;
            let patterns: Vec<Value> = snapshot
                .plugins
                .iter()
                .map(|p| &p.manifest)
                .filter(|m| !m.key_pattern.is_empty())
                .map(|m| json!({"provider": m.id, "name": m.name, "pattern": m.key_pattern}))
                .collect();
            json!({"ok": true, "patterns": patterns})
        }
        Some("save") => {
            let value = m["value"].as_str().unwrap_or_default().trim().to_owned();
            ensure!(!value.is_empty(), "There's no key to save");
            let name = m["name"].as_str().unwrap_or_default().trim().to_owned();
            let saved: CredentialView = match m["provider"].as_str() {
                Some(provider) => {
                    let snapshot: Snapshot = client.snapshot()?;
                    let manifest = &snapshot
                        .plugins
                        .iter()
                        .find(|p| p.manifest.id == provider)
                        .with_context(|| format!("GetKeys doesn't know the provider {provider}"))?
                        .manifest;
                    let field = manifest
                        .credential_fields
                        .iter()
                        .find(|f| f.secret)
                        .context("This provider's keys have no secret value")?;
                    ensure!(
                        !manifest
                            .credential_fields
                            .iter()
                            .any(|f| f.required && f.key != field.key),
                        "{} keys need more than one value. Add this one in GetKeys.",
                        manifest.name
                    );
                    client.get(Command::SaveKey {
                        credential_id: None,
                        plugin_id: provider.into(),
                        name,
                        fields: BTreeMap::from([(field.key.clone(), value)]),
                    })?
                }
                None => client.get(Command::SaveCustom {
                    credential_id: None,
                    name: if name.is_empty() {
                        default_name(m["url"].as_str())
                    } else {
                        name
                    },
                    fields: vec![CustomField {
                        variable: "API_KEY".into(),
                        value,
                        secret: true,
                    }],
                })?,
            };
            json!({"ok": true, "id": saved.id})
        }
        _ => bail!("Unknown request"),
    })
}

fn default_name(url: Option<&str>) -> String {
    url.and_then(|u| u.split("://").nth(1))
        .and_then(|rest| rest.split(['/', '?', '#']).next())
        .filter(|host| !host.is_empty())
        .map_or_else(|| "Saved key".into(), |host| format!("Key from {host}"))
}

fn browsers(browser: Browser) -> Vec<Browser> {
    match browser {
        Browser::All => vec![Browser::Chrome, Browser::Edge],
        b => vec![b],
    }
}

#[cfg(windows)]
fn registry_key(browser: Browser) -> String {
    let vendor = match browser {
        Browser::Edge => r"Microsoft\Edge",
        _ => r"Google\Chrome",
    };
    format!(r"Software\{vendor}\NativeMessagingHosts\{HOST}")
}

/// Where each browser looks for the host's manifest file.
#[cfg(not(windows))]
fn manifest_dir(browser: Browser) -> Result<std::path::PathBuf> {
    let home = directories::BaseDirs::new()
        .context("The home folder is unavailable")?
        .home_dir()
        .to_owned();
    let dir = match (browser, cfg!(target_os = "macos")) {
        (Browser::Edge, true) => "Library/Application Support/Microsoft Edge",
        (_, true) => "Library/Application Support/Google/Chrome",
        (Browser::Edge, false) => ".config/microsoft-edge",
        (_, false) => ".config/google-chrome",
    };
    Ok(home.join(dir).join("NativeMessagingHosts"))
}

/// Registers this program as the extension's native messaging host. Returns where.
pub fn install(browser: Browser, data_dir: &Path) -> Result<Vec<String>> {
    let manifest = json!({
        "name": HOST,
        "description": "GetKeys",
        "path": std::env::current_exe()?,
        "type": "stdio",
        "allowed_origins": [ORIGIN],
    });
    let text = serde_json::to_string_pretty(&manifest)?;
    let mut places = Vec::new();
    #[cfg(windows)]
    {
        std::fs::create_dir_all(data_dir)?;
        let file = data_dir.join(format!("{HOST}.json"));
        std::fs::write(&file, &text)?;
        let user = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER);
        for b in browsers(browser) {
            let path = registry_key(b);
            let (key, _) = user.create_subkey(&path)?;
            key.set_value("", &file.display().to_string())?;
            places.push(format!(r"HKCU\{path}"));
        }
        places.push(file.display().to_string());
    }
    #[cfg(not(windows))]
    {
        let _ = data_dir;
        for b in browsers(browser) {
            let dir = manifest_dir(b)?;
            std::fs::create_dir_all(&dir)?;
            let file = dir.join(format!("{HOST}.json"));
            std::fs::write(&file, &text)?;
            places.push(file.display().to_string());
        }
    }
    Ok(places)
}

pub fn uninstall(browser: Browser, data_dir: &Path) -> Result<()> {
    #[cfg(windows)]
    {
        let user = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER);
        for b in browsers(browser) {
            match user.delete_subkey_all(registry_key(b)) {
                Err(e) if e.kind() != ErrorKind::NotFound => return Err(e.into()),
                _ => {}
            }
        }
        if browser == Browser::All {
            remove(&data_dir.join(format!("{HOST}.json")))?;
        }
    }
    #[cfg(not(windows))]
    {
        let _ = data_dir;
        for b in browsers(browser) {
            remove(&manifest_dir(b)?.join(format!("{HOST}.json")))?;
        }
    }
    Ok(())
}

fn remove(path: &Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Err(e) if e.kind() != ErrorKind::NotFound => Err(e.into()),
        _ => Ok(()),
    }
}
