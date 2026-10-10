//! The local endpoint the desktop app serves, so the CLI and agents share its vault.
//! Newline-delimited JSON: `{"command": Command}` answered by `{"ok": value}` or `{"error": "…"}`.
use crate::commands::Command;
use anyhow::{Context, Result, bail};
use interprocess::local_socket::{Name, Stream, prelude::*};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    io::{BufRead, BufReader, Write},
    path::Path,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub command: Command,
}

/// Where the app serves the vault in `data_dir`: a named pipe on Windows, a socket file elsewhere.
pub fn socket(data_dir: &Path) -> String {
    if cfg!(windows) {
        if crate::App::data_dir().is_ok_and(|d| d == data_dir) {
            "getkeys-agent".into()
        } else {
            let hash = Sha256::digest(data_dir.to_string_lossy().as_bytes());
            format!("getkeys-agent-{}", &hex::encode(hash)[..12])
        }
    } else {
        data_dir.join("agent.sock").display().to_string()
    }
}

pub fn name(socket: &str) -> std::io::Result<Name<'_>> {
    #[cfg(windows)]
    return socket.to_ns_name::<interprocess::local_socket::GenericNamespaced>();
    #[cfg(not(windows))]
    return socket.to_fs_name::<interprocess::local_socket::GenericFilePath>();
}

/// One reply line.
pub fn reply(result: Result<Value>) -> String {
    let reply = match result {
        Ok(value) => json!({ "ok": value }),
        Err(e) => json!({ "error": format!("{e:#}") }),
    };
    reply.to_string() + "\n"
}

/// Runs a command in the GetKeys app. None when the app isn't running.
pub fn call(socket: &str, command: &Command) -> Result<Option<Value>> {
    let Ok(stream) = Stream::connect(name(socket)?) else {
        return Ok(None);
    };
    let mut stream = BufReader::new(stream);
    let request = json!({ "command": command }).to_string() + "\n";
    stream.get_mut().write_all(request.as_bytes())?;
    let mut line = String::new();
    stream.read_line(&mut line)?;
    let mut reply: Value = serde_json::from_str(&line).context("GetKeys sent an invalid reply")?;
    if let Some(error) = reply.get("error") {
        bail!("{}", error.as_str().unwrap_or("GetKeys failed"));
    }
    Ok(Some(
        reply.get_mut("ok").map(Value::take).unwrap_or_default(),
    ))
}
