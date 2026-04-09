use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::time::Duration;
use tracing::{error, warn};
use uuid::Uuid;

use crate::api::{AnnotationGroup, SCHEMA_VERSION};
use crate::verdict::{Verdict, parse_verdict};

fn default_schema_version() -> u32 {
    SCHEMA_VERSION
}

fn default_version() -> u32 {
    1
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SessionStatus {
    Active,
    Submitted,
    Cancelled,
    Expired,
}

pub enum SubmitResult {
    Ok,
    NotFound,
    NotActive,
}

impl SessionStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            SessionStatus::Active => "Active",
            SessionStatus::Submitted => "Submitted",
            SessionStatus::Cancelled => "Cancelled",
            SessionStatus::Expired => "Expired",
        }
    }
}

/// Typed metadata about where a session originated from.
/// Populated by the client (CLI, agent, etc.) at creation time.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SessionOrigin {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_remote: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    pub id: String,
    #[serde(default)]
    pub title: Option<String>,
    pub status: SessionStatus,
    #[serde(default = "default_version")]
    pub version: u32,
    pub content: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default)]
    pub typed_notes: Option<String>,
    #[serde(default)]
    pub verdict: Option<Verdict>,
    #[serde(default)]
    pub annotation_images: Vec<String>,
    #[serde(default)]
    pub stroke_data: Option<serde_json::Value>,
    #[serde(default)]
    pub annotations: Vec<AnnotationGroup>,
    #[serde(default)]
    pub callback_url: Option<String>,
    #[serde(default)]
    pub tags: HashMap<String, String>,
    /// Starred sessions are pinned — they survive purge, expire, and are
    /// always reloaded from disk regardless of terminal status.
    #[serde(default)]
    pub starred: bool,
    #[serde(default)]
    pub origin: Option<SessionOrigin>,
    pub state_dir: PathBuf,
}

impl Session {
    pub fn save_annotation(&self, data: &[u8]) -> String {
        let dir = self.state_dir.join("annotations");
        if let Err(e) = fs::create_dir_all(&dir) {
            error!(path = %dir.display(), error = %e, "failed to create annotations dir");
        }
        let filename = format!("img_{}.png", Uuid::new_v4().as_simple());
        let path = dir.join(&filename);
        if let Err(e) = fs::write(&path, data) {
            error!(path = %path.display(), error = %e, "failed to write annotation");
        }
        path.to_string_lossy().to_string()
    }

    fn persist(&self) {
        if let Err(e) = fs::create_dir_all(&self.state_dir) {
            error!(path = %self.state_dir.display(), error = %e, "failed to create session dir");
            return;
        }
        let path = self.state_dir.join("session.json");
        match serde_json::to_string_pretty(self) {
            Ok(json) => {
                if let Err(e) = fs::write(&path, json) {
                    error!(path = %path.display(), error = %e, "failed to persist session");
                }
            }
            Err(e) => error!(session = %self.id, error = %e, "failed to serialize session"),
        }
    }
}

pub struct SessionManager {
    sessions: HashMap<String, Session>,
    state_dir: PathBuf,
}

impl SessionManager {
    pub fn new(state_dir: PathBuf) -> Self {
        if let Err(e) = fs::create_dir_all(&state_dir) {
            error!(path = %state_dir.display(), error = %e, "failed to create state dir");
        }
        let mut mgr = Self {
            sessions: HashMap::new(),
            state_dir,
        };
        mgr.load_from_disk();
        mgr
    }

    fn load_from_disk(&mut self) {
        let sessions_dir = self.state_dir.join("sessions");
        let entries = match fs::read_dir(&sessions_dir) {
            Ok(e) => e,
            Err(_) => return,
        };
        let mut loaded = 0usize;
        let mut loaded_starred = 0usize;
        for entry in entries.flatten() {
            let json_path = entry.path().join("session.json");
            if json_path.exists()
                && let Ok(content) = fs::read_to_string(&json_path)
                && let Ok(session) = serde_json::from_str::<Session>(&content)
            {
                let is_live = matches!(
                    session.status,
                    SessionStatus::Active | SessionStatus::Submitted
                );
                if is_live || session.starred {
                    if session.starred {
                        loaded_starred += 1;
                    }
                    self.sessions.insert(session.id.clone(), session);
                    loaded += 1;
                }
            }
        }
        if loaded > 0 {
            tracing::info!(
                count = loaded,
                starred = loaded_starred,
                "restored sessions from disk"
            );
        }
    }

    pub fn create(
        &mut self,
        content: String,
        title: Option<String>,
        callback_url: Option<String>,
        tags: HashMap<String, String>,
        starred: bool,
        origin: Option<SessionOrigin>,
    ) -> Session {
        let id = loop {
            let candidate = Uuid::new_v4().as_simple().to_string()[..12].to_string();
            if !self.sessions.contains_key(&candidate) {
                break candidate;
            }
            warn!(id = %candidate, "session ID collision, retrying");
        };
        let session_dir = self.state_dir.join("sessions").join(&id);

        let session = Session {
            schema_version: SCHEMA_VERSION,
            id: id.clone(),
            title,
            status: SessionStatus::Active,
            version: 1,
            content,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            typed_notes: None,
            verdict: None,
            annotation_images: Vec::new(),
            stroke_data: None,
            annotations: Vec::new(),
            callback_url,
            tags,
            starred,
            origin,
            state_dir: session_dir,
        };
        session.persist();
        self.sessions.insert(id, session.clone());
        session
    }

    /// Toggle the starred flag on a session. Returns true if the session exists.
    pub fn set_starred(&mut self, id: &str, starred: bool) -> bool {
        if let Some(s) = self.sessions.get_mut(id) {
            if s.starred != starred {
                s.starred = starred;
                s.updated_at = Utc::now();
                s.persist();
            }
            true
        } else {
            false
        }
    }

    pub fn get(&self, id: &str) -> Option<&Session> {
        self.sessions.get(id)
    }

    pub fn list(&self) -> Vec<&Session> {
        self.sessions.values().collect()
    }

    /// Cancel an Active session. Returns true only if the session existed and was Active.
    pub fn cancel(&mut self, id: &str) -> bool {
        match self.sessions.get_mut(id) {
            Some(s) if s.status == SessionStatus::Active => {
                s.status = SessionStatus::Cancelled;
                s.updated_at = Utc::now();
                s.persist();
                true
            }
            _ => false,
        }
    }

    pub fn submit(
        &mut self,
        id: &str,
        typed_notes: String,
        images: Vec<String>,
        verdict_override: Option<Verdict>,
        stroke_data: Option<serde_json::Value>,
        annotations: Vec<AnnotationGroup>,
    ) -> SubmitResult {
        match self.sessions.get_mut(id) {
            None => SubmitResult::NotFound,
            Some(s) if s.status != SessionStatus::Active => SubmitResult::NotActive,
            Some(s) => {
                s.verdict = verdict_override.or_else(|| parse_verdict(&typed_notes));
                s.typed_notes = Some(typed_notes);
                s.annotation_images = images;
                s.stroke_data = stroke_data;
                s.annotations = annotations;
                s.status = SessionStatus::Submitted;
                s.updated_at = Utc::now();
                s.persist();
                SubmitResult::Ok
            }
        }
    }

    /// Update document content on an Active session, incrementing version.
    /// Returns the new version on success, None if not Active or not found.
    pub fn update_content(&mut self, id: &str, content: String) -> Option<u32> {
        let s = self.sessions.get_mut(id)?;
        if s.status != SessionStatus::Active {
            return None;
        }
        s.content = content;
        s.version += 1;
        s.updated_at = Utc::now();
        s.persist();
        Some(s.version)
    }

    pub fn count(&self) -> usize {
        self.sessions.len()
    }

    pub fn starred_count(&self) -> usize {
        self.sessions.values().filter(|s| s.starred).count()
    }

    pub fn delete(&mut self, id: &str) -> bool {
        if let Some(s) = self.sessions.remove(id) {
            let _ = fs::remove_dir_all(&s.state_dir);
            true
        } else {
            false
        }
    }

    /// Remove all terminal-state sessions (Submitted, Cancelled, Expired) from memory and disk.
    /// Starred sessions are never purged. Returns the number of sessions purged.
    pub fn purge_finished(&mut self) -> usize {
        let finished: Vec<String> = self
            .sessions
            .values()
            .filter(|s| {
                !s.starred
                    && matches!(
                        s.status,
                        SessionStatus::Submitted
                            | SessionStatus::Cancelled
                            | SessionStatus::Expired
                    )
            })
            .map(|s| s.id.clone())
            .collect();
        let count = finished.len();
        for id in &finished {
            if let Some(s) = self.sessions.remove(id)
                && let Err(e) = fs::remove_dir_all(&s.state_dir)
            {
                warn!(path = %s.state_dir.display(), error = %e, "failed to remove session dir");
            }
        }
        count
    }

    pub fn expire_stale(&mut self, timeout: Duration) -> usize {
        let now = Utc::now();
        let mut count = 0usize;
        for session in self.sessions.values_mut() {
            if session.status == SessionStatus::Active && !session.starred {
                let age = now.signed_duration_since(session.created_at);
                if age.to_std().unwrap_or(Duration::ZERO) > timeout {
                    session.status = SessionStatus::Expired;
                    session.persist();
                    count += 1;
                }
            }
        }
        if count > 0 {
            crate::metrics::SESSIONS_EXPIRED.inc_by(count as u64);
            crate::metrics::SESSIONS_ACTIVE.sub(count as i64);
        }
        count
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn mgr() -> (SessionManager, tempfile::TempDir) {
        let dir = tempdir().unwrap();
        (SessionManager::new(dir.path().to_path_buf()), dir)
    }

    #[test]
    fn create_returns_active_session() {
        let (mut m, _d) = mgr();
        let s = m.create(
            "# Hello".into(),
            Some("title".into()),
            None,
            HashMap::new(),
            false,
            None,
        );
        assert_eq!(s.status, SessionStatus::Active);
        assert_eq!(s.title.unwrap(), "title");
    }

    #[test]
    fn get_returns_created_session() {
        let (mut m, _d) = mgr();
        let s = m.create("content".into(), None, None, HashMap::new(), false, None);
        assert!(m.get(&s.id).is_some());
    }

    #[test]
    fn get_unknown_returns_none() {
        let (m, _d) = mgr();
        assert!(m.get("nope").is_none());
    }

    #[test]
    fn cancel_known_session_returns_true() {
        let (mut m, _d) = mgr();
        let s = m.create("x".into(), None, None, HashMap::new(), false, None);
        assert!(m.cancel(&s.id));
        assert_eq!(m.get(&s.id).unwrap().status, SessionStatus::Cancelled);
    }

    #[test]
    fn cancel_unknown_session_returns_false() {
        let (mut m, _d) = mgr();
        assert!(!m.cancel("ghost"));
    }

    #[test]
    fn submit_known_session_returns_ok() {
        let (mut m, _d) = mgr();
        let s = m.create("x".into(), None, None, HashMap::new(), false, None);
        assert!(matches!(
            m.submit(&s.id, "notes".into(), vec![], None, None, vec![]),
            SubmitResult::Ok
        ));
        let updated = m.get(&s.id).unwrap();
        assert_eq!(updated.status, SessionStatus::Submitted);
        assert_eq!(updated.typed_notes.as_deref(), Some("notes"));
    }

    #[test]
    fn submit_unknown_session_returns_not_found() {
        let (mut m, _d) = mgr();
        assert!(matches!(
            m.submit("ghost", "notes".into(), vec![], None, None, vec![]),
            SubmitResult::NotFound
        ));
    }

    #[test]
    fn submit_already_submitted_returns_not_active() {
        let (mut m, _d) = mgr();
        let s = m.create("x".into(), None, None, HashMap::new(), false, None);
        m.submit(&s.id, "first".into(), vec![], None, None, vec![]);
        assert!(matches!(
            m.submit(&s.id, "second".into(), vec![], None, None, vec![]),
            SubmitResult::NotActive
        ));
    }

    #[test]
    fn expire_stale_only_affects_active_sessions() {
        let (mut m, _d) = mgr();
        let active = m.create("a".into(), None, None, HashMap::new(), false, None);
        let will_cancel = m.create("b".into(), None, None, HashMap::new(), false, None);
        m.cancel(&will_cancel.id);

        m.expire_stale(Duration::ZERO);

        assert_eq!(m.get(&active.id).unwrap().status, SessionStatus::Expired);
        // Cancelled should remain Cancelled, not change to Expired
        assert_eq!(
            m.get(&will_cancel.id).unwrap().status,
            SessionStatus::Cancelled
        );
    }

    #[test]
    fn list_returns_all_sessions() {
        let (mut m, _d) = mgr();
        m.create("a".into(), None, None, HashMap::new(), false, None);
        m.create("b".into(), None, None, HashMap::new(), false, None);
        assert_eq!(m.list().len(), 2);
    }

    #[test]
    fn ids_are_12_chars() {
        let (mut m, _d) = mgr();
        let s = m.create("x".into(), None, None, HashMap::new(), false, None);
        assert_eq!(s.id.len(), 12);
        assert!(s.id.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn status_as_str_matches_expected_strings() {
        assert_eq!(SessionStatus::Active.as_str(), "Active");
        assert_eq!(SessionStatus::Submitted.as_str(), "Submitted");
        assert_eq!(SessionStatus::Cancelled.as_str(), "Cancelled");
        assert_eq!(SessionStatus::Expired.as_str(), "Expired");
    }

    #[test]
    fn status_as_str_case_insensitive_filter_works() {
        // mirrors the filter logic in list_sessions
        let matches =
            |s: &SessionStatus, filter: &str| s.as_str().to_lowercase() == filter.to_lowercase();
        assert!(matches(&SessionStatus::Active, "active"));
        assert!(matches(&SessionStatus::Active, "Active"));
        assert!(matches(&SessionStatus::Active, "ACTIVE"));
        assert!(!matches(&SessionStatus::Active, "cancelled"));
    }

    #[test]
    fn update_content_increments_version() {
        let (mut m, _d) = mgr();
        let s = m.create("v1".into(), None, None, HashMap::new(), false, None);
        assert_eq!(m.get(&s.id).unwrap().version, 1);
        let new_version = m.update_content(&s.id, "v2".into()).unwrap();
        assert_eq!(new_version, 2);
        let updated = m.get(&s.id).unwrap();
        assert_eq!(updated.status, SessionStatus::Active);
        assert_eq!(updated.content, "v2");
    }

    #[test]
    fn update_content_on_submitted_returns_none() {
        let (mut m, _d) = mgr();
        let s = m.create("content".into(), None, None, HashMap::new(), false, None);
        m.submit(&s.id, "done".into(), vec![], None, None, vec![]);
        assert!(m.update_content(&s.id, "new".into()).is_none());
    }

    #[test]
    fn starred_session_survives_purge_finished() {
        let (mut m, _d) = mgr();
        let pinned = m.create("keep".into(), None, None, HashMap::new(), true, None);
        let ephemeral = m.create("drop".into(), None, None, HashMap::new(), false, None);
        m.submit(&pinned.id, "notes".into(), vec![], None, None, vec![]);
        m.submit(&ephemeral.id, "notes".into(), vec![], None, None, vec![]);

        let purged = m.purge_finished();
        assert_eq!(purged, 1);
        assert!(m.get(&pinned.id).is_some());
        assert!(m.get(&ephemeral.id).is_none());
    }

    #[test]
    fn starred_session_skipped_by_expire_stale() {
        let (mut m, _d) = mgr();
        let pinned = m.create("keep".into(), None, None, HashMap::new(), true, None);
        let ephemeral = m.create("drop".into(), None, None, HashMap::new(), false, None);

        m.expire_stale(Duration::ZERO);

        assert_eq!(m.get(&pinned.id).unwrap().status, SessionStatus::Active);
        assert_eq!(m.get(&ephemeral.id).unwrap().status, SessionStatus::Expired);
    }

    #[test]
    fn set_starred_persists_and_toggles() {
        let (mut m, _d) = mgr();
        let s = m.create("x".into(), None, None, HashMap::new(), false, None);
        assert!(m.set_starred(&s.id, true));
        assert!(m.get(&s.id).unwrap().starred);
        assert!(m.set_starred(&s.id, false));
        assert!(!m.get(&s.id).unwrap().starred);
        assert!(!m.set_starred("ghost", true));
    }

    #[test]
    fn starred_cancelled_session_reloads_from_disk() {
        let dir = tempdir().unwrap();
        let id = {
            let mut m = SessionManager::new(dir.path().to_path_buf());
            let s = m.create(
                "important".into(),
                Some("doc".into()),
                None,
                HashMap::new(),
                true,
                Some(SessionOrigin {
                    cwd: Some("/home/me/project".into()),
                    host: Some("laptop".into()),
                    tool: Some("claude-code".into()),
                    git_branch: Some("main".into()),
                    git_remote: Some("git@github.com:me/project.git".into()),
                }),
            );
            m.cancel(&s.id);
            s.id
        };
        let m2 = SessionManager::new(dir.path().to_path_buf());
        let s = m2
            .get(&id)
            .expect("starred cancelled session should reload");
        assert_eq!(s.status, SessionStatus::Cancelled);
        assert!(s.starred);
        let origin = s.origin.as_ref().unwrap();
        assert_eq!(origin.cwd.as_deref(), Some("/home/me/project"));
        assert_eq!(origin.tool.as_deref(), Some("claude-code"));
    }

    #[test]
    fn unstarred_cancelled_session_does_not_reload() {
        let dir = tempdir().unwrap();
        let id = {
            let mut m = SessionManager::new(dir.path().to_path_buf());
            let s = m.create("x".into(), None, None, HashMap::new(), false, None);
            m.cancel(&s.id);
            s.id
        };
        let m2 = SessionManager::new(dir.path().to_path_buf());
        assert!(m2.get(&id).is_none());
    }

    #[test]
    fn session_persists_to_disk_and_reloads() {
        let dir = tempdir().unwrap();
        let id = {
            let mut m = SessionManager::new(dir.path().to_path_buf());
            m.create(
                "persisted".into(),
                Some("doc".into()),
                None,
                HashMap::new(),
                false,
                None,
            )
            .id
        };
        let m2 = SessionManager::new(dir.path().to_path_buf());
        let s = m2.get(&id).unwrap();
        assert_eq!(s.title.as_deref(), Some("doc"));
        assert_eq!(s.status, SessionStatus::Active);
    }
}
