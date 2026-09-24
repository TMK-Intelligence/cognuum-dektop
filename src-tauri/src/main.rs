#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod policy;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem, Submenu},
    Manager, WebviewUrl, WebviewWindowBuilder,
};
use tauri_plugin_deep_link::DeepLinkExt;
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons};
use tauri_plugin_opener::OpenerExt;
use tauri_plugin_updater::UpdaterExt;
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
    if let Some(window) = app.get_webview_window("main") {
        let env = app.state::<Environment>();
        let _ = window.navigate(Url::parse(&env.origin).expect("validated origin"));
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
            if let Some(window) = app.get_webview_window("main") { let _ = window.show(); let _ = window.set_focus(); }
        }))
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .on_window_event(|window, event| {
            #[cfg(target_os = "macos")]
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
            #[cfg(not(target_os = "macos"))]
            let _ = (window, event);
        })
        .setup(|app| {
            if app.config().plugins.0.contains_key("updater") {
                app.handle().plugin(tauri_plugin_updater::Builder::new().build())?;
            }
            let config = app.config().plugins.0.get("desktop").ok_or("Missing desktop environment")?;
            let env = Environment {
                origin: config["origin"].as_str().unwrap_or("").into(),
                channel: config["channel"].as_str().unwrap_or("").into(),
                scheme: config["scheme"].as_str().unwrap_or("").into(),
            };
            if !policy::valid_environment(&env.origin, &env.channel, &env.scheme) { return Err("Invalid desktop environment".into()); }
            app.manage(env.clone());
            let version = app.package_info().version.to_string();
            let metadata = serde_json::json!({ "version": version, "channel": env.channel, "scheme": env.scheme });
            let origin_json = serde_json::to_string(&env.origin)?;
            let script = format!(r#"if (location.origin === {origin_json}) {{
                Object.defineProperty(window, '__COGNUUM_DESKTOP__', {{ value: Object.freeze({metadata}), configurable: false }});
                const mark = () => {{ document.documentElement.dataset.tauri = 'true'; document.documentElement.dataset.appVersion = window.__COGNUUM_DESKTOP__.version; }};
                if (document.documentElement) mark(); else document.addEventListener('DOMContentLoaded', mark, {{ once: true }});
            }}"#);
            let nav_app = app.handle().clone();
            let nav_origin = env.origin.clone();
            let popup_app = app.handle().clone();
            let popup_origin = env.origin.clone();
            let loaded = Arc::new(AtomicBool::new(false));
            let loaded_event = loaded.clone();
            let load_origin = env.origin.clone();
            let window = WebviewWindowBuilder::new(app, "main", WebviewUrl::External(Url::parse(&env.origin)?))
                .title(if env.channel == "production" { "Cognuum" } else { "Cognuum Staging" })
                .inner_size(1440.0, 960.0).min_inner_size(960.0, 640.0)
                .initialization_script(script)
                .on_navigation(move |url| {
                    if policy::same_origin(url, &nav_origin) || url.scheme() == "tauri" || url.origin().ascii_serialization() == "http://tauri.localhost" { return true; }
                    if policy::external_url(url) { let _ = nav_app.opener().open_url(url.as_str(), None::<&str>); }
                    false
                })
                .on_new_window(move |url, _| {
                    if policy::same_origin(&url, &popup_origin) {
                        if let Some(window) = popup_app.get_webview_window("main") { let _ = window.navigate(url); }
                    } else if policy::external_url(&url) { let _ = popup_app.opener().open_url(url.as_str(), None::<&str>); }
                    tauri::webview::NewWindowResponse::Deny
                })
                .on_page_load(move |_, payload| {
                    if policy::same_origin(payload.url(), &load_origin) && matches!(payload.event(), tauri::webview::PageLoadEvent::Finished) { loaded_event.store(true, Ordering::SeqCst); }
                })
                .build()?;
            let timeout_window = window.clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_secs(30));
                if !loaded.load(Ordering::SeqCst) {
                    #[cfg(target_os = "windows")]
                    let fallback = "http://tauri.localhost/index.html";
                    #[cfg(not(target_os = "windows"))]
                    let fallback = "tauri://localhost/index.html";
                    let _ = timeout_window.navigate(Url::parse(fallback).expect("static fallback"));
                }
            });
            let update = MenuItem::with_id(app, "update", "Check for Updates…", true, None::<&str>)?;
            let settings = MenuItem::with_id(app, "settings", "Settings…", true, Some("CmdOrCtrl+,"))?;
            let reload = MenuItem::with_id(app, "reload", "Reload Cognuum", true, Some("CmdOrCtrl+R"))?;
            let menu = Menu::with_items(app, &[
                &Submenu::with_items(app, "Cognuum", true, &[&PredefinedMenuItem::about(app, None, None)?, &settings, &update, &PredefinedMenuItem::quit(app, None)?])?,
                &Submenu::with_items(app, "Edit", true, &[&PredefinedMenuItem::undo(app, None)?, &PredefinedMenuItem::redo(app, None)?, &PredefinedMenuItem::cut(app, None)?, &PredefinedMenuItem::copy(app, None)?, &PredefinedMenuItem::paste(app, None)?, &PredefinedMenuItem::select_all(app, None)?])?,
                &Submenu::with_items(app, "View", true, &[&reload, &PredefinedMenuItem::fullscreen(app, None)?])?,
                &Submenu::with_items(app, "Window", true, &[&PredefinedMenuItem::minimize(app, None)?, &PredefinedMenuItem::close_window(app, None)?])?,
            ])?;
            app.set_menu(menu)?;
            app.on_menu_event(|app, event| match event.id().as_ref() {
                "reload" => load_home(app),
                "update" => check_updates(app.clone(), true),
                "settings" => { if let Some(window) = app.get_webview_window("main") { let _ = window.navigate(Url::parse(&format!("{}/console/settings?tab=account&section=desktop", app.state::<Environment>().origin)).expect("validated origin")); } },
                _ => {},
            });
            check_updates(app.handle().clone(), false);
            let link_app = app.handle().clone();
            app.deep_link().on_open_url(move |event| { for url in event.urls() { handle_link(&link_app, &url); } });
            if let Some(urls) = app.deep_link().get_current()? { for url in urls { handle_link(app.handle(), &url); } }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("Could not start Cognuum");
    app.run(|app, event| {
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
