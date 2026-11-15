use crate::workspace::Document;
use serde::{Deserialize, Serialize};
/// Session persistence for RustPad (`rustpad_session.json`).
/// Remembers open file paths, per-file cursor positions, active tab index,
/// Cargo project working directory, and visual preferences across restarts.
use std::fs;
use std::path::{Path, PathBuf};

pub const SESSION_FILE_NAME: &str = "rustpad_session.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedTabState {
    pub path: PathBuf,
    pub cursor_char_idx: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionState {
    pub open_files: Vec<SavedTabState>,
    pub active_tab_path: Option<PathBuf>,
    pub cargo_working_dir: String,
    pub dark_mode: bool,
    pub font_size: f32,
}

impl SessionState {
    /// Resolves the path to `rustpad_session.json` in the user's home directory
    /// (or falls back to the current working directory if no home env var is set).
    pub fn session_file_path() -> PathBuf {
        if let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
            PathBuf::from(home).join(".rustpad_session.json")
        } else {
            PathBuf::from(SESSION_FILE_NAME)
        }
    }

    pub fn load() -> Option<Self> {
        let primary = Self::session_file_path();
        if let Ok(raw) = fs::read_to_string(&primary) {
            if let Ok(state) = serde_json::from_str::<Self>(&raw) {
                return Some(state);
            }
        }
        // Fallback: also check local directory `rustpad_session.json`
        let local = Path::new(SESSION_FILE_NAME);
        if let Ok(raw) = fs::read_to_string(local) {
            if let Ok(state) = serde_json::from_str::<Self>(&raw) {
                return Some(state);
            }
        }
        None
    }

    pub fn save_from_app(
        documents: &[Document],
        active_tab: usize,
        cargo_working_dir: &str,
        dark_mode: bool,
        font_size: f32,
    ) {
        let mut open_files = Vec::new();
        for doc in documents {
            if let Some(ref p) = doc.path {
                open_files.push(SavedTabState {
                    path: p.clone(),
                    cursor_char_idx: doc.cursor_char_idx,
                });
            }
        }

        let active_tab_path = documents.get(active_tab).and_then(|d| d.path.clone());

        let state = SessionState {
            open_files,
            active_tab_path,
            cargo_working_dir: cargo_working_dir.to_string(),
            dark_mode,
            font_size,
        };

        if let Ok(json) = serde_json::to_string_pretty(&state) {
            let primary = Self::session_file_path();
            let _ = fs::write(&primary, &json);
            // Also write a copy in the current working directory if writable for easy inspection
            let _ = fs::write(SESSION_FILE_NAME, &json);
        }
    }
}
