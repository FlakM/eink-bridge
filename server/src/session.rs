use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
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
    /// Latest unacknowledged update request. Kept separately from final submission.
    #[serde(default)]
    pub pending_feedback: Option<serde_json::Value>,
    #[serde(default)]
    pub last_content_update: Option<serde_json::Value>,
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
    /// The checkout `/source` may read from. Recorded by the server, and only for a
    /// session created from this machine: `origin.cwd` is what the client *said*, and a
    /// client on the LAN must not be able to name a directory to read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_root: Option<PathBuf>,
    pub state_dir: PathBuf,
    #[serde(skip, default)]
    writer: Option<tokio::sync::mpsc::UnboundedSender<WriteRequest>>,
}

/// What a new session is made from.
#[derive(Default)]
pub struct NewSession {
    pub content: String,
    pub title: Option<String>,
    pub callback_url: Option<String>,
    pub tags: HashMap<String, String>,
    pub starred: bool,
    pub origin: Option<SessionOrigin>,
    pub source_root: Option<PathBuf>,
}

/// A disk write that has been moved off the session lock.
pub enum WriteRequest {
    Snapshot {
        path: PathBuf,
        json: String,
    },
    /// Asks the drain task to confirm everything before it has landed, and to say whether
    /// any of it failed. Ordering in the channel is what makes this a barrier.
    Barrier {
        ack: tokio::sync::oneshot::Sender<Result<(), String>>,
    },
}

/// Write snapshots in the order they were produced, until the queue closes.
///
/// The write runs on a blocking thread: moving it off the session lock is only half the
/// job, because `fs::write` inside the async task would still pin a runtime worker and
/// stall every other request while the disk works. A failure is kept until the next
/// barrier asks, so the response that promised durability can say it was not delivered.
pub async fn drain_writes(mut rx: tokio::sync::mpsc::UnboundedReceiver<WriteRequest>) {
    let mut failed: Option<String> = None;
    while let Some(request) = rx.recv().await {
        match request {
            WriteRequest::Snapshot { path, json } => {
                let shown = path.display().to_string();
                match tokio::task::spawn_blocking(move || write_snapshot(&path, &json)).await {
                    Ok(Ok(())) => {}
                    Ok(Err(e)) => {
                        error!(path = %shown, error = %e, "failed to persist session");
                        failed = Some(format!("{shown}: {e}"));
                    }
                    Err(_) => {
                        error!(path = %shown, "the writer thread panicked; a session snapshot was lost");
                        failed = Some(format!("{shown}: writer panicked"));
                    }
                }
            }
            WriteRequest::Barrier { ack } => {
                let _ = ack.send(failed.take().map_or(Ok(()), Err));
            }
        }
    }
}

/// Write the snapshot so that `session.json` is always a whole document: into a sibling
/// file, synced, then renamed over the old one. Writing in place left a torn file behind
/// a crash, and a torn file was silently a lost session.
fn write_snapshot(path: &Path, json: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let staged = path.with_extension("json.tmp");
    {
        let mut file = fs::File::create(&staged)?;
        std::io::Write::write_all(&mut file, json.as_bytes())?;
        file.sync_all()?;
    }
    fs::rename(&staged, path)
}

/// Write one uploaded annotation image into `dir` and return the path recorded for it.
pub fn write_annotation(dir: &Path, data: &[u8]) -> std::io::Result<String> {
    fs::create_dir_all(dir)?;
    let path = dir.join(format!("img_{}.png", Uuid::new_v4().as_simple()));
    fs::write(&path, data)?;
    Ok(path.to_string_lossy().to_string())
}

impl Session {
    /// Where this session's annotation images live.
    pub fn annotations_dir(&self) -> PathBuf {
        self.state_dir.join("annotations")
    }

    pub fn save_annotation(&self, data: &[u8]) -> std::io::Result<String> {
        write_annotation(&self.annotations_dir(), data)
    }

    /// Serialize now (cheap, and done while the caller holds the session), write later.
    ///
    /// The write used to happen right here, inside the manager's write lock, so every
    /// mutation waited on the disk and a slow or full disk stalled the whole API. Handing
    /// it to the drain task keeps the lock short; callers that must be durable before they
    /// answer ask the manager to flush.
    fn persist(&self) {
        let path = self.state_dir.join("session.json");
        match serde_json::to_string_pretty(self) {
            Ok(json) => match &self.writer {
                Some(writer) => {
                    if writer
                        .send(WriteRequest::Snapshot {
                            path: path.clone(),
                            json: json.clone(),
                        })
                        .is_err()
                    {
                        // The drain task is gone; do not drop the write on the floor.
                        write_inline(&path, &json);
                    }
                }
                None => write_inline(&path, &json),
            },
            Err(e) => error!(session = %self.id, error = %e, "failed to serialize session"),
        }
    }
}

/// A session file that cannot be read is moved next to itself rather than skipped or
/// deleted: the operator is told, and the bytes are kept for a look.
fn set_aside(json_path: &Path, error: &str) {
    let aside = json_path.with_extension("json.corrupt");
    warn!(path = %json_path.display(), error, aside = %aside.display(), "unreadable session file set aside");
    if let Err(e) = fs::rename(json_path, &aside) {
        error!(path = %json_path.display(), error = %e, "failed to set the session file aside");
    }
}

fn write_inline(path: &Path, json: &str) {
    if let Err(e) = write_snapshot(path, json) {
        error!(path = %path.display(), error = %e, "failed to persist session");
    }
}

pub struct SessionManager {
    sessions: HashMap<String, Session>,
    state_dir: PathBuf,
    writer: Option<tokio::sync::mpsc::UnboundedSender<WriteRequest>>,
}

impl SessionManager {
    pub fn new(state_dir: PathBuf) -> Self {
        if let Err(e) = fs::create_dir_all(&state_dir) {
            error!(path = %state_dir.display(), error = %e, "failed to create state dir");
        }
        let mut mgr = Self {
            sessions: HashMap::new(),
            state_dir,
            writer: None,
        };
        mgr.load_from_disk();
        mgr
    }

    /// A manager whose writes are queued for a drain task, so no mutation waits on the
    /// disk while holding the session lock.
    pub fn with_writer(
        state_dir: PathBuf,
        writer: tokio::sync::mpsc::UnboundedSender<WriteRequest>,
    ) -> Self {
        if let Err(e) = fs::create_dir_all(&state_dir) {
            error!(path = %state_dir.display(), error = %e, "failed to create state dir");
        }
        let mut mgr = Self {
            sessions: HashMap::new(),
            state_dir,
            writer: Some(writer),
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
            if !json_path.exists() {
                continue;
            }
            let parsed = fs::read_to_string(&json_path)
                .map_err(|e| e.to_string())
                .and_then(|content| {
                    serde_json::from_str::<Session>(&content).map_err(|e| e.to_string())
                });
            match parsed {
                Err(error) => set_aside(&json_path, &error),
                Ok(session) => {
                    let is_live = matches!(
                        session.status,
                        SessionStatus::Active | SessionStatus::Submitted
                    );
                    if is_live || session.starred {
                        if session.starred {
                            loaded_starred += 1;
                        }
                        let mut session = session;
                        session.writer = self.writer.clone();
                        self.sessions.insert(session.id.clone(), session);
                        loaded += 1;
                    }
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
        self.create_session(NewSession {
            content,
            title,
            callback_url,
            tags,
            starred,
            origin,
            source_root: None,
        })
    }

    pub fn create_session(&mut self, new: NewSession) -> Session {
        let id = loop {
            let candidate = Uuid::new_v4().as_simple().to_string()[..12].to_string();
            if !self.sessions.contains_key(&candidate) {
                break candidate;
            }
            warn!(id = %candidate, "session ID collision, retrying");
        };
        let session_dir = self.state_dir.join("sessions").join(&id);

        let now = Utc::now();
        let session = Session {
            schema_version: SCHEMA_VERSION,
            id: id.clone(),
            title: new.title,
            status: SessionStatus::Active,
            version: 1,
            content: new.content,
            created_at: now,
            updated_at: now,
            typed_notes: None,
            verdict: None,
            annotation_images: Vec::new(),
            stroke_data: None,
            annotations: Vec::new(),
            pending_feedback: None,
            last_content_update: None,
            callback_url: new.callback_url,
            tags: new.tags,
            starred: new.starred,
            origin: new.origin,
            source_root: new.source_root,
            state_dir: session_dir,
            writer: self.writer.clone(),
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

    /// Record where this session's events should be delivered.
    ///
    /// Separate from creation because the channel instance that owns a session only
    /// learns its own port at startup: `push` cannot know which instance will claim the
    /// session. Registering on claim is also what repairs the URL after a restart.
    pub fn set_callback(&mut self, id: &str, url: Option<String>) -> bool {
        if let Some(s) = self.sessions.get_mut(id) {
            if s.callback_url != url {
                s.callback_url = url;
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

    pub fn record_content_update(&mut self, id: &str, event: serde_json::Value, acknowledge: bool) {
        if let Some(s) = self.sessions.get_mut(id) {
            if acknowledge {
                s.pending_feedback = None;
            }
            s.last_content_update = Some(event);
            s.persist();
        }
    }

    pub fn record_feedback(&mut self, id: &str, version: u32, feedback: serde_json::Value) -> bool {
        let Some(s) = self.sessions.get_mut(id) else {
            return false;
        };
        if s.status != SessionStatus::Active || s.version != version {
            return false;
        }
        s.pending_feedback = Some(feedback);
        s.updated_at = Utc::now();
        s.persist();
        true
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

    pub fn state_dir(&self) -> &std::path::Path {
        &self.state_dir
    }

    pub fn count(&self) -> usize {
        self.sessions.len()
    }

    pub fn starred_count(&self) -> usize {
        self.sessions.values().filter(|s| s.starred).count()
    }

    pub fn delete(&mut self, id: &str) -> bool {
        self.remove(id).is_some()
    }

    /// Forget a session and delete its files, handing back what it was so the caller can
    /// tell a live review from a finished one.
    pub fn remove(&mut self, id: &str) -> Option<Session> {
        let session = self.sessions.remove(id)?;
        let _ = fs::remove_dir_all(&session.state_dir);
        Some(session)
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
        self.expire_stale_ids(timeout).len()
    }

    /// Expire the Active sessions nobody has touched for `timeout`. Idle is measured from
    /// the last change, not from creation: a review that is being edited and annotated
    /// is in use however long ago it was opened.
    pub(crate) fn expire_stale_ids(&mut self, timeout: Duration) -> Vec<String> {
        let now = Utc::now();
        let mut expired = Vec::new();
        for session in self.sessions.values_mut() {
            if session.status == SessionStatus::Active && !session.starred {
                let age = now.signed_duration_since(session.updated_at);
                if age.to_std().unwrap_or(Duration::ZERO) > timeout {
                    session.status = SessionStatus::Expired;
                    session.updated_at = now;
                    session.persist();
                    expired.push(session.id.clone());
                }
            }
        }
        let count = expired.len();
        if count > 0 {
            crate::metrics::SESSIONS_EXPIRED.inc_by(count as u64);
            crate::metrics::SESSIONS_ACTIVE.sub(count as i64);
        }
        expired
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
    fn a_session_in_use_does_not_expire_however_old_it_is() {
        // Expiry used to count from creation, so a review being annotated for forty
        // minutes vanished under the reader's pen.
        let (mut m, _d) = mgr();
        let id = m
            .create("x".into(), None, None, HashMap::new(), false, None)
            .id;
        let hour = chrono::Duration::hours(1);
        {
            let s = m.sessions.get_mut(&id).unwrap();
            s.created_at = Utc::now() - hour;
            s.updated_at = Utc::now();
        }
        assert_eq!(m.expire_stale(Duration::from_secs(30 * 60)), 0);
        assert_eq!(m.get(&id).unwrap().status, SessionStatus::Active);

        m.sessions.get_mut(&id).unwrap().updated_at = Utc::now() - hour;
        assert_eq!(m.expire_stale(Duration::from_secs(30 * 60)), 1);
        assert_eq!(m.get(&id).unwrap().status, SessionStatus::Expired);
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

#[cfg(test)]
mod disk_trouble {
    use super::*;

    /// A state directory that cannot be created, because something else is already there.
    fn blocked_state_dir() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("not-a-directory");
        fs::write(&path, b"in the way").unwrap();
        (dir, path)
    }

    #[test]
    fn a_state_directory_that_cannot_be_written_still_serves_the_review() {
        // Disk trouble must degrade, not take the server down: the reader's page is in
        // memory and a review that cannot be persisted is still better than no server.
        let (_dir, blocked) = blocked_state_dir();
        let mut manager = SessionManager::new(blocked.clone());
        let session = manager.create(
            "# Review".into(),
            Some("title".into()),
            None,
            HashMap::new(),
            false,
            None,
        );
        assert!(manager.get(&session.id).is_some(), "the session is usable");
        assert!(manager.list().len() == 1);
        assert!(session.state_dir.starts_with(&blocked));
    }

    #[test]
    fn an_annotation_that_cannot_be_written_is_an_error_not_a_path() {
        // Recording a path for bytes that never landed made the submit say OK and the
        // result list an image nobody could open.
        let (_dir, blocked) = blocked_state_dir();
        let mut manager = SessionManager::new(blocked);
        let session = manager.create("# Review".into(), None, None, HashMap::new(), false, None);
        assert!(session.save_annotation(b"\x89PNG").is_err());
    }

    #[test]
    fn an_unreadable_session_file_is_set_aside_and_reported() {
        // A torn or hand-edited session.json used to be skipped in silence: the review
        // was simply gone after a restart, with nothing in the log to say why.
        let dir = tempfile::tempdir().unwrap();
        let healthy = {
            let mut manager = SessionManager::new(dir.path().to_path_buf());
            manager
                .create("# Fine".into(), None, None, HashMap::new(), false, None)
                .id
        };
        let torn_dir = dir.path().join("sessions").join("torn00000000");
        fs::create_dir_all(&torn_dir).unwrap();
        let torn = torn_dir.join("session.json");
        fs::write(&torn, b"{\"id\": \"torn00000000\", \"status\": \"Act").unwrap();

        let manager = SessionManager::new(dir.path().to_path_buf());
        assert!(manager.get(&healthy).is_some(), "the good file still loads");
        assert!(manager.get("torn00000000").is_none());
        assert!(
            !torn.exists(),
            "the bad file is no longer where the loader looks"
        );
        assert_eq!(
            fs::read(torn_dir.join("session.json.corrupt")).unwrap(),
            b"{\"id\": \"torn00000000\", \"status\": \"Act",
            "and it is kept, byte for byte, for a look"
        );
    }

    #[test]
    fn a_session_whose_directory_is_gone_can_still_be_removed() {
        // The state dir can be cleaned up under the server's feet; removal must not fail
        // on a missing directory.
        let dir = tempfile::tempdir().unwrap();
        let mut manager = SessionManager::new(dir.path().to_path_buf());
        let session = manager.create("# Review".into(), None, None, HashMap::new(), false, None);
        fs::remove_dir_all(&session.state_dir).unwrap();
        assert!(manager.delete(&session.id), "removal should still succeed");
        assert!(manager.get(&session.id).is_none());
        assert!(
            !manager.delete(&session.id),
            "removing twice is not a success"
        );
    }
}

#[cfg(test)]
mod write_queue {
    use super::*;

    fn session_json(dir: &std::path::Path, id: &str) -> PathBuf {
        dir.join("sessions").join(id).join("session.json")
    }

    #[tokio::test]
    async fn a_queued_write_does_not_hold_the_caller() {
        // With a drain task that is not running, the mutation still returns: the write is
        // a channel push, so the session lock is never held while the disk is written.
        let dir = tempfile::tempdir().unwrap();
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let mut manager = SessionManager::with_writer(dir.path().to_path_buf(), tx);

        let session = manager.create(
            "# Review".into(),
            Some("title".into()),
            None,
            HashMap::new(),
            false,
            None,
        );
        assert!(
            manager.get(&session.id).is_some(),
            "the session is readable immediately"
        );
        assert!(
            !session_json(dir.path(), &session.id).exists(),
            "and the disk has not been touched yet"
        );

        // Draining lands the write. Every sender has to be gone first or the drain waits
        // for more work: the manager holds one, and so does the clone `create` returned.
        let id = session.id.clone();
        drop(session);
        drop(manager);
        crate::session::drain_writes(rx).await;
        let on_disk = fs::read_to_string(session_json(dir.path(), &id)).unwrap();
        assert!(on_disk.contains("# Review"));
    }

    #[tokio::test]
    async fn queued_writes_land_in_the_order_they_were_produced() {
        let dir = tempfile::tempdir().unwrap();
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let mut manager = SessionManager::with_writer(dir.path().to_path_buf(), tx);
        let id = manager
            .create("# v1".into(), None, None, HashMap::new(), false, None)
            .id;

        for version in 2..=5 {
            assert_eq!(
                manager.update_content(&id, format!("# v{version}")),
                Some(version)
            );
        }
        drop(manager);
        crate::session::drain_writes(rx).await;

        let on_disk = fs::read_to_string(session_json(dir.path(), &id)).unwrap();
        assert!(
            on_disk.contains("# v5"),
            "the last write wins, so a stale snapshot cannot overwrite a newer one"
        );
    }

    #[tokio::test]
    async fn without_a_writer_the_write_is_inline() {
        // The synchronous path is what unit tests and any embedder without a runtime use.
        let dir = tempfile::tempdir().unwrap();
        let mut manager = SessionManager::new(dir.path().to_path_buf());
        let session = manager.create("# Inline".into(), None, None, HashMap::new(), false, None);
        assert!(session_json(dir.path(), &session.id).exists());
    }

    #[tokio::test]
    async fn a_snapshot_is_whole_or_absent_and_leaves_no_staging_file() {
        // The write goes to a sibling and is renamed over the old file, so a crash in the
        // middle leaves the previous whole snapshot rather than half of the new one.
        let dir = tempfile::tempdir().unwrap();
        let mut manager = SessionManager::new(dir.path().to_path_buf());
        let id = manager
            .create("# v1".into(), None, None, HashMap::new(), false, None)
            .id;
        manager.update_content(&id, "# v2".into());
        let path = session_json(dir.path(), &id);
        let on_disk: Session = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(on_disk.content, "# v2");
        assert!(
            !path.with_extension("json.tmp").exists(),
            "the staging file is renamed away, not left behind"
        );
    }

    #[tokio::test]
    async fn a_barrier_reports_a_write_that_did_not_land() {
        // The response that waits on the barrier is a promise the change is on disk; a
        // barrier that only said "done" let it promise a write that had failed.
        let dir = tempfile::tempdir().unwrap();
        let blocked = dir.path().join("not-a-directory");
        fs::write(&blocked, b"in the way").unwrap();
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let drain = tokio::spawn(crate::session::drain_writes(rx));
        tx.send(WriteRequest::Snapshot {
            path: blocked.join("sessions").join("x").join("session.json"),
            json: "{}".into(),
        })
        .unwrap();
        let (ack, waited) = tokio::sync::oneshot::channel();
        tx.send(WriteRequest::Barrier { ack }).unwrap();
        let outcome = waited.await.unwrap();
        assert!(
            outcome.is_err(),
            "the failure reaches the barrier: {outcome:?}"
        );

        // Once reported, the failure is spent: the next barrier speaks for the writes
        // after it.
        let (ack, waited) = tokio::sync::oneshot::channel();
        tx.send(WriteRequest::Barrier { ack }).unwrap();
        assert!(waited.await.unwrap().is_ok());
        drop(tx);
        drain.await.unwrap();
    }

    #[tokio::test]
    async fn a_dropped_drain_task_does_not_lose_the_write() {
        // If the queue is gone, the write falls back to the caller rather than vanishing.
        let dir = tempfile::tempdir().unwrap();
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        drop(rx);
        let mut manager = SessionManager::with_writer(dir.path().to_path_buf(), tx);
        let session = manager.create("# Orphaned".into(), None, None, HashMap::new(), false, None);
        assert!(
            session_json(dir.path(), &session.id).exists(),
            "a dead queue must not mean a lost session"
        );
    }
}
