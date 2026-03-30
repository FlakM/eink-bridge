use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::time::Duration;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SessionStatus {
    Active,
    Submitted,
    Cancelled,
    Expired,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub title: Option<String>,
    pub status: SessionStatus,
    pub content: String,
    pub created_at: DateTime<Utc>,
    pub typed_notes: Option<String>,
    pub annotation_images: Vec<String>,
    pub state_dir: PathBuf,
}

impl Session {
    pub fn save_annotation(&self, data: &[u8]) -> String {
        let dir = self.state_dir.join("annotations");
        fs::create_dir_all(&dir).ok();
        let filename = format!("img_{}.png", Uuid::new_v4().as_simple());
        let path = dir.join(&filename);
        fs::write(&path, data).ok();
        path.to_string_lossy().to_string()
    }

    fn persist(&self) {
        fs::create_dir_all(&self.state_dir).ok();
        let path = self.state_dir.join("session.json");
        if let Ok(json) = serde_json::to_string_pretty(self) {
            fs::write(path, json).ok();
        }
    }
}

pub struct SessionManager {
    sessions: HashMap<String, Session>,
    state_dir: PathBuf,
}

impl SessionManager {
    pub fn new(state_dir: PathBuf) -> Self {
        fs::create_dir_all(&state_dir).ok();
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
        for entry in entries.flatten() {
            let json_path = entry.path().join("session.json");
            if json_path.exists()
                && let Ok(content) = fs::read_to_string(&json_path)
                && let Ok(session) = serde_json::from_str::<Session>(&content)
                && (session.status == SessionStatus::Active
                    || session.status == SessionStatus::Submitted)
            {
                self.sessions.insert(session.id.clone(), session);
            }
        }
    }

    pub fn create(&mut self, content: String, title: Option<String>) -> Session {
        let id = Uuid::new_v4().as_simple().to_string()[..8].to_string();
        let session_dir = self.state_dir.join("sessions").join(&id);

        let session = Session {
            id: id.clone(),
            title,
            status: SessionStatus::Active,
            content,
            created_at: Utc::now(),
            typed_notes: None,
            annotation_images: Vec::new(),
            state_dir: session_dir,
        };
        session.persist();
        self.sessions.insert(id, session.clone());
        session
    }

    pub fn get(&self, id: &str) -> Option<&Session> {
        self.sessions.get(id)
    }

    pub fn list(&self) -> Vec<&Session> {
        self.sessions.values().collect()
    }

    pub fn list_by_status(&self, status: &SessionStatus) -> Vec<&Session> {
        self.sessions
            .values()
            .filter(|s| s.status == *status)
            .collect()
    }

    pub fn cancel(&mut self, id: &str) -> bool {
        if let Some(s) = self.sessions.get_mut(id) {
            s.status = SessionStatus::Cancelled;
            s.persist();
            true
        } else {
            false
        }
    }

    pub fn submit(&mut self, id: &str, typed_notes: String, images: Vec<String>) -> bool {
        if let Some(s) = self.sessions.get_mut(id) {
            s.typed_notes = Some(typed_notes);
            s.annotation_images = images;
            s.status = SessionStatus::Submitted;
            s.persist();
            true
        } else {
            false
        }
    }

    pub fn expire_stale(&mut self, timeout: Duration) {
        let now = Utc::now();
        for session in self.sessions.values_mut() {
            if session.status == SessionStatus::Active {
                let age = now.signed_duration_since(session.created_at);
                if age.to_std().unwrap_or(Duration::ZERO) > timeout {
                    session.status = SessionStatus::Expired;
                    session.persist();
                }
            }
        }
    }
}
