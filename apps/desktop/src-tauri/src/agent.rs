//! Serves the vault to the CLI and agents on this device (see `getkeys_core::endpoint`).
use crate::Core;
use anyhow::{Context, Result};
use getkeys_core::{commands::Command, endpoint};
use interprocess::local_socket::{ListenerOptions, Stream, prelude::*};
use serde_json::Value;
use std::{
    io::{BufRead, BufReader, Write},
    sync::Arc,
};
use tauri::{AppHandle, Emitter, Manager};

type Handler = Arc<dyn Fn(Command) -> Result<Value> + Send + Sync>;

pub fn start(app: &AppHandle) {
    let core = app.state::<Core>().inner().clone();
    let Some(dir) = core.lock().ok().and_then(|guard| {
        let app = guard.as_ref().ok()?;
        Some(app.vault.path().parent()?.to_owned())
    }) else {
        return;
    };
    let handle = app.clone();
    let handler: Handler = Arc::new(move |command: Command| {
        let mutates = command.mutates();
        let forked = {
            let guard = core
                .lock()
                .map_err(|_| anyhow::anyhow!("Application state is unavailable"))?;
            let app = guard.as_ref().map_err(|e| anyhow::anyhow!("{e}"))?;
            app.fork()?
        };
        let result = forked.dispatch(command);
        if mutates {
            let _ = handle.emit("vault-changed", ());
        }
        result
    });
    if let Err(e) = listen(&endpoint::socket(&dir), handler) {
        eprintln!("The agent endpoint is unavailable: {e:#}");
    }
}

fn listen(socket: &str, handler: Handler) -> Result<()> {
    #[cfg(unix)]
    let _ = std::fs::remove_file(socket);
    let options = ListenerOptions::new().name(endpoint::name(socket)?);
    #[cfg(unix)]
    let options = {
        use interprocess::os::unix::local_socket::ListenerOptionsExt;
        options.mode(0o600)
    };
    let listener = options
        .create_sync()
        .with_context(|| format!("Unable to serve {socket}"))?;
    std::thread::spawn(move || {
        for stream in listener.incoming().filter_map(Result::ok) {
            let handler = handler.clone();
            std::thread::spawn(move || serve(stream, &handler));
        }
    });
    Ok(())
}

fn serve(stream: Stream, handler: &Handler) {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    while reader.read_line(&mut line).is_ok_and(|n| n > 0) {
        let result = serde_json::from_str::<endpoint::Request>(&line)
            .context("Invalid request")
            .and_then(|request| handler(request.command));
        line.clear();
        if reader
            .get_mut()
            .write_all(endpoint::reply(result).as_bytes())
            .is_err()
        {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_requests() {
        let socket = if cfg!(windows) {
            format!("getkeys-agent-test-{}", std::process::id())
        } else {
            std::env::temp_dir()
                .join(format!("getkeys-agent-test-{}.sock", std::process::id()))
                .display()
                .to_string()
        };
        listen(
            &socket,
            Arc::new(|command| match command {
                Command::Snapshot => Ok(serde_json::json!({"keys": 1})),
                _ => anyhow::bail!("Not here"),
            }),
        )
        .unwrap();
        assert_eq!(
            endpoint::call(&socket, &Command::Snapshot).unwrap(),
            Some(serde_json::json!({"keys": 1}))
        );
        let error = endpoint::call(&socket, &Command::GetPrefs).unwrap_err();
        assert_eq!(error.to_string(), "Not here");
        assert!(
            endpoint::call("getkeys-agent-missing", &Command::Snapshot)
                .unwrap()
                .is_none()
        );
    }
}
