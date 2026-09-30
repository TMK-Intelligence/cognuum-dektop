use crate::{policy, workspace::*, Environment};
use std::{
    fs,
    sync::{atomic::Ordering, Arc},
};
use tauri::{Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use tauri_plugin_opener::OpenerExt;
use tauri_plugin_window_state::{AppHandleExt, StateFlags};
use url::Url;

pub(crate) fn verify(window: &WebviewWindow) -> Result<(), String> {
    let env = window.state::<Environment>();
    if !valid_label(window.label())
        || window
            .app_handle()
            .get_webview_window(window.label())
            .is_none()
        || !window
            .url()
            .is_ok_and(|url| policy::same_origin(&url, &env.origin))
    {
        return Err("Desktop request denied".into());
    }
    Ok(())
}

#[tauri::command]
pub fn desktop_auth_read(window: WebviewWindow, key: String) -> Result<Option<String>, String> {
    verify(&window)?;
    let state = window.state::<WorkspaceState>();
    let result = state
        .auth
        .lock()
        .map_err(|_| "Session unavailable")?
        .read(&key);
    result
}

#[tauri::command]
pub fn desktop_auth_write(
    window: WebviewWindow,
    key: String,
    value: Option<String>,
) -> Result<(), String> {
    verify(&window)?;
    let app = window.app_handle();
    let state = app.state::<WorkspaceState>();
    let change = state
        .auth
        .lock()
        .map_err(|_| "Session unavailable")?
        .write(key, value)?;
    if let Some(change) = change {
        let owner = state.auth.lock().expect("session").owner();
        let changed_account = state
            .ready_owner
            .lock()
            .expect("workspace owner")
            .as_ref()
            .is_some_and(|previous| Some(previous) != owner.as_ref());
        if change.event == "SIGNED_OUT" || changed_account {
            crate::voice::stop(app, None);
            *state.ready_owner.lock().expect("workspace owner") = None;
            *state.saved.lock().expect("saved workspace") = SavedWorkspace::default();
            write_saved(app);
            for (label, other) in app.webview_windows() {
                if label != "main" {
                    let _ = other.close();
                }
            }
            if let Some(main) = app.get_webview_window("main") {
                let _ = main.show();
            }
        }
        // The event contains only a revision and event type, never session material.
        for (label, other) in app.webview_windows() {
            if label != window.label() && verify(&other).is_ok() {
                let _ = other.emit(AUTH_EVENT, &change);
            }
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn desktop_auth_lock(window: WebviewWindow, timeout_ms: i64) -> Result<u64, String> {
    verify(&window)?;
    let state = window.state::<WorkspaceState>();
    let generation = state.generation(window.label());
    let guard = tokio::time::timeout(
        std::time::Duration::from_millis(if timeout_ms < 0 {
            30_000
        } else {
            timeout_ms.clamp(1, 30_000) as u64
        }),
        Arc::clone(&state.auth_lock).lock_owned(),
    )
    .await
    .map_err(|_| "Desktop session is busy; please try again")?;
    verify(&window)?;
    let lease = state.next_lease.fetch_add(1, Ordering::SeqCst);
    state.hold_lock(window.label(), generation, lease, guard)?;
    Ok(lease)
}

#[tauri::command]
pub fn desktop_auth_unlock(window: WebviewWindow, lease: u64) -> Result<(), String> {
    verify(&window)?;
    let state = window.state::<WorkspaceState>();
    state.unlock(window.label(), lease);
    Ok(())
}

#[tauri::command]
pub async fn desktop_workspace_ready(window: WebviewWindow) -> Result<(), String> {
    verify(&window)?;
    if window.label() != "main" {
        return Err("Only the primary window restores a workspace".into());
    }
    let app = window.app_handle();
    let state = app.state::<WorkspaceState>();
    let owner = state
        .auth
        .lock()
        .map_err(|_| "Session unavailable")?
        .owner()
        .ok_or("Sign in first")?;
    {
        let mut ready = state
            .ready_owner
            .lock()
            .map_err(|_| "Workspace unavailable")?;
        if ready.as_ref() == Some(&owner) {
            return Ok(());
        }
        *ready = Some(owner.clone());
    }
    let saved = state
        .saved
        .lock()
        .map_err(|_| "Workspace unavailable")?
        .clone();
    if saved.owner == owner {
        restore(app, saved)?;
    } else {
        save(app, None);
    }
    Ok(())
}

pub fn load(app: &tauri::AppHandle, origin: &str) -> SavedWorkspace {
    let result = (|| {
        let path = app.path().app_config_dir().ok()?.join("workspace.json");
        if fs::metadata(&path).ok()?.len() > 32_768 {
            return None;
        }
        serde_json::from_slice::<SavedWorkspace>(&fs::read(path).ok()?).ok()
    })();
    result.unwrap_or_default().validated(origin)
}

fn write_saved(app: &tauri::AppHandle) {
    let state = app.state::<WorkspaceState>();
    let _persistence = state
        .persistence_lock
        .lock()
        .expect("workspace persistence");
    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
        let dir = app.path().app_config_dir()?;
        fs::create_dir_all(&dir)?;
        let data = serde_json::to_vec(
            &*app
                .state::<WorkspaceState>()
                .saved
                .lock()
                .expect("saved workspace"),
        )?;
        let temp = dir.join("workspace.json.tmp");
        fs::write(&temp, data)?;
        fs::rename(temp, dir.join("workspace.json"))?;
        Ok(())
    })();
    if result.is_err() {
        eprintln!("Could not save desktop workspace metadata");
    }
}

pub fn save(app: &tauri::AppHandle, excluding: Option<&str>) {
    let state = app.state::<WorkspaceState>();
    if state.restoring.load(Ordering::SeqCst) {
        return;
    }
    let Some(owner) = state.ready_owner.lock().expect("workspace owner").clone() else {
        return;
    };
    let env = app.state::<Environment>();
    let mut windows: Vec<_> = app
        .webview_windows()
        .into_iter()
        .filter_map(|(label, window)| {
            if excluding == Some(label.as_str()) || !valid_label(&label) {
                return None;
            }
            let path = safe_path(&window.url().ok()?, &env.origin)?;
            Some(SavedWindow { label, path })
        })
        .collect();
    windows.sort_by(|a, b| a.label.cmp(&b.label));
    windows.truncate(MAX_WINDOWS);
    {
        let current_owner = state.ready_owner.lock().expect("workspace owner");
        if current_owner.as_ref() != Some(&owner) {
            return;
        }
        *state.saved.lock().expect("saved workspace") = SavedWorkspace {
            version: 1,
            owner,
            windows,
        };
        write_saved(app);
    }
    let _ = app.save_window_state(StateFlags::all() - StateFlags::VISIBLE);
}

pub fn restore(app: &tauri::AppHandle, saved: SavedWorkspace) -> Result<(), String> {
    let state = app.state::<WorkspaceState>();
    let _windows = state
        .window_operations
        .lock()
        .map_err(|_| "Workspace unavailable")?;
    if state.restoring.swap(true, Ordering::SeqCst) {
        return Ok(());
    }
    let env = app.state::<Environment>();
    let result = (|| {
        let saved = saved.validated(&env.origin);
        for entry in saved.windows {
            if state.ready_owner.lock().expect("workspace owner").as_ref() != Some(&saved.owner) {
                return Err("Workspace account changed".into());
            }
            let url = Url::parse(&env.origin)
                .map_err(|_| "Invalid environment")?
                .join(&entry.path)
                .map_err(|_| "Invalid workspace route")?;
            if let Some(window) = app.get_webview_window(&entry.label) {
                // Authentication is in process memory, so restoring a page does
                // not lose the newly verified session or replay its OAuth code.
                if window.url().ok().as_ref() != Some(&url) {
                    window
                        .navigate(url)
                        .map_err(|_| "Could not restore window")?;
                }
            } else {
                let window = create(app, &entry.label, url, false)
                    .map_err(|_| "Could not restore window")?;
                if state.ready_owner.lock().expect("workspace owner").as_ref() != Some(&saved.owner)
                {
                    let _ = window.close();
                    return Err("Workspace account changed".into());
                }
            }
        }
        Ok(())
    })();
    state.restoring.store(false, Ordering::SeqCst);
    result
}

pub fn active(app: &tauri::AppHandle) -> Option<WebviewWindow> {
    app.webview_windows()
        .into_values()
        .find(|w| w.is_focused().unwrap_or(false))
        .or_else(|| app.get_webview_window("main"))
}

pub fn open(app: &tauri::AppHandle, target: Option<Url>) -> Result<(), String> {
    let state = app.state::<WorkspaceState>();
    let _windows = state
        .window_operations
        .lock()
        .map_err(|_| "Workspace unavailable")?;
    let owner = state.ready_owner.lock().expect("workspace owner").clone();
    if owner.is_none() {
        if let Some(main) = app.get_webview_window("main") {
            let _ = main.show();
            let _ = main.set_focus();
        }
        return Err("Sign in to Cognuum before opening another window.".into());
    }
    let label = (1..MAX_WINDOWS)
        .map(|id| format!("workspace-{id}"))
        .find(|label| app.get_webview_window(label).is_none())
        .ok_or("Up to eight windows can be open at once. Close a window before opening another.")?;
    let env = app.state::<Environment>();
    let origin = Url::parse(&env.origin).map_err(|_| "Invalid environment")?;
    let path = target
        .and_then(|url| safe_path(&url, &env.origin))
        .unwrap_or_else(|| "/console".into());
    let window = create(
        app,
        &label,
        origin.join(&path).map_err(|_| "Invalid route")?,
        true,
    )
    .map_err(|_| "Could not open window")?;
    if *state.ready_owner.lock().expect("workspace owner") != owner {
        let _ = window.close();
        return Err("Workspace account changed".into());
    }
    save(app, None);
    Ok(())
}

pub fn create(
    app: &tauri::AppHandle,
    label: &str,
    url: Url,
    focused: bool,
) -> tauri::Result<WebviewWindow> {
    let env = app.state::<Environment>().inner().clone();
    let metadata = serde_json::json!({ "version":app.package_info().version.to_string(), "channel":env.channel, "scheme":env.scheme,
        "workspace": {"protocol":1,"windowLabel":label,"maxWindows":MAX_WINDOWS},
        "downloads": {"protocol":1}, "voice": {"protocol":1,"language":"en"} });
    let origin_json = serde_json::to_string(&env.origin).expect("origin JSON");
    let script = format!(
        r#"if (location.origin === {origin_json}) {{
        Object.defineProperty(window, '__COGNUUM_DESKTOP__', {{ value: Object.freeze({metadata}), configurable: false }});
        const mark = () => {{ document.documentElement.dataset.tauri = 'true'; document.documentElement.dataset.appVersion = window.__COGNUUM_DESKTOP__.version; }};
        if (document.documentElement) mark(); else document.addEventListener('DOMContentLoaded', mark, {{ once: true }});
    }}"#
    );
    let nav_app = app.clone();
    let nav_origin = env.origin.clone();
    let popup_app = app.clone();
    let popup_origin = env.origin.clone();
    let load_app = app.clone();
    let load_label = label.to_string();
    let load_origin = env.origin.clone();
    let loaded = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let loaded_event = loaded.clone();
    let window = WebviewWindowBuilder::new(app, label, WebviewUrl::External(url))
        .title(if env.channel == "production" {
            "Cognuum"
        } else {
            "Cognuum Staging"
        })
        .inner_size(1440.0, 960.0)
        .min_inner_size(640.0, 480.0)
        .focused(focused)
        .initialization_script(script)
        .on_navigation(move |url| {
            if policy::same_origin(url, &nav_origin)
                || url.scheme() == "tauri"
                || url.origin().ascii_serialization() == "http://tauri.localhost"
            {
                return true;
            }
            if policy::external_url(url) {
                let _ = nav_app.opener().open_url(url.as_str(), None::<&str>);
            }
            false
        })
        .on_new_window(move |url, _| {
            if safe_path(&url, &popup_origin).is_some() {
                let app = popup_app.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(message) = open(&app, Some(url)) {
                        crate::show_message(&app, &message);
                    }
                });
            } else if !policy::same_origin(&url, &popup_origin) && policy::external_url(&url) {
                let _ = popup_app.opener().open_url(url.as_str(), None::<&str>);
            }
            tauri::webview::NewWindowResponse::Deny
        })
        .on_document_title_changed(|window, title| {
            let title: String = title
                .trim()
                .chars()
                .filter(|c| !c.is_control())
                .take(120)
                .collect();
            if !title.is_empty() {
                let _ = window.set_title(&title);
            }
        })
        .on_page_load(move |_, payload| {
            if matches!(payload.event(), tauri::webview::PageLoadEvent::Started) {
                crate::voice::stop(&load_app, Some(&load_label));
                load_app
                    .state::<WorkspaceState>()
                    .invalidate_window(&load_label);
            }
            if policy::same_origin(payload.url(), &load_origin)
                && matches!(payload.event(), tauri::webview::PageLoadEvent::Finished)
            {
                loaded_event.store(true, Ordering::SeqCst);
            }
        })
        .build()?;
    let timeout_window = window.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        if !loaded.load(Ordering::SeqCst) {
            #[cfg(target_os = "windows")]
            let fallback = "http://tauri.localhost/index.html";
            #[cfg(not(target_os = "windows"))]
            let fallback = "tauri://localhost/index.html";
            let _ = timeout_window.navigate(Url::parse(fallback).expect("fallback URL"));
        }
    });
    Ok(window)
}

pub fn window_event(window: &tauri::Window, event: &tauri::WindowEvent) {
    let app = window.app_handle();
    let Some(state) = app.try_state::<WorkspaceState>() else {
        return;
    };
    match event {
        tauri::WindowEvent::CloseRequested { api, .. }
            if !state.quitting.load(Ordering::SeqCst) =>
        {
            crate::voice::stop(app, Some(window.label()));
            if window.label() == "main" {
                save(app, None);
                if cfg!(target_os = "macos") || app.webview_windows().len() > 1 {
                    api.prevent_close();
                    let _ = window.hide();
                } else {
                    state.quitting.store(true, Ordering::SeqCst);
                }
            } else {
                save(app, Some(window.label()));
                if app.webview_windows().len() == 2 {
                    if let Some(main) = app.get_webview_window("main") {
                        let _ = main.show();
                    }
                }
            }
        }
        tauri::WindowEvent::Destroyed => {
            crate::voice::stop(app, Some(window.label()));
            state.invalidate_window(window.label());
            if !state.quitting.load(Ordering::SeqCst) {
                save(app, Some(window.label()));
            }
        }
        tauri::WindowEvent::Focused(false) if !state.quitting.load(Ordering::SeqCst) => {
            crate::voice::stop(app, Some(window.label()));
            save(app, None)
        }
        _ => {}
    }
}
