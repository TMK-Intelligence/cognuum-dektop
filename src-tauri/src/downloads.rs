use base64::{engine::general_purpose::STANDARD, Engine};
use std::{
    fs::{self, OpenOptions},
    io::{ErrorKind, Write},
    path::Path,
};
use tauri::{Manager, WebviewWindow};

const MAX_PDF_BYTES: usize = 64 * 1024 * 1024;
const SAVE_ERROR: &str =
    "Could not save the PDF to Downloads. Check folder permissions and available space.";

fn allowed_source(label: &str, url: &url::Url, origin: &str) -> bool {
    crate::workspace::valid_label(label)
        && url.scheme() == "https"
        && crate::policy::same_origin(url, origin)
}

fn decode_pdf(data: &str) -> Result<Vec<u8>, String> {
    if data.len() > MAX_PDF_BYTES.div_ceil(3) * 4 {
        return Err("Document exceeds 64 MB".into());
    }
    STANDARD
        .decode(data)
        .map_err(|_| "Invalid PDF encoding".into())
}

fn validate_pdf(filename: &str, bytes: &[u8]) -> Result<(), String> {
    let stem = filename
        .strip_suffix(".pdf")
        .ok_or("Invalid PDF filename")?;
    // A basename only, portable to Windows: no traversal, ADS, device names,
    // hidden files or platform-specific path separators.
    let device = stem.split('.').next().unwrap_or("").to_ascii_uppercase();
    if stem.is_empty()
        || filename.len() > 180
        || stem.starts_with('.')
        || !stem
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
        || matches!(device.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (device.len() == 4
            && (device.starts_with("COM") || device.starts_with("LPT"))
            && matches!(device.as_bytes()[3], b'1'..=b'9'))
    {
        return Err("Invalid PDF filename".into());
    }
    if bytes.len() > MAX_PDF_BYTES || !bytes.starts_with(b"%PDF-") {
        return Err("Invalid PDF or document exceeds 64 MB".into());
    }
    Ok(())
}

fn write_pdf(directory: &Path, filename: &str, bytes: &[u8]) -> Result<String, String> {
    validate_pdf(filename, bytes)?;
    let stem = filename.strip_suffix(".pdf").expect("validated filename");
    // create_new is atomic: another window cannot overwrite this export, and
    // pre-existing files (including symlinks) are never followed or replaced.
    for index in 0..1000 {
        let name = if index == 0 {
            filename.to_string()
        } else {
            format!("{stem}-{index}.pdf")
        };
        let path = directory.join(&name);
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = match options.open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
            Err(_) => return Err(SAVE_ERROR.into()),
        };
        if file.write_all(bytes).and_then(|_| file.sync_all()).is_err() {
            drop(file);
            let _ = fs::remove_file(&path);
            return Err(SAVE_ERROR.into());
        }
        return Ok(name);
    }
    Err("Too many copies of this PDF already exist in Downloads.".into())
}

/// No remote filesystem API: only a bounded PDF, only in the OS Downloads folder.
/// No URL fetches, user-supplied directories, tokens, automatic opens or retries.
#[tauri::command]
pub async fn desktop_save_pdf(
    window: WebviewWindow,
    filename: String,
    data_base64: String,
) -> Result<String, String> {
    let origin = &window.state::<crate::Environment>().origin;
    if !window
        .url()
        .is_ok_and(|url| allowed_source(window.label(), &url, origin))
    {
        return Err("Desktop request denied".into());
    }
    let bytes = decode_pdf(&data_base64)?;
    drop(data_base64);
    validate_pdf(&filename, &bytes)?;
    let directory = window.path().download_dir().map_err(|_| SAVE_ERROR)?;
    tauri::async_runtime::spawn_blocking(move || write_pdf(&directory, &filename, &bytes))
        .await
        .map_err(|_| SAVE_ERROR.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    const PDF: &[u8] = b"%PDF-1.7\nexport test\n%%EOF";

    #[test]
    fn json_transport_preserves_pdf_bytes_without_weakening_hosted_csp() {
        assert_eq!(decode_pdf(&STANDARD.encode(PDF)).unwrap(), PDF);
        assert!(decode_pdf("%not-base64%").is_err());
        assert!(decode_pdf(&"A".repeat(MAX_PDF_BYTES.div_ceil(3) * 4 + 1)).is_err());
    }

    struct Directory(std::path::PathBuf);
    impl Directory {
        fn new() -> Self {
            let nonce = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir()
                .join(format!("cognuum-pdf-test-{}-{nonce}", std::process::id()));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn only_the_channel_page_in_a_workspace_window_can_save() {
        let origin = "https://access.cognuum.com";
        let page = url::Url::parse(&format!("{origin}/console")).unwrap();
        assert!(allowed_source("main", &page, origin));
        assert!(allowed_source("workspace-7", &page, origin));
        for label in ["popup", "workspace-8", "workspace-0"] {
            assert!(!allowed_source(label, &page, origin));
        }
        for address in [
            "https://staging-access.cognuum.com/console",
            "https://access.cognuum.com.evil.test/",
            "https://user@access.cognuum.com/",
            "blob:https://access.cognuum.com/export",
            "tauri://localhost/index.html",
            "file:///tmp/page.html",
        ] {
            assert!(!allowed_source(
                "main",
                &url::Url::parse(address).unwrap(),
                origin
            ));
        }
    }

    #[test]
    fn rejects_non_pdf_payloads_and_unsafe_filenames() {
        assert!(validate_pdf("AI-Notes_2026.pdf", PDF).is_ok());
        assert!(validate_pdf("-Market-Update-AI-Notes.pdf", PDF).is_ok());
        for name in [
            "../notes.pdf",
            "C:\\notes.pdf",
            "folder/notes.pdf",
            "notes.pdf:stream",
            ".hidden.pdf",
            "CON.pdf",
            "aux.txt.pdf",
            "LPT9.pdf",
            "notes.exe",
            "notes\n.pdf",
            ".pdf",
            "notes ".repeat(40).as_str(),
        ] {
            assert!(validate_pdf(name, PDF).is_err(), "{name}");
        }
        assert!(validate_pdf("notes.pdf", b"not a PDF").is_err());
        let mut oversized = vec![0; MAX_PDF_BYTES + 1];
        oversized[..5].copy_from_slice(b"%PDF-");
        assert!(validate_pdf("notes.pdf", &oversized).is_err());
    }

    #[test]
    fn saves_exact_bytes_and_never_overwrites_an_existing_export() {
        let directory = Directory::new();
        fs::write(directory.0.join("notes.pdf"), b"existing document").unwrap();
        assert_eq!(
            write_pdf(&directory.0, "notes.pdf", PDF).unwrap(),
            "notes-1.pdf"
        );
        assert_eq!(
            fs::read(directory.0.join("notes.pdf")).unwrap(),
            b"existing document"
        );
        assert_eq!(fs::read(directory.0.join("notes-1.pdf")).unwrap(), PDF);
        assert_eq!(
            write_pdf(&directory.0, "notes.pdf", PDF).unwrap(),
            "notes-2.pdf"
        );
    }

    #[test]
    fn failed_or_invalid_writes_do_not_claim_success_or_create_directories() {
        let directory = Directory::new();
        assert!(write_pdf(&directory.0.join("missing"), "notes.pdf", PDF).is_err());
        assert!(!directory.0.join("missing").exists());
        assert!(write_pdf(&directory.0, "../notes.pdf", PDF).is_err());
        assert!(write_pdf(&directory.0, "notes.pdf", b"bad payload").is_err());
        assert_eq!(fs::read_dir(&directory.0).unwrap().count(), 0);
    }

    #[test]
    fn concurrent_windows_get_distinct_files() {
        let directory = Directory::new();
        let writers: Vec<_> = (0..8)
            .map(|_| {
                let path = directory.0.clone();
                std::thread::spawn(move || write_pdf(&path, "notes.pdf", PDF).unwrap())
            })
            .collect();
        let names: std::collections::HashSet<_> =
            writers.into_iter().map(|t| t.join().unwrap()).collect();
        assert_eq!(names.len(), 8);
        for name in names {
            assert_eq!(fs::read(directory.0.join(name)).unwrap(), PDF);
        }
    }
}
