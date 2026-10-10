//! Desktop shell: tray icon, the quick-copy palette, `getkeys://` links, and close-to-tray.
use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};
use tauri::{
    AppHandle, Emitter, Manager, Url, WebviewUrl, WebviewWindowBuilder, Window, WindowEvent,
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    webview::PageLoadEvent,
};
use tauri_plugin_deep_link::DeepLinkExt;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

/// Passed by launch at login so GetKeys starts in the tray.
pub const HIDDEN: &str = "--hidden";
const PALETTE: &str = "palette";

/// Events for the main window that arrive before it listens; `None` once it does.
pub struct Pending(Mutex<Option<Vec<(&'static str, String)>>>);

pub fn setup(app: &AppHandle) -> tauri::Result<()> {
    app.manage(Pending(Mutex::new(Some(Vec::new()))));
    tray(app)?;
    // Another app may own the shortcut; the tray still opens the palette.
    let _ = app
        .global_shortcut()
        .on_shortcut("ctrl+shift+k", |app, _, event| {
            if event.state == ShortcutState::Pressed {
                toggle_palette(app);
            }
        });
    if cfg!(dev) {
        // Installers register the scheme; development builds register themselves.
        let _ = app.deep_link().register_all();
    }
    let links = app.clone();
    app.deep_link().on_open_url(move |event| {
        for url in event.urls() {
            open_url(&links, &url);
        }
    });
    if let Ok(Some(urls)) = app.deep_link().get_current() {
        for url in urls {
            open_url(app, &url);
        }
    }
    if !std::env::args().any(|arg| arg == HIDDEN) {
        show_main(app);
    }
    Ok(())
}

/// A second launch focuses this one; its deep link was already forwarded.
pub fn second_instance(app: &AppHandle, args: Vec<String>, _cwd: String) {
    if !args.iter().any(|arg| arg == HIDDEN) {
        show_main(app);
    }
}

/// Closing a window hides it; GetKeys keeps running in the tray until Quit.
pub fn window_event(window: &Window, event: &WindowEvent) {
    if let WindowEvent::CloseRequested { api, .. } = event {
        api.prevent_close();
        let _ = window.hide();
    }
}

/// Called by the main window once it listens, to deliver what arrived earlier.
#[tauri::command]
pub fn shell_ready(app: AppHandle, pending: tauri::State<'_, Pending>) {
    let queued = pending
        .0
        .lock()
        .map(|mut queue| queue.take().unwrap_or_default())
        .unwrap_or_default();
    for (event, payload) in queued {
        let _ = app.emit_to("main", event, payload);
    }
}

fn notify(app: &AppHandle, event: &'static str, payload: String) {
    let pending = app.state::<Pending>();
    let Ok(mut queue) = pending.0.lock() else {
        return;
    };
    match queue.as_mut() {
        Some(queue) => queue.push((event, payload)),
        None => {
            let _ = app.emit_to("main", event, payload);
        }
    }
}

pub fn show_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// `getkeys://open`, `getkeys://key/<id>`, and `getkeys://add/<provider id>`.
fn open_url(app: &AppHandle, url: &Url) {
    if url.scheme() != "getkeys" {
        return;
    }
    show_main(app);
    let target = percent_encoding::percent_decode_str(url.path().trim_start_matches('/'))
        .decode_utf8_lossy()
        .into_owned();
    match url.host_str() {
        Some("key") if !target.is_empty() => notify(app, "open-key", target),
        Some("add") if !target.is_empty() => notify(app, "open-provider", target),
        _ => {}
    }
}

fn tray(app: &AppHandle) -> tauri::Result<()> {
    let menu = Menu::with_items(
        app,
        &[
            &MenuItem::with_id(app, "open", "Open GetKeys", true, None::<&str>)?,
            &MenuItem::with_id(app, PALETTE, "Quick copy…", true, Some("Ctrl+Shift+K"))?,
            &MenuItem::with_id(app, "refresh", "Refresh keys", true, None::<&str>)?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?,
        ],
    )?;
    let mut tray = TrayIconBuilder::with_id("main")
        .tooltip("GetKeys")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => show_main(app),
            PALETTE => toggle_palette(app),
            "refresh" => {
                show_main(app);
                notify(app, "tray-refresh", String::new());
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

/// Shows the palette, or hides it when it is open. It is built on first use and then kept.
fn toggle_palette(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(PALETTE) {
        if window.is_visible().unwrap_or(false) {
            let _ = window.hide();
        } else {
            let _ = window.center();
            let _ = window.show();
            let _ = window.set_focus();
        }
        return;
    }
    let loaded = AtomicBool::new(false);
    let builder =
        WebviewWindowBuilder::new(app, PALETTE, WebviewUrl::App("index.html#palette".into()))
            .title("Quick copy")
            .inner_size(600.0, 420.0)
            .resizable(false)
            .decorations(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .center()
            .visible(false)
            .on_page_load(move |window, payload| {
                if payload.event() == PageLoadEvent::Finished
                    && !loaded.swap(true, Ordering::Relaxed)
                {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            });
    // WebView2 needs every window in a process to share the main window's browser arguments.
    #[cfg(debug_assertions)]
    let builder = match std::env::var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS") {
        Ok(arguments) => builder.additional_browser_args(&arguments),
        Err(_) => builder,
    };
    let _ = builder.build();
}
