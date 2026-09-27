//! Workspace metadata is durable; authentication is deliberately process-memory only.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};
use url::Url;

pub const MAX_WINDOWS: usize = 8;
pub const AUTH_EVENT: &str = "cognuum-desktop-auth";
const MAX_SESSION_BYTES: usize = 64 * 1024;

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct SavedWindow {
    pub label: String,
    pub path: String,
}

#[derive(Clone, Serialize, Deserialize, Default)]
pub struct SavedWorkspace {
    pub version: u8,
    pub owner: String,
    pub windows: Vec<SavedWindow>,
}

pub fn valid_label(label: &str) -> bool {
    label == "main"
        || label
            .strip_prefix("workspace-")
            .and_then(|id| id.parse::<usize>().ok())
            .is_some_and(|id| id > 0 && id < MAX_WINDOWS && label == format!("workspace-{id}"))
}

/// Persist only product routes and a bounded set of non-authentication query fields.
/// Login callbacks, URL fragments and arbitrary query strings never reach disk.
pub fn safe_path(url: &Url, origin: &str) -> Option<String> {
    if !crate::policy::same_origin(url, origin) || url.as_str().len() > 4096 {
        return None;
    }
    let path = url.path();
    if ![
        "/console",
        "/charting",
        "/analysis",
        "/article",
        "/articles",
        "/matrix",
    ]
    .iter()
    .any(|prefix| path == *prefix || path.starts_with(&format!("{prefix}/")))
    {
        return None;
    }
    let mut clean = Url::parse(origin).ok()?;
    clean.set_path(path);
    for (key, value) in url.query_pairs() {
        if [
            "tab",
            "section",
            "mode",
            "view",
            "id",
            "series",
            "seriesId",
            "series_id",
            "chart",
            "chartId",
            "dashboard",
            "dashboardId",
            "portfolio",
            "portfolioId",
            "symbol",
            "period",
            "range",
        ]
        .contains(&key.as_ref())
            && value.len() <= 256
        {
            clean.query_pairs_mut().append_pair(&key, &value);
        }
    }
    Some(format!(
        "{}{}",
        clean.path(),
        clean.query().map(|q| format!("?{q}")).unwrap_or_default()
    ))
}

impl SavedWorkspace {
    pub fn validated(self, origin: &str) -> Self {
        if self.version != 1 || self.owner.is_empty() || self.owner.len() > 128 {
            return Self::default();
        }
        let mut seen = BTreeSet::new();
        let windows = self
            .windows
            .into_iter()
            .take(MAX_WINDOWS)
            .filter_map(|entry| {
                if !valid_label(&entry.label) || !seen.insert(entry.label.clone()) {
                    return None;
                }
                let url = Url::parse(origin).ok()?.join(&entry.path).ok()?;
                Some(SavedWindow {
                    label: entry.label,
                    path: safe_path(&url, origin)?,
                })
            })
            .collect();
        Self {
            version: 1,
            owner: self.owner,
            windows,
        }
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthChange {
    pub revision: u64,
    pub event: &'static str,
}

#[derive(Default)]
pub struct AuthMemory {
    values: BTreeMap<String, String>,
    pub revision: u64,
}

impl AuthMemory {
    fn valid_key(key: &str) -> bool {
        key == "sb-cognuum-desktop-auth-token"
    }
    pub fn read(&self, key: &str) -> Result<Option<String>, String> {
        if !Self::valid_key(key) {
            return Err("Unsupported session key".into());
        }
        Ok(self.values.get(key).cloned())
    }
    pub fn owner(&self) -> Option<String> {
        self.values.values().find_map(|value| {
            serde_json::from_str::<serde_json::Value>(value)
                .ok()?
                .get("user")?
                .get("id")?
                .as_str()
                .map(str::to_owned)
        })
    }
    pub fn write(
        &mut self,
        key: String,
        value: Option<String>,
    ) -> Result<Option<AuthChange>, String> {
        if !Self::valid_key(&key) {
            return Err("Unsupported session key".into());
        }
        if let Some(raw) = &value {
            if raw.len() > MAX_SESSION_BYTES {
                return Err("Session exceeds size limit".into());
            }
            let parsed: serde_json::Value =
                serde_json::from_str(raw).map_err(|_| "Invalid session")?;
            if !["access_token", "refresh_token"]
                .iter()
                .all(|key| parsed[key].as_str().is_some_and(|v| !v.is_empty()))
                || !parsed["user"]["id"]
                    .as_str()
                    .is_some_and(|v| !v.is_empty() && v.len() <= 128)
            {
                return Err("Invalid session".into());
            }
        }
        if self.values.get(&key) == value.as_ref() {
            return Ok(None);
        }
        let previous_owner = self.owner();
        if let Some(raw) = value {
            self.values.insert(key, raw);
        } else {
            self.values.remove(&key);
        }
        self.revision += 1;
        let event = match self.owner() {
            None => "SIGNED_OUT",
            Some(owner) if Some(owner.as_str()) != previous_owner.as_deref() => "SIGNED_IN",
            _ => "TOKEN_REFRESHED",
        };
        Ok(Some(AuthChange {
            revision: self.revision,
            event,
        }))
    }
}

pub struct LockHolder {
    pub window: String,
    pub lease: u64,
    pub _guard: OwnedMutexGuard<()>,
}

pub struct WorkspaceState {
    pub auth: Mutex<AuthMemory>,
    pub auth_lock: Arc<AsyncMutex<()>>,
    pub lock_holder: Mutex<Option<LockHolder>>,
    pub generations: Mutex<BTreeMap<String, u64>>,
    pub window_operations: Mutex<()>,
    pub next_lease: std::sync::atomic::AtomicU64,
    pub saved: Mutex<SavedWorkspace>,
    pub persistence_lock: Mutex<()>,
    pub ready_owner: Mutex<Option<String>>,
    pub restoring: std::sync::atomic::AtomicBool,
    pub quitting: std::sync::atomic::AtomicBool,
}

impl WorkspaceState {
    pub fn new(saved: SavedWorkspace) -> Self {
        Self {
            auth: Mutex::new(AuthMemory::default()),
            auth_lock: Arc::new(AsyncMutex::new(())),
            lock_holder: Mutex::new(None),
            generations: Mutex::new(BTreeMap::new()),
            window_operations: Mutex::new(()),
            next_lease: std::sync::atomic::AtomicU64::new(1),
            saved: Mutex::new(saved),
            persistence_lock: Mutex::new(()),
            ready_owner: Mutex::new(None),
            restoring: std::sync::atomic::AtomicBool::new(false),
            quitting: std::sync::atomic::AtomicBool::new(false),
        }
    }
    pub fn release_window_lock(&self, label: &str) {
        let mut holder = self.lock_holder.lock().expect("auth lock state");
        if holder.as_ref().is_some_and(|h| h.window == label) {
            holder.take();
        }
    }
    pub fn generation(&self, label: &str) -> u64 {
        *self
            .generations
            .lock()
            .expect("window generations")
            .get(label)
            .unwrap_or(&0)
    }
    pub fn invalidate_window(&self, label: &str) {
        let mut generations = self.generations.lock().expect("window generations");
        *generations.entry(label.into()).or_default() += 1;
        self.release_window_lock(label);
    }
    pub fn unlock(&self, label: &str, lease: u64) {
        let mut holder = self.lock_holder.lock().expect("auth lock state");
        if holder
            .as_ref()
            .is_some_and(|h| h.window == label && h.lease == lease)
        {
            holder.take();
        }
    }
    pub fn hold_lock(
        &self,
        label: &str,
        generation: u64,
        lease: u64,
        guard: OwnedMutexGuard<()>,
    ) -> Result<(), String> {
        // Keep the generation check and holder installation atomic with unload.
        let generations = self.generations.lock().map_err(|_| "Session unavailable")?;
        if *generations.get(label).unwrap_or(&0) != generation {
            return Err("Window changed during authentication".into());
        }
        *self.lock_holder.lock().map_err(|_| "Session unavailable")? = Some(LockHolder {
            window: label.into(),
            lease,
            _guard: guard,
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const ORIGIN: &str = "https://access.cognuum.com";
    #[test]
    fn auth_leases_serialize_windows_and_abandoned_documents_cannot_hold_them() {
        let state = WorkspaceState::new(SavedWorkspace::default());
        let generation = state.generation("main");
        let guard = Arc::clone(&state.auth_lock).try_lock_owned().unwrap();
        state.hold_lock("main", generation, 1, guard).unwrap();
        assert!(Arc::clone(&state.auth_lock).try_lock_owned().is_err());
        state.unlock("workspace-1", 1);
        state.unlock("main", 999);
        assert!(Arc::clone(&state.auth_lock).try_lock_owned().is_err());
        state.invalidate_window("main");
        let guard = Arc::clone(&state.auth_lock).try_lock_owned().unwrap();
        assert!(state.hold_lock("main", generation, 2, guard).is_err());
        let guard = Arc::clone(&state.auth_lock).try_lock_owned().unwrap();
        state
            .hold_lock("workspace-1", state.generation("workspace-1"), 3, guard)
            .unwrap();
        state.unlock("workspace-1", 3);
        assert!(Arc::clone(&state.auth_lock).try_lock_owned().is_ok());
    }
    #[test]
    fn saved_routes_strip_credentials_and_reject_external_destinations() {
        assert_eq!(safe_path(&Url::parse(&format!("{ORIGIN}/console/settings?tab=account&access_token=secret&desktop_code=secret#secret")).unwrap(), ORIGIN), Some("/console/settings?tab=account".into()));
        for raw in [
            "https://access.cognuum.com.evil.test/console",
            "https://user@access.cognuum.com/console",
            "https://access.cognuum.com/login?desktop_code=secret",
            "file:///console",
        ] {
            assert!(safe_path(&Url::parse(raw).unwrap(), ORIGIN).is_none());
        }
    }
    #[test]
    fn corrupted_workspaces_are_bounded_deduplicated_and_sanitized() {
        let saved = SavedWorkspace {
            version: 1,
            owner: "member".into(),
            windows: vec![
                SavedWindow {
                    label: "main".into(),
                    path: "/console?token=secret".into(),
                },
                SavedWindow {
                    label: "main".into(),
                    path: "/console".into(),
                },
                SavedWindow {
                    label: "workspace-1".into(),
                    path: "//evil.test/console".into(),
                },
                SavedWindow {
                    label: "workspace-999".into(),
                    path: "/console".into(),
                },
            ],
        }
        .validated(ORIGIN);
        assert_eq!(
            saved.windows,
            vec![SavedWindow {
                label: "main".into(),
                path: "/console".into()
            }]
        );
        assert!(SavedWorkspace {
            version: 2,
            ..saved
        }
        .validated(ORIGIN)
        .windows
        .is_empty());
    }
    #[test]
    fn authentication_is_memory_only_and_revisions_do_not_echo_identical_writes() {
        let mut memory = AuthMemory::default();
        let key = "sb-cognuum-desktop-auth-token".to_string();
        let session =
            r#"{"access_token":"access","refresh_token":"refresh","user":{"id":"member"}}"#;
        assert_eq!(
            memory
                .write(key.clone(), Some(session.into()))
                .unwrap()
                .unwrap()
                .event,
            "SIGNED_IN"
        );
        assert!(memory
            .write(key.clone(), Some(session.into()))
            .unwrap()
            .is_none());
        assert_eq!(memory.read(&key).unwrap().as_deref(), Some(session));
        assert_eq!(
            memory.write(key.clone(), None).unwrap().unwrap().event,
            "SIGNED_OUT"
        );
        assert!(memory.read(&key).unwrap().is_none());
        assert!(memory
            .write("arbitrary-file".into(), Some(session.into()))
            .is_err());
        assert!(memory.write(key, Some("{}".into())).is_err());
        assert!(!serde_json::to_string(&SavedWorkspace::default())
            .unwrap()
            .contains("token"));
    }
}
