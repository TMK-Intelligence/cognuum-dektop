fn main() {
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "desktop_auth_read",
            "desktop_auth_write",
            "desktop_auth_lock",
            "desktop_auth_unlock",
            "desktop_workspace_ready",
            "desktop_save_pdf",
            "desktop_voice_start",
            "desktop_voice_control",
        ]),
    ))
    .expect("Could not build the desktop permission manifest");
}
