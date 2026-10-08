#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use getkeys_core::{App, commands::Command};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tauri::Manager;

type Core = Arc<Mutex<Result<App, String>>>;
#[tauri::command]
async fn request(
    state: tauri::State<'_, Core>,
    command: Command,
) -> Result<serde_json::Value, String> {
    let core = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let guard = core
            .lock()
            .map_err(|_| "Application state is unavailable".to_owned())?;
        guard
            .as_ref()
            .map_err(Clone::clone)?
            .dispatch(command)
            .map_err(|e| format!("{e:#}"))
    })
    .await
    .map_err(|_| "Application worker failed".to_owned())?
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
            let secret = guard
                .as_ref()
                .map_err(Clone::clone)?
                .secret(&credential_id)
                .map_err(|e| e.to_string())?;
            secret
                .fields
                .get(&field)
                .cloned()
                .ok_or("Credential field is unavailable")?
        };
        let mut clipboard = arboard::Clipboard::new().map_err(|_| "Clipboard is unavailable")?;
        clipboard
            .set_text(&value)
            .map_err(|_| "Unable to copy credential")?;
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(30));
            if let Ok(mut clipboard) = arboard::Clipboard::new()
                && clipboard.get_text().is_ok_and(|current| current == value)
            {
                let _ = clipboard.clear();
            }
        });
        Ok(())
    })
    .await
    .map_err(|_| "Clipboard worker failed".to_owned())?
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
        .invoke_handler(tauri::generate_handler![request, copy_credential])
        .run(context)
        .expect("Unable to start GetKeys");
}
