#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod desktop;
mod downloads;
mod policy;
mod voice;
mod voice_protocol;
mod workspace;
use std::sync::atomic::Ordering;
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem, Submenu},
    Manager,
};
use tauri_plugin_deep_link::DeepLinkExt;
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons};
use tauri_plugin_updater::UpdaterExt;
use tauri_plugin_window_state::StateFlags;
use url::Url;

#[derive(Clone)]
struct Environment {
    origin: String,
    channel: String,
    scheme: String,
}

fn show_message(app: &tauri::AppHandle, message: &str) {
    app.dialog().message(message).title("Cognuum").show(|_| {});
}

fn load_home(app: &tauri::AppHandle) {
    if let Some(window) = desktop::active(app) {
        let env = app.state::<Environment>();
        let url = window
            .url()
            .ok()
            .filter(|url| policy::same_origin(url, &env.origin))
            .unwrap_or_else(|| Url::parse(&env.origin).expect("validated origin"));
        let _ = window.navigate(url);
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn handle_link(app: &tauri::AppHandle, link: &Url) {
    let env = app.state::<Environment>();
    if let Some(target) = policy::auth_callback(link, &env.scheme, &env.origin) {
        if let Some(window) = app.get_webview_window("main") {
            let _ = window.navigate(target);
            let _ = window.show();
            let _ = window.set_focus();
        }
    }
}

fn check_updates(app: tauri::AppHandle, interactive: bool) {
    if !app.config().plugins.0.contains_key("updater") {
        if !interactive {
            return;
        }
        show_message(&app, "This development build does not receive updates. Install a signed release from Settings → Account → Desktop app.");
        return;
    }
    tauri::async_runtime::spawn(async move {
        let result = async {
            let update = app.updater()?.check().await?;
            if let Some(update) = update {
                let approved = app
                    .dialog()
                    .message(format!(
                        "Cognuum {} is available. Install and restart?",
                        update.version
                    ))
                    .title("Update Cognuum")
                    .buttons(MessageDialogButtons::OkCancel)
                    .blocking_show();
                if approved {
                    update.download_and_install(|_, _| {}, || {}).await?;
                    app.restart();
                }
            } else if interactive {
                show_message(&app, "You’re using the latest desktop version.");
            }
            Ok::<(), tauri_plugin_updater::Error>(())
        }
        .await;
        if result.is_err() && interactive {
            show_message(
                &app,
                "Couldn’t check or install the update. Please try again later.",
            );
        }
    });
}

fn main() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_state_flags(StateFlags::all() - StateFlags::VISIBLE)
                .build(),
        )
        .invoke_handler(tauri::generate_handler![
            desktop::desktop_auth_read,
            desktop::desktop_auth_write,
            desktop::desktop_auth_lock,
            desktop::desktop_auth_unlock,
            desktop::desktop_workspace_ready,
            downloads::desktop_save_pdf,
            voice::desktop_voice_start,
            voice::desktop_voice_control
        ])
        .on_window_event(desktop::window_event)
        .setup(|app| {
            if app.config().plugins.0.contains_key("updater") {
                app.handle()
                    .plugin(tauri_plugin_updater::Builder::new().build())?;
            }
            let config = app
                .config()
                .plugins
                .0
                .get("desktop")
                .ok_or("Missing desktop environment")?;
            let env = Environment {
                origin: config["origin"].as_str().unwrap_or("").into(),
                channel: config["channel"].as_str().unwrap_or("").into(),
                scheme: config["scheme"].as_str().unwrap_or("").into(),
            };
            if !policy::valid_environment(&env.origin, &env.channel, &env.scheme) {
                return Err("Invalid desktop environment".into());
            }
            app.manage(env.clone());
            app.manage(voice::VoiceState::default());
            app.manage(workspace::WorkspaceState::new(desktop::load(
                app.handle(),
                &env.origin,
            )));
            desktop::create(
                app.handle(),
                "main",
                Url::parse(&format!("{}/console", env.origin))?,
                true,
            )?;
            let new_window =
                MenuItem::with_id(app, "new-window", "New Window", true, Some("CmdOrCtrl+N"))?;
            let duplicate = MenuItem::with_id(
                app,
                "duplicate-window",
                "Open This Page in New Window",
                true,
                Some("CmdOrCtrl+Shift+N"),
            )?;
            let save = MenuItem::with_id(
                app,
                "save-workspace",
                "Save Workspace",
                true,
                Some("CmdOrCtrl+Shift+S"),
            )?;
            let show_windows =
                MenuItem::with_id(app, "show-windows", "Show All Windows", true, None::<&str>)?;
            let update =
                MenuItem::with_id(app, "update", "Check for Updates…", true, None::<&str>)?;
            let settings =
                MenuItem::with_id(app, "settings", "Settings…", true, Some("CmdOrCtrl+,"))?;
            let reload =
                MenuItem::with_id(app, "reload", "Reload Cognuum", true, Some("CmdOrCtrl+R"))?;
            let menu = Menu::with_items(
                app,
                &[
                    &Submenu::with_items(
                        app,
                        "Cognuum",
                        true,
                        &[
                            &PredefinedMenuItem::about(app, None, None)?,
                            &settings,
                            &update,
                            &PredefinedMenuItem::quit(app, None)?,
                        ],
                    )?,
                    &Submenu::with_items(
                        app,
                        "Edit",
                        true,
                        &[
                            &PredefinedMenuItem::undo(app, None)?,
                            &PredefinedMenuItem::redo(app, None)?,
                            &PredefinedMenuItem::cut(app, None)?,
                            &PredefinedMenuItem::copy(app, None)?,
                            &PredefinedMenuItem::paste(app, None)?,
                            &PredefinedMenuItem::select_all(app, None)?,
                        ],
                    )?,
                    &Submenu::with_items(
                        app,
                        "View",
                        true,
                        &[&reload, &PredefinedMenuItem::fullscreen(app, None)?],
                    )?,
                    &Submenu::with_items(
                        app,
                        "Window",
                        true,
                        &[
                            &new_window,
                            &duplicate,
                            &save,
                            &show_windows,
                            &PredefinedMenuItem::separator(app)?,
                            &PredefinedMenuItem::minimize(app, None)?,
                            &PredefinedMenuItem::close_window(app, None)?,
                        ],
                    )?,
                ],
            )?;
            app.set_menu(menu)?;
            app.on_menu_event(|app, event| match event.id().as_ref() {
                "reload" => load_home(app),
                "update" => check_updates(app.clone(), true),
                "settings" => {
                    if let Some(window) = desktop::active(app) {
                        let _ = window.navigate(
                            Url::parse(&format!(
                                "{}/console/settings?tab=account&section=desktop",
                                app.state::<Environment>().origin
                            ))
                            .expect("validated origin"),
                        );
                    }
                }
                "new-window" | "duplicate-window" => {
                    let target = if event.id().as_ref() == "duplicate-window" {
                        desktop::active(app).and_then(|w| w.url().ok())
                    } else {
                        None
                    };
                    let app = app.clone();
                    tauri::async_runtime::spawn(async move {
                        if let Err(message) = desktop::open(&app, target) {
                            show_message(&app, &message);
                        }
                    });
                }
                "save-workspace" => desktop::save(app, None),
                "show-windows" => {
                    for window in app.webview_windows().into_values() {
                        let _ = window.show();
                    }
                }
                _ => {}
            });
            check_updates(app.handle().clone(), false);
            let link_app = app.handle().clone();
            app.deep_link().on_open_url(move |event| {
                for url in event.urls() {
                    handle_link(&link_app, &url);
                }
            });
            if let Some(urls) = app.deep_link().get_current()? {
                for url in urls {
                    handle_link(app.handle(), &url);
                }
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("Could not start Cognuum");
    app.run(|app, event| {
        if matches!(event, tauri::RunEvent::ExitRequested { .. }) {
            let state = app.state::<workspace::WorkspaceState>();
            if !state.quitting.swap(true, Ordering::SeqCst) {
                desktop::save(app, None);
            }
        }
        #[cfg(target_os = "macos")]
        if let tauri::RunEvent::Reopen { .. } = event {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }
        #[cfg(not(target_os = "macos"))]
        let _ = (app, event);
    });
}
