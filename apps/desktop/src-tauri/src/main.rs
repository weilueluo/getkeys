#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use getkeys_core::{App, commands::Command, model::EnvLine};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tauri::Manager;
use tauri_plugin_dialog::DialogExt;

type Core = Arc<Mutex<Result<App, String>>>;
fn message(e: anyhow::Error) -> String {
    format!("{e:#}")
}
/// Slow provider calls run on their own vault connection, so a refresh can list many accounts
/// at once while the window stays responsive.
fn parallel(command: &Command) -> bool {
    matches!(
        command,
        Command::Sync { .. }
            | Command::Resources { .. }
            | Command::ConnectLocal { .. }
            | Command::ScanSources { .. }
            | Command::Check { .. }
    )
}
#[tauri::command]
async fn request(
    state: tauri::State<'_, Core>,
    command: Command,
) -> Result<serde_json::Value, String> {
    let core = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let forked = {
            let guard = core
                .lock()
                .map_err(|_| "Application state is unavailable".to_owned())?;
            let app = guard.as_ref().map_err(Clone::clone)?;
            if !parallel(&command) {
                return app.dispatch(command).map_err(message);
            }
            app.fork().map_err(message)?
        };
        forked.dispatch(command).map_err(message)
    })
    .await
    .map_err(|_| "Application worker failed".to_owned())?
}
/// Puts a value on the clipboard and clears it after a while if it is still there.
fn copy(value: String, seconds: u64) -> Result<(), String> {
    let mut clipboard = arboard::Clipboard::new().map_err(|_| "Clipboard is unavailable")?;
    clipboard
        .set_text(&value)
        .map_err(|_| "Unable to copy credential")?;
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(seconds));
        if let Ok(mut clipboard) = arboard::Clipboard::new()
            && clipboard.get_text().is_ok_and(|current| current == value)
        {
            let _ = clipboard.clear();
        }
    });
    Ok(())
}
#[tauri::command]
async fn copy_credential(
    state: tauri::State<'_, Core>,
    credential_id: String,
    field: String,
) -> Result<(), String> {
    let core = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let value = {
            let guard = core
                .lock()
                .map_err(|_| "Application state is unavailable".to_owned())?;
            guard
                .as_ref()
                .map_err(Clone::clone)?
                .clip(&credential_id, &field)
                .map_err(message)?
        };
        copy(value, 30)
    })
    .await
    .map_err(|_| "Clipboard worker failed".to_owned())?
}
/// Writes the chosen keys as a `.env` file, or copies them. The values never reach the window.
/// Returns false when the person closes the save dialog.
#[tauri::command]
async fn export_env(
    window: tauri::Window,
    state: tauri::State<'_, Core>,
    lines: Vec<EnvLine>,
    save: bool,
) -> Result<bool, String> {
    let core = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let path = if save {
            let Some(path) = window
                .dialog()
                .file()
                .set_parent(&window)
                .set_file_name(".env")
                .blocking_save_file()
            else {
                return Ok(false);
            };
            Some(
                path.into_path()
                    .map_err(|_| "Choose a file on this device")?,
            )
        } else {
            None
        };
        let text = {
            let guard = core
                .lock()
                .map_err(|_| "Application state is unavailable".to_owned())?;
            guard
                .as_ref()
                .map_err(Clone::clone)?
                .export_env(&lines)
                .map_err(message)?
        };
        match path {
            Some(path) => std::fs::write(&path, text)
                .map_err(|e| format!("Unable to write {}: {e}", path.display()))?,
            None => copy(text, 60)?,
        }
        Ok(true)
    })
    .await
    .map_err(|_| "Export worker failed".to_owned())?
}
fn main() {
    let context = tauri::generate_context!();
    // Elevated WebView2 hosts ignore environment-supplied debugging arguments.
    // Pass them through its API for debug builds used by native UI tests.
    #[cfg(debug_assertions)]
    let context = {
        let mut context = context;
        if let Ok(arguments) = std::env::var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS") {
            for window in &mut context.config_mut().app.windows {
                window.additional_browser_args = Some(arguments.clone());
            }
        }
        context
    };
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let registry = std::env::var_os("GETKEYS_REGISTRY")
                .map(PathBuf::from)
                .unwrap_or_else(|| {
                    if cfg!(dev) {
                        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../registry/dist")
                    } else {
                        app.path()
                            .resource_dir()
                            .expect("resource directory")
                            .join("registry")
                    }
                });
            let data = std::env::var_os("GETKEYS_DATA_DIR")
                .map(PathBuf::from)
                .map(Ok)
                .unwrap_or_else(App::data_dir);
            let core = data
                .and_then(|data| App::open(data, registry))
                .map_err(|e| format!("{e:#}"));
            app.manage(Arc::new(Mutex::new(core)));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            request,
            copy_credential,
            export_env
        ])
        .run(context)
        .expect("Unable to start GetKeys");
}
