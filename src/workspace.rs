use crate::brace_matcher::char_to_byte_idx;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub fn file_modified_time(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).ok()?.modified().ok()
}

pub struct Document {
    pub title: String,
    pub path: Option<PathBuf>,
    pub content: String,
    pub is_dirty: bool,
    pub last_modified: Option<SystemTime>,
    pub cursor_char_idx: usize,
    pub selection_char_range: Option<(usize, usize)>,
    pub undo_stack: Vec<String>,
    pub redo_stack: Vec<String>,
}

impl Document {
    pub fn new_empty(index: usize) -> Self {
        Self {
            title: format!("untitled_{}.rs", index),
            path: None,
            content: String::new(),
            is_dirty: false,
            last_modified: None,
            cursor_char_idx: 0,
            selection_char_range: None,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
        }
    }

    pub fn new_starter_demo() -> Self {
        let sample = r#"// Welcome to RustPad (Drop 1 Prototype)
// PSPad-inspired lightweight programmer's editor in pure Rust.
//
// Try these built-in features right now:
// 1. Point-and-Shoot Member Functions (Option A): Place your cursor after `target.` or `msg.`
//    below and press `Ctrl+.` (or `Alt+M`, or `Ctrl+Space` after a dot)!
// 2. Offline Rust Docs (F1): Place your cursor on `Vec`, `Point3D`, or `println` and press `F1`.
// 3. Live Brace Matching: Move onto any `{`, `}`, `(`, `)`, `[`, or `]`.
// 4. Clips (`Ctrl+J`) & Block Comments (`Ctrl+/`).

#[derive(Debug, Clone)]
pub struct Point3D {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Point3D {
    pub fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }

    pub fn norm_sq(&self) -> f64 {
        (self.x * self.x) + (self.y * self.y) + (self.z * self.z)
    }
}

fn main() {
    let origin = Point3D::new(0.0, 0.0, 0.0);
    let target = Point3D::new(3.0, 4.0, 12.0);
    let msg = "  Hello from RustPad v0.5!  ";
    let mut samples: Vec<f64> = vec![1.0, 2.0, 3.0];

    // Notice that braces inside strings "{ not matched }" or comments /* { */ are ignored!
    println!("Origin: {:?}, Target norm^2: {}, Trimmed: {}", origin, target.norm_sq(), msg.trim());
    samples.push(target.norm_sq());
}
"#;
        Self {
            title: "demo_main.rs".to_string(),
            path: None,
            content: sample.to_string(),
            is_dirty: false,
            last_modified: None,
            cursor_char_idx: 0,
            selection_char_range: None,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
        }
    }

    pub fn from_file(path: PathBuf) -> Result<Self, String> {
        let raw = fs::read_to_string(&path)
            .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;
        // Normalize CRLF to LF for consistent internal indexing
        let content = raw.replace("\r\n", "\n");
        let title = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("file.rs")
            .to_string();

        let last_modified = file_modified_time(&path);
        Ok(Self {
            title,
            path: Some(path),
            content,
            is_dirty: false,
            last_modified,
            cursor_char_idx: 0,
            selection_char_range: None,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
        })
    }

    pub fn save(&mut self) -> Result<(), String> {
        if let Some(ref path) = self.path {
            fs::write(path, &self.content)
                .map_err(|e| format!("Failed to save {}: {}", path.display(), e))?;
            self.last_modified = file_modified_time(path);
            self.is_dirty = false;
            Ok(())
        } else {
            Err("No file path assigned".to_string())
        }
    }

    pub fn save_as(&mut self, path: PathBuf) -> Result<(), String> {
        fs::write(&path, &self.content)
            .map_err(|e| format!("Failed to save {}: {}", path.display(), e))?;
        self.title = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("file.rs")
            .to_string();
        self.last_modified = file_modified_time(&path);
        self.path = Some(path);
        self.is_dirty = false;
        Ok(())
    }

    /// Reloads the document content from disk, pushing the previous buffer to the undo stack.
/// Reloads the document content from disk, pushing the previous buffer to the undo stack.
    pub fn reload_from_disk(&mut self) -> Result<(), String> {
        let Some(ref path) = self.path else {
            return Err("No file path assigned".to_string());
        };
        let raw = fs::read_to_string(path)
            .map_err(|e| format!("Failed to reload {}: {}", path.display(), e))?;
        let new_mtime = file_modified_time(path); // immutable borrow of `self.path` ends here

        let new_content = raw.replace("\r\n", "\n");
        let snapshot = self.content.clone();
        self.push_undo_snapshot(snapshot);
        self.content = new_content;
        self.last_modified = new_mtime;
        self.is_dirty = false;
        let max_chars = self.content.chars().count();
        self.cursor_char_idx = self.cursor_char_idx.min(max_chars);
        self.selection_char_range = None;
        Ok(())
    }
    
    pub fn push_undo_snapshot(&mut self, snapshot: String) {
        if self
            .undo_stack
            .last()
            .map(|s| s == &snapshot)
            .unwrap_or(false)
        {
            return;
        }
        if self.undo_stack.len() >= 100 {
            self.undo_stack.remove(0);
        }
        self.undo_stack.push(snapshot);
        self.redo_stack.clear();
    }

    pub fn undo(&mut self) -> bool {
        if let Some(prev) = self.undo_stack.pop() {
            let current = std::mem::replace(&mut self.content, prev);
            self.redo_stack.push(current);
            self.is_dirty = true;
            true
        } else {
            false
        }
    }

    pub fn redo(&mut self) -> bool {
        if let Some(next) = self.redo_stack.pop() {
            let current = std::mem::replace(&mut self.content, next);
            self.undo_stack.push(current);
            self.is_dirty = true;
            true
        } else {
            false
        }
    }

    /// Toggles `// ` comments across the current line or selected line range (PSPad style)
    pub fn toggle_line_comments(&mut self) {
        let (sel_start_char, sel_end_char) = self
            .selection_char_range
            .unwrap_or((self.cursor_char_idx, self.cursor_char_idx));

        let min_char = sel_start_char.min(sel_end_char);
        let max_char = sel_start_char.max(sel_end_char);

        let min_byte = char_to_byte_idx(&self.content, min_char);
        let max_byte = char_to_byte_idx(&self.content, max_char);

        let line_start_byte = self.content[..min_byte]
            .rfind('\n')
            .map(|i| i + 1)
            .unwrap_or(0);

        let line_end_byte = self.content[max_byte..]
            .find('\n')
            .map(|i| max_byte + i)
            .unwrap_or(self.content.len());

        let block = &self.content[line_start_byte..line_end_byte];
        let lines: Vec<&str> = block.split('\n').collect();

        // Check if all non-empty lines already start with `//`
        let all_commented = lines
            .iter()
            .filter(|l| !l.trim().is_empty())
            .all(|l| l.trim_start().starts_with("//"));

        let mut new_lines = Vec::with_capacity(lines.len());
        for line in lines {
            if all_commented {
                // Strip one leading `// ` or `//`
                if let Some(pos) = line.find("//") {
                    let mut updated = String::with_capacity(line.len());
                    updated.push_str(&line[..pos]);
                    let rest = &line[pos + 2..];
                    if let Some(stripped) = rest.strip_prefix(' ') {
                        updated.push_str(stripped);
                    } else {
                        updated.push_str(rest);
                    }
                    new_lines.push(updated);
                } else {
                    new_lines.push(line.to_string());
                }
            } else if line.trim().is_empty() {
                new_lines.push(line.to_string());
            } else {
                new_lines.push(format!("// {}", line));
            }
        }

        let replacement = new_lines.join("\n");
        let snapshot = self.content.clone();
        self.push_undo_snapshot(snapshot);
        self.content
            .replace_range(line_start_byte..line_end_byte, &replacement);
        self.is_dirty = true;
    }
}
