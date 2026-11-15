mod brace_matcher;
mod cargo_runner;
mod highlighter;
mod member_help;
mod search;
mod session;
mod templates;
mod workspace;

use eframe::egui::text::{CCursor, CCursorRange};
use eframe::egui::text_edit::TextEditOutput;
use eframe::egui::{self, Color32, FontFamily, FontId, Key, KeyboardShortcut, Modifiers, RichText};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;

use brace_matcher::{byte_to_line_col, char_to_byte_idx, find_matching_brace, BraceMatch};
use cargo_runner::{
    create_cargo_new_project, find_cargo_root, spawn_cargo_command, CargoEvent, CargoSubcommand,
    CompilerDiagnostic, DiagnosticLevel,
};
use highlighter::highlight_rust_code;
use member_help::{
    analyze_cursor_receiver, apply_method_insertion, methods_for_category, open_offline_rust_docs,
    word_at_cursor, MethodEntry, TypeCategory,
};
use search::{find_in_text, run_multifile_search, SearchMatch, SearchScope};
use session::SessionState;
use templates::{insert_template_at_cursor, try_expand_template, BUILTIN_TEMPLATES};
use workspace::Document;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BottomTab {
    CompilerDiagnostics,
    CargoRawLog,
    SearchResults,
}

fn main() -> eframe::Result<()> {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("RustPad v0.5 — Lightweight Rust Programmer's Editor")
            .with_inner_size([1280.0, 840.0]),
        ..Default::default()
    };

    eframe::run_native(
        "RustPad",
        native_options,
        Box::new(|cc| Ok(Box::new(RustPadApp::new(cc)))),
    )
}

struct RustPadApp {
    documents: Vec<Document>,
    active_tab: usize,
    untitled_counter: usize,
    dark_mode: bool,
    font_size: f32,
    show_clips_panel: bool,

    // Single-file Find / Replace bar (Ctrl+F / Ctrl+H / F3 / Shift+F3)
    show_find_bar: bool,
    focus_find_input: bool,
    find_query: String,
    replace_query: String,
    find_case_sensitive: bool,

    // Multi-file Search dialog & bottom results panel (Ctrl+Shift+F)
    show_find_in_files_modal: bool,
    multifile_query: String,
    multifile_scope: SearchScope,
    multifile_dir: String,
    multifile_extensions: String,
    multifile_results: Vec<SearchMatch>,

    // Cargo Build / Check / Test / Run Integration
    cargo_working_dir: String,
    show_project_folder_modal: bool,
    cargo_rx: Option<Receiver<CargoEvent>>,
    cargo_running: bool,
    cargo_diagnostics: Vec<CompilerDiagnostic>,
    selected_diagnostic_idx: Option<usize>,
    cargo_raw_log: Vec<String>,

    // `cargo new` Project Creator Modal (Ctrl+Shift+N)
    show_cargo_new_modal: bool,
    cargo_new_parent_dir: String,
    cargo_new_crate_name: String,
    cargo_new_is_lib: bool,
    cargo_new_error: Option<String>,

    // Option A + B: Point-and-Shoot Member Function Browser (Ctrl+. or Alt+M) & F1 Docs
    show_member_help_modal: bool,
    focus_member_filter: bool,
    member_category: TypeCategory,
    member_filter: String,
    member_selected_idx: usize,
    member_receiver_ident: Option<String>,
    member_inferred_type: Option<String>,
    member_replace_range: (usize, usize),

    // Bottom Dock Panel (PSPad Log / Compiler / Search Results)
    show_bottom_panel: bool,
    bottom_tab: BottomTab,

    // Go To Line modal (Ctrl+G)
    show_goto_line_modal: bool,
    goto_line_input: String,

    status_message: String,
    pending_selection_override: Option<(usize, usize)>,
}

impl RustPadApp {
    fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let cwd = std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .display()
            .to_string();

        let mut documents = Vec::new();
        let mut active_tab = 0usize;
        let mut cargo_working_dir = cwd.clone();
        let mut dark_mode = true;
        let mut font_size = 14.5_f32;
        let mut restored_msg = None;
        let mut initial_cursor_override = None;

        // 1. Attempt to restore previous session from `rustpad_session.json`
        if let Some(saved) = SessionState::load() {
            dark_mode = saved.dark_mode;
            font_size = saved.font_size.clamp(9.0, 28.0);
            if !saved.cargo_working_dir.trim().is_empty()
                && Path::new(saved.cargo_working_dir.trim()).exists()
            {
                cargo_working_dir = saved.cargo_working_dir.clone();
            }

            for tab_state in saved.open_files {
                if tab_state.path.is_file() {
                    if let Ok(mut doc) = Document::from_file(tab_state.path.clone()) {
                        let max_chars = doc.content.chars().count();
                        doc.cursor_char_idx = tab_state.cursor_char_idx.min(max_chars);
                        documents.push(doc);
                    }
                }
            }

            if let Some(active_path) = saved.active_tab_path {
                if let Some(pos) = documents
                    .iter()
                    .position(|d| d.path.as_ref() == Some(&active_path))
                {
                    active_tab = pos;
                }
            }

            if !documents.is_empty() {
                let c_idx = documents[active_tab].cursor_char_idx;
                initial_cursor_override = Some((c_idx, c_idx));
                restored_msg = Some(format!(
                    "Restored session ({} file(s), active: {})",
                    documents.len(),
                    documents[active_tab].title
                ));
            }
        }

        // Fallback to starter demo if no session or no saved files existed on disk
        if documents.is_empty() {
            documents.push(Document::new_starter_demo());
            active_tab = 0;
        }

        let parent_default = Path::new(&cargo_working_dir)
            .parent()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| cargo_working_dir.clone());

        Self {
            documents,
            active_tab,
            untitled_counter: 1,
            dark_mode,
            font_size,
            show_clips_panel: false,

            show_find_bar: false,
            focus_find_input: false,
            find_query: String::new(),
            replace_query: String::new(),
            find_case_sensitive: false,

            show_find_in_files_modal: false,
            multifile_query: String::new(),
            multifile_scope: SearchScope::Directory,
            multifile_dir: cargo_working_dir.clone(),
            multifile_extensions: "rs, toml, md".to_string(),
            multifile_results: Vec::new(),

            cargo_working_dir,
            show_project_folder_modal: false,
            cargo_rx: None,
            cargo_running: false,
            cargo_diagnostics: Vec::new(),
            selected_diagnostic_idx: None,
            cargo_raw_log: Vec::new(),

            show_cargo_new_modal: false,
            cargo_new_parent_dir: parent_default,
            cargo_new_crate_name: "my_rust_app".to_string(),
            cargo_new_is_lib: false,
            cargo_new_error: None,

            show_member_help_modal: false,
            focus_member_filter: false,
            member_category: TypeCategory::StrAndString,
            member_filter: String::new(),
            member_selected_idx: 0,
            member_receiver_ident: None,
            member_inferred_type: None,
            member_replace_range: (0, 0),

            show_bottom_panel: true,
            bottom_tab: BottomTab::CompilerDiagnostics,

            show_goto_line_modal: false,
            goto_line_input: String::new(),

            status_message: restored_msg.unwrap_or_else(|| {
                "Ready — Press Ctrl+. (or Alt+M) for Point & Shoot Member Help, or F1 for Rust Docs."
                    .to_string()
            }),
            pending_selection_override: initial_cursor_override,
        }
    }

    fn save_session(&self) {
        SessionState::save_from_app(
            &self.documents,
            self.active_tab,
            &self.cargo_working_dir,
            self.dark_mode,
            self.font_size,
        );
    }

    fn active_doc(&self) -> Option<&Document> {
        self.documents.get(self.active_tab)
    }

    fn active_doc_mut(&mut self) -> Option<&mut Document> {
        self.documents.get_mut(self.active_tab)
    }

    fn has_cargo_toml(&self) -> bool {
        Path::new(self.cargo_working_dir.trim())
            .join("Cargo.toml")
            .is_file()
    }

    /// Triggers Option A: Point-and-Shoot Member Function Browser at the current cursor position.
    fn action_open_member_help(&mut self) {
        let Some(doc) = self.documents.get(self.active_tab) else {
            return;
        };
        let ctx_info = analyze_cursor_receiver(&doc.content, doc.cursor_char_idx, &self.documents);

        if let Some(cat) = ctx_info.inferred_category {
            self.member_category = cat;
        }
        self.member_receiver_ident = ctx_info.receiver_ident;
        self.member_inferred_type = ctx_info.inferred_type_name;
        self.member_filter = ctx_info.partial_method;
        self.member_replace_range = (ctx_info.replace_start_char, ctx_info.replace_end_char);
        self.member_selected_idx = 0;
        self.show_member_help_modal = true;
        self.focus_member_filter = true;

        if let (Some(ref var), Some(ref ty)) =
            (&self.member_receiver_ident, &self.member_inferred_type)
        {
            self.status_message = format!(
                "Member Help: detected `{}` as `{}` — click a method or press Enter to insert.",
                var, ty
            );
        } else {
            self.status_message =
                "Member Help: pick a type tab, filter methods, and click or press Enter to shoot into code."
                    .to_string();
        }
    }

    /// Shoots the selected `MethodEntry` into the active document at `self.member_replace_range`.
    fn action_shoot_member_entry(&mut self, entry: &MethodEntry) {
        let (start_char, end_char) = self.member_replace_range;
        if let Some(doc) = self.active_doc_mut() {
            let snap = doc.content.clone();
            doc.push_undo_snapshot(snap);
            let new_cursor = apply_method_insertion(&mut doc.content, start_char, end_char, entry);
            doc.cursor_char_idx = new_cursor;
            doc.selection_char_range = Some((new_cursor, new_cursor));
            doc.is_dirty = true;
            self.pending_selection_override = Some((new_cursor, new_cursor));
            self.status_message = format!("Inserted `.{}` — {}", entry.name, entry.signature);
        }
        self.show_member_help_modal = false;
    }

    /// Triggers Option B (`F1`): Opens offline Rust documentation (`rustup doc --std <symbol>`).
    fn action_lookup_f1_docs(&mut self) {
        let query = if let Some(doc) = self.active_doc() {
            // Check if the user has highlighted a word first, otherwise inspect word at cursor
            if let Some((s, e)) = doc.selection_char_range {
                if s != e {
                    let min_c = s.min(e);
                    let max_c = s.max(e);
                    let b_start = char_to_byte_idx(&doc.content, min_c);
                    let b_end = char_to_byte_idx(&doc.content, max_c);
                    let selected = doc.content[b_start..b_end].trim().to_string();
                    if !selected.is_empty() && !selected.contains('\n') {
                        selected
                    } else {
                        word_at_cursor(&doc.content, doc.cursor_char_idx).unwrap_or_default()
                    }
                } else {
                    word_at_cursor(&doc.content, doc.cursor_char_idx).unwrap_or_default()
                }
            } else {
                word_at_cursor(&doc.content, doc.cursor_char_idx).unwrap_or_default()
            }
        } else {
            String::new()
        };

        self.status_message = open_offline_rust_docs(&query);
    }

    fn action_browse_project_folder(&mut self) {
        if let Some(folder) = rfd::FileDialog::new()
            .set_title("Select Cargo Project Folder (containing Cargo.toml)")
            .pick_folder()
        {
            let resolved = find_cargo_root(&folder).unwrap_or(folder);
            self.cargo_working_dir = resolved.display().to_string();
            self.multifile_dir = self.cargo_working_dir.clone();
            self.status_message =
                format!("Set Cargo Project Folder to: {}", self.cargo_working_dir);
            self.save_session();
        }
    }

    fn action_detect_project_from_active_file(&mut self) {
        if let Some(active_path) = self.active_doc().and_then(|d| d.path.clone()) {
            if let Some(root) = find_cargo_root(&active_path) {
                self.cargo_working_dir = root.display().to_string();
                self.multifile_dir = self.cargo_working_dir.clone();
                self.status_message =
                    format!("Detected Cargo Project root: {}", self.cargo_working_dir);
                self.save_session();
            } else if let Some(parent) = active_path.parent() {
                self.cargo_working_dir = parent.display().to_string();
                self.status_message = format!(
                    "No Cargo.toml found upward; set folder to: {}",
                    self.cargo_working_dir
                );
                self.save_session();
            }
        } else {
            self.status_message =
                "Active tab is unsaved; save it first or use Browse to pick a Project Folder."
                    .to_string();
        }
    }

    fn action_execute_cargo_new(&mut self) {
        let parent = PathBuf::from(self.cargo_new_parent_dir.trim());
        match create_cargo_new_project(&parent, &self.cargo_new_crate_name, self.cargo_new_is_lib) {
            Ok((project_dir, entry_rs)) => {
                self.cargo_working_dir = project_dir.display().to_string();
                self.multifile_dir = self.cargo_working_dir.clone();
                self.cargo_new_error = None;
                self.show_cargo_new_modal = false;

                if entry_rs.is_file() {
                    self.open_or_activate_path(entry_rs.clone());
                }
                self.save_session();
                self.status_message = format!(
                    "Created new Cargo project at {} and opened {}",
                    project_dir.display(),
                    entry_rs.display()
                );
            }
            Err(err_msg) => {
                self.cargo_new_error = Some(err_msg.clone());
                self.status_message = err_msg;
            }
        }
    }

    fn action_new_file(&mut self) {
        self.untitled_counter += 1;
        self.documents
            .push(Document::new_empty(self.untitled_counter));
        self.active_tab = self.documents.len() - 1;
        self.status_message = "Created new untitled buffer.".to_string();
    }

    fn action_open_file(&mut self) {
        if let Some(paths) = rfd::FileDialog::new()
            .add_filter("Rust & Project Files", &["rs", "toml", "md", "txt", "json"])
            .add_filter("All Files", &["*"])
            .pick_files()
        {
            for path in paths {
                if let Some(root) = find_cargo_root(&path) {
                    self.cargo_working_dir = root.display().to_string();
                    self.multifile_dir = self.cargo_working_dir.clone();
                }
                self.open_or_activate_path(path);
            }
            self.save_session();
        }
    }

    fn open_or_activate_path(&mut self, path: PathBuf) -> bool {
        if let Some(existing_idx) = self
            .documents
            .iter()
            .position(|d| d.path.as_ref() == Some(&path))
        {
            self.active_tab = existing_idx;
            self.save_session();
            return true;
        }

        match Document::from_file(path) {
            Ok(doc) => {
                self.status_message = format!("Opened {}", doc.title);
                self.documents.push(doc);
                self.active_tab = self.documents.len() - 1;
                self.save_session();
                true
            }
            Err(err) => {
                self.status_message = err;
                false
            }
        }
    }

    fn action_save_active(&mut self) {
        let Some(doc) = self.active_doc_mut() else {
            return;
        };
        if doc.path.is_some() {
            match doc.save() {
                Ok(()) => {
                    self.status_message =
                        format!("Saved {}", self.documents[self.active_tab].title);
                    self.save_session();
                }
                Err(e) => self.status_message = e,
            }
        } else {
            self.action_save_as_active();
        }
    }

    fn action_save_as_active(&mut self) {
        let Some(default_name) = self.active_doc().map(|d| d.title.clone()) else {
            return;
        };
        if let Some(path) = rfd::FileDialog::new()
            .set_file_name(&default_name)
            .add_filter("Rust Source", &["rs"])
            .add_filter("All Files", &["*"])
            .save_file()
        {
            if let Some(root) = find_cargo_root(&path) {
                self.cargo_working_dir = root.display().to_string();
            }
            if let Some(doc) = self.active_doc_mut() {
                match doc.save_as(path) {
                    Ok(()) => {
                        self.status_message =
                            format!("Saved as {}", self.documents[self.active_tab].title);
                        self.save_session();
                    }
                    Err(e) => self.status_message = e,
                }
            }
        }
    }

    fn action_save_all(&mut self) {
        let mut saved_count = 0usize;
        for doc in &mut self.documents {
            if doc.path.is_some() && doc.is_dirty {
                if doc.save().is_ok() {
                    saved_count += 1;
                }
            }
        }
        self.save_session();
        self.status_message = format!("Save All completed ({} file(s) written).", saved_count);
    }

    fn action_close_tab(&mut self, index: usize) {
        if index < self.documents.len() {
            let closed_title = self.documents[index].title.clone();
            self.documents.remove(index);
            if self.documents.is_empty() {
                self.untitled_counter += 1;
                self.documents
                    .push(Document::new_empty(self.untitled_counter));
                self.active_tab = 0;
            } else if self.active_tab >= self.documents.len() {
                self.active_tab = self.documents.len() - 1;
            }
            self.save_session();
            self.status_message = format!("Closed {}", closed_title);
        }
    }

    fn action_expand_clip(&mut self) {
        if let Some(doc) = self.active_doc_mut() {
            let snapshot = doc.content.clone();
            if let Some((new_cursor, trigger)) =
                try_expand_template(&mut doc.content, doc.cursor_char_idx)
            {
                doc.push_undo_snapshot(snapshot);
                doc.cursor_char_idx = new_cursor;
                doc.is_dirty = true;
                self.pending_selection_override = Some((new_cursor, new_cursor));
                self.status_message = format!("Expanded clip template '{}'", trigger);
            } else {
                self.status_message = "No matching clip trigger before cursor (try: fn, pfn, for, if, match, struct, impl, test).".to_string();
            }
        }
    }

    fn action_toggle_comments(&mut self) {
        if let Some(doc) = self.active_doc_mut() {
            doc.toggle_line_comments();
            self.status_message = "Toggled line comments (//).".to_string();
        }
    }

    fn action_find_step(&mut self, backward: bool) {
        let query = self.find_query.clone();
        if query.is_empty() {
            self.show_find_bar = true;
            self.focus_find_input = true;
            return;
        }
        let case_sensitive = self.find_case_sensitive;

        if let Some(doc) = self.active_doc_mut() {
            if let Some((start_char, end_char, byte_pos)) = find_in_text(
                &doc.content,
                &query,
                doc.cursor_char_idx,
                case_sensitive,
                backward,
            ) {
                let (line, col) = byte_to_line_col(&doc.content, byte_pos);
                doc.cursor_char_idx = end_char;
                doc.selection_char_range = Some((start_char, end_char));
                self.pending_selection_override = Some((start_char, end_char));
                self.status_message = format!(
                    "Found '{}' at Ln {}, Col {} (chars {}..{})",
                    query, line, col, start_char, end_char
                );
            } else {
                self.status_message = format!("'{}' not found in current file.", query);
            }
        }
    }

    fn action_run_find_in_files(&mut self) {
        let dir = PathBuf::from(self.multifile_dir.trim());
        self.multifile_results = run_multifile_search(
            &self.multifile_query,
            self.find_case_sensitive,
            self.multifile_scope,
            &self.documents,
            &dir,
            &self.multifile_extensions,
        );
        self.show_bottom_panel = true;
        self.bottom_tab = BottomTab::SearchResults;
        self.status_message = format!(
            "Find in Files: {} match(es) for '{}'",
            self.multifile_results.len(),
            self.multifile_query
        );
    }

    fn action_jump_to_search_match(&mut self, m: &SearchMatch) {
        if let Some(tab_idx) = m.open_tab_idx {
            if tab_idx < self.documents.len() {
                self.active_tab = tab_idx;
            }
        } else if let Some(ref path) = m.file_path {
            if !self.open_or_activate_path(path.clone()) {
                return;
            }
        }

        let start = m.char_offset;
        let end = m.char_offset + m.match_char_len;
        if let Some(doc) = self.active_doc_mut() {
            doc.cursor_char_idx = end;
            doc.selection_char_range = Some((start, end));
        }
        self.pending_selection_override = Some((start, end));
        self.status_message = format!(
            "Jumped to {} — Ln {}, Col {}",
            m.file_label, m.line_number, m.col_number
        );
    }

    fn action_run_cargo(&mut self, subcmd: CargoSubcommand) {
        if self.cargo_running {
            self.status_message =
                "A Cargo command is already running in the background...".to_string();
            return;
        }

        self.action_save_all();

        let work_dir = PathBuf::from(self.cargo_working_dir.trim());
        if !work_dir.join("Cargo.toml").is_file() {
            if let Some(active_path) = self.active_doc().and_then(|d| d.path.clone()) {
                if let Some(root) = find_cargo_root(&active_path) {
                    self.cargo_working_dir = root.display().to_string();
                }
            }
        }

        let final_dir = PathBuf::from(self.cargo_working_dir.trim());
        self.cargo_diagnostics.clear();
        self.selected_diagnostic_idx = None;
        self.cargo_raw_log.clear();
        self.cargo_running = true;
        self.show_bottom_panel = true;
        self.bottom_tab = match subcmd {
            CargoSubcommand::Run | CargoSubcommand::Test => BottomTab::CargoRawLog,
            _ => BottomTab::CompilerDiagnostics,
        };

        self.status_message = format!("Running `{}` in {}...", subcmd.label(), final_dir.display());
        self.cargo_rx = Some(spawn_cargo_command(subcmd, final_dir));
    }

    fn poll_cargo_events(&mut self, ctx: &egui::Context) {
        let mut finished = false;
        if let Some(ref rx) = self.cargo_rx {
            while let Ok(ev) = rx.try_recv() {
                match ev {
                    CargoEvent::RawLine(line) => {
                        self.cargo_raw_log.push(line);
                    }
                    CargoEvent::Diagnostic(diag) => {
                        self.cargo_diagnostics.push(diag);
                    }
                    CargoEvent::Finished {
                        success,
                        status_text,
                    } => {
                        let err_count = self
                            .cargo_diagnostics
                            .iter()
                            .filter(|d| d.level == DiagnosticLevel::Error)
                            .count();
                        let warn_count = self
                            .cargo_diagnostics
                            .iter()
                            .filter(|d| d.level == DiagnosticLevel::Warning)
                            .count();
                        self.status_message = format!(
                            "{} in [{}] | {} error(s), {} warning(s) (Success: {})",
                            status_text, self.cargo_working_dir, err_count, warn_count, success
                        );
                        finished = true;
                    }
                }
            }
            ctx.request_repaint();
        }

        if finished {
            self.cargo_running = false;
            self.cargo_rx = None;
        }
    }

    fn action_jump_to_diagnostic(&mut self, diag: &CompilerDiagnostic) {
        if diag.file_path.exists() && !self.open_or_activate_path(diag.file_path.clone()) {
            return;
        }

        if let Some(doc) = self.active_doc_mut() {
            let target_line = diag.line_start.max(1);
            let target_col = diag.col_start.max(1);

            let mut cur_line = 1usize;
            let mut cur_col = 1usize;
            let mut target_char_idx = 0usize;

            for ch in doc.content.chars() {
                if cur_line == target_line && cur_col == target_col {
                    break;
                }
                target_char_idx += 1;
                if ch == '\n' {
                    if cur_line == target_line {
                        target_char_idx = target_char_idx.saturating_sub(1);
                        break;
                    }
                    cur_line += 1;
                    cur_col = 1;
                } else {
                    cur_col += 1;
                }
            }

            let span_char_len = match (diag.byte_start, diag.byte_end) {
                (Some(bs), Some(be)) if be > bs && be <= doc.content.len() => {
                    doc.content[bs..be].chars().count().max(1)
                }
                _ => 1,
            };

            let end_char = (target_char_idx + span_char_len).min(doc.content.chars().count());
            doc.cursor_char_idx = end_char;
            doc.selection_char_range = Some((target_char_idx, end_char));
            self.pending_selection_override = Some((target_char_idx, end_char));
        }

        self.status_message = format!(
            "Jumped to {}:{} — {}",
            diag.display_file, diag.line_start, diag.message
        );
    }

    fn action_goto_line(&mut self, target_line_1based: usize) {
        if let Some(doc) = self.active_doc_mut() {
            let mut current_line = 1usize;
            let mut char_idx = 0usize;
            for ch in doc.content.chars() {
                if current_line >= target_line_1based {
                    break;
                }
                char_idx += 1;
                if ch == '\n' {
                    current_line += 1;
                }
            }
            doc.cursor_char_idx = char_idx;
            self.pending_selection_override = Some((char_idx, char_idx));
            self.status_message = format!("Jumped to Line {}", current_line);
        }
    }

    fn handle_keyboard_shortcuts(&mut self, ctx: &egui::Context) {
        let sc_new = KeyboardShortcut::new(Modifiers::CTRL, Key::N);
        let sc_cargo_new = KeyboardShortcut::new(Modifiers::CTRL | Modifiers::SHIFT, Key::N);
        let sc_open = KeyboardShortcut::new(Modifiers::CTRL, Key::O);
        let sc_save = KeyboardShortcut::new(Modifiers::CTRL, Key::S);
        let sc_save_all = KeyboardShortcut::new(Modifiers::CTRL | Modifiers::SHIFT, Key::S);
        let sc_close = KeyboardShortcut::new(Modifiers::CTRL, Key::W);
        let sc_clip_j = KeyboardShortcut::new(Modifiers::CTRL, Key::J);
        let sc_clip_space = KeyboardShortcut::new(Modifiers::CTRL, Key::Space);
        let sc_member_dot = KeyboardShortcut::new(Modifiers::CTRL, Key::Period);
        let sc_member_alt_m = KeyboardShortcut::new(Modifiers::ALT, Key::M);
        let sc_f1_docs = KeyboardShortcut::new(Modifiers::NONE, Key::F1);
        let sc_comment = KeyboardShortcut::new(Modifiers::CTRL, Key::Slash);
        let sc_comment_pspad = KeyboardShortcut::new(Modifiers::CTRL | Modifiers::SHIFT, Key::A);

        let sc_find_in_files = KeyboardShortcut::new(Modifiers::CTRL | Modifiers::SHIFT, Key::F);
        let sc_find = KeyboardShortcut::new(Modifiers::CTRL, Key::F);
        let sc_replace = KeyboardShortcut::new(Modifiers::CTRL, Key::H);
        let sc_find_next = KeyboardShortcut::new(Modifiers::NONE, Key::F3);
        let sc_find_prev = KeyboardShortcut::new(Modifiers::SHIFT, Key::F3);
        let sc_goto_line = KeyboardShortcut::new(Modifiers::CTRL, Key::G);
        let sc_next_tab = KeyboardShortcut::new(Modifiers::CTRL, Key::Tab);

        let sc_cargo_check = KeyboardShortcut::new(Modifiers::NONE, Key::F9);
        let sc_cargo_build = KeyboardShortcut::new(Modifiers::CTRL, Key::F9);
        let sc_cargo_test = KeyboardShortcut::new(Modifiers::SHIFT, Key::F9);
        let sc_cargo_run = KeyboardShortcut::new(Modifiers::NONE, Key::F5);

        ctx.input_mut(|i| {
            if i.consume_shortcut(&sc_cargo_new) {
                self.cargo_new_error = None;
                self.show_cargo_new_modal = true;
            } else if i.consume_shortcut(&sc_new) {
                self.action_new_file();
            } else if i.consume_shortcut(&sc_open) {
                self.action_open_file();
            } else if i.consume_shortcut(&sc_save_all) {
                self.action_save_all();
            } else if i.consume_shortcut(&sc_save) {
                self.action_save_active();
            } else if i.consume_shortcut(&sc_close) {
                let idx = self.active_tab;
                self.action_close_tab(idx);
            } else if i.consume_shortcut(&sc_member_dot) || i.consume_shortcut(&sc_member_alt_m) {
                self.action_open_member_help();
            } else if i.consume_shortcut(&sc_f1_docs) {
                self.action_lookup_f1_docs();
            } else if i.consume_shortcut(&sc_clip_j) || i.consume_shortcut(&sc_clip_space) {
                // Smart routing: if the character before the cursor is `.`, open Member Function Help!
                let after_dot = self
                    .active_doc()
                    .map(|d| {
                        let b = char_to_byte_idx(&d.content, d.cursor_char_idx);
                        b > 0 && d.content.as_bytes()[b - 1] == b'.'
                    })
                    .unwrap_or(false);
                if after_dot {
                    self.action_open_member_help();
                } else {
                    self.action_expand_clip();
                }
            } else if i.consume_shortcut(&sc_comment) || i.consume_shortcut(&sc_comment_pspad) {
                self.action_toggle_comments();
            } else if i.consume_shortcut(&sc_find_in_files) {
                self.show_find_in_files_modal = true;
                if self.multifile_query.is_empty() && !self.find_query.is_empty() {
                    self.multifile_query = self.find_query.clone();
                }
            } else if i.consume_shortcut(&sc_find) || i.consume_shortcut(&sc_replace) {
                self.show_find_bar = true;
                self.focus_find_input = true;
            } else if i.consume_shortcut(&sc_find_prev) {
                self.action_find_step(true);
            } else if i.consume_shortcut(&sc_find_next) {
                self.action_find_step(false);
            } else if i.consume_shortcut(&sc_goto_line) {
                self.show_goto_line_modal = true;
            } else if i.consume_shortcut(&sc_next_tab) {
                if !self.documents.is_empty() {
                    self.active_tab = (self.active_tab + 1) % self.documents.len();
                    self.save_session();
                }
            } else if i.consume_shortcut(&sc_cargo_build) {
                self.action_run_cargo(CargoSubcommand::Build);
            } else if i.consume_shortcut(&sc_cargo_test) {
                self.action_run_cargo(CargoSubcommand::Test);
            } else if i.consume_shortcut(&sc_cargo_check) {
                self.action_run_cargo(CargoSubcommand::Check);
            } else if i.consume_shortcut(&sc_cargo_run) {
                self.action_run_cargo(CargoSubcommand::Run);
            }
        });
    }
}

impl eframe::App for RustPadApp {
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // Persist session state automatically whenever the window closes
        self.save_session();
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.dark_mode {
            ctx.set_visuals(egui::Visuals::dark());
        } else {
            ctx.set_visuals(egui::Visuals::light());
        }

        self.poll_cargo_events(ctx);
        self.handle_keyboard_shortcuts(ctx);

        // =========================================================================
        // Modal 1: `cargo new` Project Creator Dialog (Ctrl+Shift+N)
        // =========================================================================
        if self.show_cargo_new_modal {
            let mut open = self.show_cargo_new_modal;
            let mut run_create = false;

            egui::Window::new("New Cargo Project (`cargo new`)")
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .default_width(520.0)
                .show(ctx, |ui| {
                    ui.label("Create a new Rust crate with `cargo new` and open it immediately in RustPad:");
                    ui.add_space(6.0);

                    egui::Grid::new("cargo_new_grid")
                        .num_columns(2)
                        .spacing([10.0, 8.0])
                        .show(ui, |ui| {
                            ui.label("Parent Folder:");
                            ui.horizontal(|ui| {
                                ui.add(
                                    egui::TextEdit::singleline(&mut self.cargo_new_parent_dir)
                                        .desired_width(300.0),
                                );
                                if ui.button("Browse...").clicked() {
                                    if let Some(folder) = rfd::FileDialog::new()
                                        .set_title("Select Parent Folder for New Cargo Project")
                                        .pick_folder()
                                    {
                                        self.cargo_new_parent_dir = folder.display().to_string();
                                    }
                                }
                            });
                            ui.end_row();

                            ui.label("Crate Name:");
                            let resp = ui.add(
                                egui::TextEdit::singleline(&mut self.cargo_new_crate_name)
                                    .desired_width(300.0)
                                    .hint_text("e.g., my_simulation"),
                            );
                            if resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                                run_create = true;
                            }
                            ui.end_row();

                            ui.label("Template:");
                            ui.horizontal(|ui| {
                                ui.radio_value(
                                    &mut self.cargo_new_is_lib,
                                    false,
                                    "Binary Application (--bin -> src/main.rs)",
                                );
                                ui.radio_value(
                                    &mut self.cargo_new_is_lib,
                                    true,
                                    "Library Crate (--lib -> src/lib.rs)",
                                );
                            });
                            ui.end_row();
                        });

                    let preview_path = Path::new(self.cargo_new_parent_dir.trim())
                        .join(self.cargo_new_crate_name.trim());
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(format!("Target path: {}", preview_path.display()))
                            .monospace()
                            .small(),
                    );

                    if let Some(ref err) = self.cargo_new_error {
                        ui.colored_label(Color32::from_rgb(255, 110, 110), err);
                    }

                    ui.separator();
                    ui.horizontal(|ui| {
                        if ui.button("Create Project").clicked() {
                            run_create = true;
                        }
                        if ui.button("Cancel").clicked() {
                            self.show_cargo_new_modal = false;
                        }
                    });
                });

            if !open {
                self.show_cargo_new_modal = false;
            }
            if run_create {
                self.action_execute_cargo_new();
            }
        }

        // =========================================================================
        // Modal 2: Option A + F1 Point-and-Shoot Member Function Browser
        // =========================================================================
        if self.show_member_help_modal {
            let mut open = self.show_member_help_modal;
            let mut entry_to_shoot: Option<MethodEntry> = None;
            let mut doc_query_to_open: Option<String> = None;

            let all_methods = methods_for_category(self.member_category, &self.documents);
            let filter_lower = self.member_filter.trim().to_lowercase();
            let filtered_methods: Vec<MethodEntry> = all_methods
                .into_iter()
                .filter(|m| {
                    filter_lower.is_empty()
                        || m.name.to_lowercase().contains(&filter_lower)
                        || m.description.to_lowercase().contains(&filter_lower)
                })
                .collect();

            if self.member_selected_idx >= filtered_methods.len() {
                self.member_selected_idx = 0;
            }

            egui::Window::new("Point & Shoot Member Functions (Ctrl+. / Alt+M)")
                .open(&mut open)
                .collapsible(false)
                .resizable(true)
                .default_width(720.0)
                .default_height(430.0)
                .show(ctx, |ui| {
                    // Header showing heuristic detection result
                    ui.horizontal(|ui| {
                        if let Some(ref recv) = self.member_receiver_ident {
                            ui.label(
                                RichText::new(format!("Receiver: `{}`", recv))
                                    .monospace()
                                    .strong()
                                    .color(Color32::from_rgb(120, 210, 255)),
                            );
                        } else {
                            ui.label(RichText::new("Receiver: (cursor)").monospace());
                        }

                        if let Some(ref inferred) = self.member_inferred_type {
                            ui.colored_label(
                                Color32::from_rgb(100, 220, 140),
                                format!("✔ Auto-detected type: {}", inferred),
                            );
                        } else {
                            ui.label(
                                RichText::new("(Select a type tab below or filter by method name)")
                                    .small(),
                            );
                        }

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui
                                .button("📘 Open Type in Rust Docs (F1)")
                                .on_hover_text("Runs `rustup doc --std` for the selected type")
                                .clicked()
                            {
                                doc_query_to_open =
                                    Some(self.member_category.std_doc_keyword().to_string());
                            }
                        });
                    });

                    ui.separator();

                    // Category Tabs
                    ui.horizontal_wrapped(|ui| {
                        for &cat in TypeCategory::ALL {
                            if ui
                                .selectable_label(self.member_category == cat, cat.label())
                                .clicked()
                            {
                                self.member_category = cat;
                                self.member_selected_idx = 0;
                            }
                        }
                    });

                    ui.separator();

                    // Filter Input Bar + Keyboard Navigation (Up / Down / Enter / Escape)
                    ui.horizontal(|ui| {
                        ui.label("Filter method:");
                        let filter_resp = ui.add(
                            egui::TextEdit::singleline(&mut self.member_filter)
                                .desired_width(260.0)
                                .hint_text("Type e.g. len, split, iter, unwrap..."),
                        );
                        if self.focus_member_filter {
                            filter_resp.request_focus();
                            self.focus_member_filter = false;
                        }
                        if ui.button("Clear").clicked() {
                            self.member_filter.clear();
                        }
                        ui.label(
                            RichText::new("Tip: Click any row or press Enter to shoot into code!")
                                .small()
                                .color(Color32::from_gray(160)),
                        );
                    });

                    // Handle Up/Down/Enter/Escape keys inside the modal
                    if ui.input(|i| i.key_pressed(Key::ArrowDown)) && !filtered_methods.is_empty() {
                        self.member_selected_idx =
                            (self.member_selected_idx + 1) % filtered_methods.len();
                    }
                    if ui.input(|i| i.key_pressed(Key::ArrowUp)) && !filtered_methods.is_empty() {
                        self.member_selected_idx = self
                            .member_selected_idx
                            .checked_sub(1)
                            .unwrap_or(filtered_methods.len() - 1);
                    }
                    if ui.input(|i| i.key_pressed(Key::Enter)) {
                        if let Some(chosen) = filtered_methods.get(self.member_selected_idx) {
                            entry_to_shoot = Some(chosen.clone());
                        }
                    }
                    if ui.input(|i| i.key_pressed(Key::Escape)) {
                        self.show_member_help_modal = false;
                    }

                    ui.separator();

                    // Scrollable Method Table
                    egui::ScrollArea::vertical()
                        .id_salt("member_methods_scroll")
                        .max_height(280.0)
                        .show(ui, |ui| {
                            if filtered_methods.is_empty() {
                                ui.label("No matching methods in this category. Try another type tab above or clear the filter.");
                            }

                            egui::Grid::new("member_methods_grid")
                                .striped(true)
                                .num_columns(4)
                                .spacing([12.0, 6.0])
                                .show(ui, |ui| {
                                    for (idx, m) in filtered_methods.iter().enumerate() {
                                        let is_selected = idx == self.member_selected_idx;

                                        // Column 1: Shoot button / Method Name
                                        let btn_label = RichText::new(format!(".{}", m.insert_template.replace('|', "")))
                                            .monospace()
                                            .strong()
                                            .color(if is_selected {
                                                Color32::from_rgb(255, 220, 100)
                                            } else {
                                                Color32::from_rgb(130, 200, 255)
                                            });

                                        if ui
                                            .selectable_label(is_selected, btn_label)
                                            .on_hover_text("Click to insert at cursor")
                                            .clicked()
                                        {
                                            entry_to_shoot = Some(m.clone());
                                        }

                                        // Column 2: Owner Type badge
                                        ui.label(
                                            RichText::new(&m.owner_type)
                                                .monospace()
                                                .small()
                                                .color(Color32::from_gray(155)),
                                        );

                                        // Column 3: Signature
                                        ui.label(RichText::new(&m.signature).monospace().small());

                                        // Column 4: Description + F1 Doc button
                                        ui.horizontal(|ui| {
                                            ui.label(RichText::new(&m.description).small());
                                            if ui
                                                .small_button("F1")
                                                .on_hover_text("Look up in offline Rust docs")
                                                .clicked()
                                            {
                                                doc_query_to_open = Some(format!(
                                                    "{}::{}",
                                                    self.member_category.std_doc_keyword(),
                                                    m.name
                                                ));
                                            }
                                        });

                                        ui.end_row();
                                    }
                                });
                        });
                });

            if !open {
                self.show_member_help_modal = false;
            }
            if let Some(entry) = entry_to_shoot {
                self.action_shoot_member_entry(&entry);
            }
            if let Some(q) = doc_query_to_open {
                self.status_message = open_offline_rust_docs(&q);
            }
        }

        // Modal 3: Set Cargo Project Folder Dialog
        if self.show_project_folder_modal {
            let mut open = self.show_project_folder_modal;
            egui::Window::new("Set Cargo Project Folder")
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .default_width(520.0)
                .show(ctx, |ui| {
                    ui.label("Specify the directory where `cargo check`, `cargo build`, `cargo test`, and `cargo run` execute:");
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.label("Project Folder:");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.cargo_working_dir)
                                .desired_width(310.0),
                        );
                        if ui.button("Browse...").clicked() {
                            self.action_browse_project_folder();
                        }
                    });

                    ui.add_space(4.0);
                    if self.has_cargo_toml() {
                        ui.colored_label(
                            Color32::from_rgb(80, 200, 120),
                            "✔ Valid Cargo project (Cargo.toml found in this folder).",
                        );
                    } else {
                        ui.colored_label(
                            Color32::from_rgb(255, 170, 60),
                            "⚠ No Cargo.toml found in this folder. Cargo commands require a folder with Cargo.toml.",
                        );
                    }

                    ui.separator();
                    ui.horizontal(|ui| {
                        if ui.button("Use Active File's Cargo Root").clicked() {
                            self.action_detect_project_from_active_file();
                        }
                        if ui.button("Done").clicked() {
                            self.save_session();
                            self.show_project_folder_modal = false;
                        }
                    });
                });
            if !open {
                self.show_project_folder_modal = false;
            }
        }

        // Modal 4: Find in Files Dialog (Ctrl+Shift+F)
        if self.show_find_in_files_modal {
            let mut open = self.show_find_in_files_modal;
            let mut trigger_search = false;

            egui::Window::new("Find in Files (Ctrl+Shift+F)")
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .default_width(480.0)
                .show(ctx, |ui| {
                    egui::Grid::new("find_in_files_grid")
                        .num_columns(2)
                        .spacing([10.0, 8.0])
                        .show(ui, |ui| {
                            ui.label("Find text:");
                            let resp = ui.add(
                                egui::TextEdit::singleline(&mut self.multifile_query)
                                    .desired_width(340.0),
                            );
                            if resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                                trigger_search = true;
                            }
                            ui.end_row();

                            ui.label("Scope:");
                            ui.horizontal(|ui| {
                                ui.radio_value(
                                    &mut self.multifile_scope,
                                    SearchScope::Directory,
                                    "Directory Tree",
                                );
                                ui.radio_value(
                                    &mut self.multifile_scope,
                                    SearchScope::OpenFiles,
                                    "All Open Tabs",
                                );
                            });
                            ui.end_row();

                            if self.multifile_scope == SearchScope::Directory {
                                ui.label("Folder:");
                                ui.horizontal(|ui| {
                                    ui.add(
                                        egui::TextEdit::singleline(&mut self.multifile_dir)
                                            .desired_width(265.0),
                                    );
                                    if ui.button("Browse...").clicked() {
                                        if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                                            self.multifile_dir = folder.display().to_string();
                                        }
                                    }
                                });
                                ui.end_row();

                                ui.label("Extensions:");
                                ui.add(
                                    egui::TextEdit::singleline(&mut self.multifile_extensions)
                                        .desired_width(340.0),
                                );
                                ui.end_row();
                            }

                            ui.label("Options:");
                            ui.checkbox(&mut self.find_case_sensitive, "Case sensitive");
                            ui.end_row();
                        });

                    ui.separator();
                    ui.horizontal(|ui| {
                        if ui.button("Search Now").clicked() {
                            trigger_search = true;
                        }
                        if ui.button("Close").clicked() {
                            self.show_find_in_files_modal = false;
                        }
                    });
                });

            if !open {
                self.show_find_in_files_modal = false;
            }
            if trigger_search {
                self.action_run_find_in_files();
                self.show_find_in_files_modal = false;
            }
        }

        // Modal 5: Go To Line (Ctrl+G)
        if self.show_goto_line_modal {
            let mut open = self.show_goto_line_modal;
            let mut jump_now = false;
            egui::Window::new("Go to Line (Ctrl+G)")
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Line number:");
                        let resp = ui.text_edit_singleline(&mut self.goto_line_input);
                        if resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                            jump_now = true;
                        }
                        if ui.button("Go").clicked() {
                            jump_now = true;
                        }
                    });
                });
            if !open {
                self.show_goto_line_modal = false;
            }
            if jump_now {
                if let Ok(line_num) = self.goto_line_input.trim().parse::<usize>() {
                    self.action_goto_line(line_num.max(1));
                }
                self.show_goto_line_modal = false;
            }
        }

        // =========================================================================
        // 1. Standard Windows App Top Menu Bar: File | Edit | Build | Misc
        // =========================================================================
        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                // --- FILE MENU ---
                ui.menu_button("File", |ui| {
                    if ui.button("New File (Ctrl+N)").clicked() {
                        self.action_new_file();
                        ui.close_menu();
                    }
                    if ui
                        .button("New Cargo Project (`cargo new`)... (Ctrl+Shift+N)")
                        .clicked()
                    {
                        self.cargo_new_error = None;
                        self.show_cargo_new_modal = true;
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Open File... (Ctrl+O)").clicked() {
                        self.action_open_file();
                        ui.close_menu();
                    }
                    if ui.button("Open Cargo Project Folder...").clicked() {
                        self.action_browse_project_folder();
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Save (Ctrl+S)").clicked() {
                        self.action_save_active();
                        ui.close_menu();
                    }
                    if ui.button("Save As...").clicked() {
                        self.action_save_as_active();
                        ui.close_menu();
                    }
                    if ui.button("Save All (Ctrl+Shift+S)").clicked() {
                        self.action_save_all();
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Close Active Tab (Ctrl+W)").clicked() {
                        let idx = self.active_tab;
                        self.action_close_tab(idx);
                        ui.close_menu();
                    }
                    if ui.button("Next Tab (Ctrl+Tab)").clicked() {
                        if !self.documents.is_empty() {
                            self.active_tab = (self.active_tab + 1) % self.documents.len();
                            self.save_session();
                        }
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Save Session & Exit").clicked() {
                        self.save_session();
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });

                // --- EDIT MENU ---
                ui.menu_button("Edit", |ui| {
                    if ui.button("Undo (Ctrl+Z)").clicked() {
                        if let Some(doc) = self.active_doc_mut() {
                            doc.undo();
                        }
                        ui.close_menu();
                    }
                    if ui.button("Redo (Ctrl+Y)").clicked() {
                        if let Some(doc) = self.active_doc_mut() {
                            doc.redo();
                        }
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui
                        .button("Point & Shoot Member Functions... (Ctrl+. / Alt+M)")
                        .clicked()
                    {
                        self.action_open_member_help();
                        ui.close_menu();
                    }
                    if ui
                        .button("Look Up Symbol at Cursor in Rust Docs (F1)")
                        .clicked()
                    {
                        self.action_lookup_f1_docs();
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui
                        .button("Find / Replace in File... (Ctrl+F / Ctrl+H)")
                        .clicked()
                    {
                        self.show_find_bar = true;
                        self.focus_find_input = true;
                        ui.close_menu();
                    }
                    if ui.button("Find Next (F3)").clicked() {
                        self.action_find_step(false);
                        ui.close_menu();
                    }
                    if ui.button("Find Previous (Shift+F3)").clicked() {
                        self.action_find_step(true);
                        ui.close_menu();
                    }
                    if ui.button("Find in Files... (Ctrl+Shift+F)").clicked() {
                        self.show_find_in_files_modal = true;
                        if self.multifile_query.is_empty() && !self.find_query.is_empty() {
                            self.multifile_query = self.find_query.clone();
                        }
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Go to Line... (Ctrl+G)").clicked() {
                        self.show_goto_line_modal = true;
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui
                        .button("Toggle Line Comment // (Ctrl+/ or Ctrl+Shift+A)")
                        .clicked()
                    {
                        self.action_toggle_comments();
                        ui.close_menu();
                    }
                    if ui
                        .button("Expand Clip Template at Cursor (Ctrl+J / Ctrl+Space)")
                        .clicked()
                    {
                        self.action_expand_clip();
                        ui.close_menu();
                    }
                });

                // --- BUILD MENU ---
                ui.menu_button("Build", |ui| {
                    ui.add_enabled_ui(!self.cargo_running, |ui| {
                        if ui.button("Cargo Check (F9)").clicked() {
                            self.action_run_cargo(CargoSubcommand::Check);
                            ui.close_menu();
                        }
                        if ui.button("Cargo Build (Ctrl+F9)").clicked() {
                            self.action_run_cargo(CargoSubcommand::Build);
                            ui.close_menu();
                        }
                        if ui.button("Cargo Clippy (Lint)").clicked() {
                            self.action_run_cargo(CargoSubcommand::Clippy);
                            ui.close_menu();
                        }
                        if ui.button("Cargo Test (Shift+F9)").clicked() {
                            self.action_run_cargo(CargoSubcommand::Test);
                            ui.close_menu();
                        }
                        if ui.button("Cargo Run (F5)").clicked() {
                            self.action_run_cargo(CargoSubcommand::Run);
                            ui.close_menu();
                        }
                    });
                    ui.separator();
                    if ui
                        .button("New Cargo Project (`cargo new`)... (Ctrl+Shift+N)")
                        .clicked()
                    {
                        self.cargo_new_error = None;
                        self.show_cargo_new_modal = true;
                        ui.close_menu();
                    }
                    if ui.button("Set Cargo Project Folder...").clicked() {
                        self.show_project_folder_modal = true;
                        ui.close_menu();
                    }
                    if ui.button("Browse for Project Folder...").clicked() {
                        self.action_browse_project_folder();
                        ui.close_menu();
                    }
                    if ui
                        .button("Detect Project Folder from Active File")
                        .clicked()
                    {
                        self.action_detect_project_from_active_file();
                        ui.close_menu();
                    }
                    ui.separator();
                    ui.checkbox(
                        &mut self.show_bottom_panel,
                        "Show Build / Compiler Output Panel",
                    );
                });

                // --- MISC MENU ---
                ui.menu_button("Misc", |ui| {
                    if ui
                        .button("Point & Shoot Member Functions... (Ctrl+. / Alt+M)")
                        .clicked()
                    {
                        self.action_open_member_help();
                        ui.close_menu();
                    }
                    if ui
                        .button("Offline Rust Stdlib Docs at Cursor (F1)")
                        .clicked()
                    {
                        self.action_lookup_f1_docs();
                        ui.close_menu();
                    }
                    ui.separator();
                    ui.menu_button("Insert Rust Clip Template", |ui| {
                        for clip in BUILTIN_TEMPLATES {
                            let label = format!("{:<6} — {}", clip.trigger, clip.description);
                            if ui.button(label).clicked() {
                                if let Some(doc) = self.active_doc_mut() {
                                    let snap = doc.content.clone();
                                    doc.push_undo_snapshot(snap);
                                    let new_cursor = insert_template_at_cursor(
                                        &mut doc.content,
                                        doc.cursor_char_idx,
                                        clip,
                                    );
                                    doc.cursor_char_idx = new_cursor;
                                    doc.is_dirty = true;
                                    self.pending_selection_override =
                                        Some((new_cursor, new_cursor));
                                    self.status_message =
                                        format!("Inserted clip '{}'", clip.trigger);
                                }
                                ui.close_menu();
                            }
                        }
                    });
                    ui.checkbox(&mut self.show_clips_panel, "Show Clips Reference Sidebar");
                    ui.checkbox(
                        &mut self.show_bottom_panel,
                        "Show Bottom Output / Search Panel",
                    );
                    ui.separator();
                    if ui
                        .checkbox(
                            &mut self.dark_mode,
                            "Dark Syntax Theme (Uncheck for PSPad Light)",
                        )
                        .changed()
                    {
                        self.save_session();
                    }
                    ui.separator();
                    if ui.button("Zoom In Font (+)").clicked() {
                        self.font_size = (self.font_size + 1.0).min(28.0);
                        self.save_session();
                        ui.close_menu();
                    }
                    if ui.button("Zoom Out Font (-)").clicked() {
                        self.font_size = (self.font_size - 1.0).max(9.0);
                        self.save_session();
                        ui.close_menu();
                    }
                    if ui.button("Reset Font Size (14.5)").clicked() {
                        self.font_size = 14.5;
                        self.save_session();
                        ui.close_menu();
                    }
                });

                // Quick-Access Member Help & F1 buttons + Project Folder Bar
                ui.separator();
                if ui
                    .button("⚡ .methods (Ctrl+.)")
                    .on_hover_text("Open Point & Shoot Member Function Browser (Ctrl+. or Alt+M)")
                    .clicked()
                {
                    self.action_open_member_help();
                }
                if ui
                    .button("📘 F1 Doc")
                    .on_hover_text("Look up word at cursor in offline Rust docs (F1)")
                    .clicked()
                {
                    self.action_lookup_f1_docs();
                }

                ui.separator();
                let has_toml = self.has_cargo_toml();
                let folder_badge = if has_toml {
                    RichText::new("Cargo Dir:")
                        .color(Color32::from_rgb(100, 210, 130))
                        .strong()
                } else {
                    RichText::new("Dir (No Cargo.toml):")
                        .color(Color32::from_rgb(255, 175, 70))
                        .strong()
                };
                ui.label(folder_badge);
                if ui
                    .add(
                        egui::TextEdit::singleline(&mut self.cargo_working_dir)
                            .desired_width(220.0)
                            .hint_text("Path to folder containing Cargo.toml"),
                    )
                    .changed()
                {
                    self.save_session();
                }
                if ui
                    .button("Browse...")
                    .on_hover_text("Select folder where Cargo builds run")
                    .clicked()
                {
                    self.action_browse_project_folder();
                }
                if ui
                    .button("cargo new")
                    .on_hover_text("Create a new Cargo project (Ctrl+Shift+N)")
                    .clicked()
                {
                    self.cargo_new_error = None;
                    self.show_cargo_new_modal = true;
                }
                if self.cargo_running {
                    ui.spinner();
                    ui.label("Building...");
                }
            });

            // Quick Find/Replace Bar
            if self.show_find_bar {
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label("Find:");
                    let find_resp = ui
                        .add(egui::TextEdit::singleline(&mut self.find_query).desired_width(180.0));
                    if self.focus_find_input {
                        find_resp.request_focus();
                        self.focus_find_input = false;
                    }
                    if find_resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                        self.action_find_step(false);
                    }
                    if ui.button("Next (F3)").clicked() {
                        self.action_find_step(false);
                    }
                    if ui.button("Prev (Shift+F3)").clicked() {
                        self.action_find_step(true);
                    }
                    ui.checkbox(&mut self.find_case_sensitive, "Match case");
                    ui.separator();
                    ui.label("Replace:");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.replace_query).desired_width(150.0),
                    );
                    if ui.button("Replace All").clicked() {
                        let q = self.find_query.clone();
                        let r = self.replace_query.clone();
                        if !q.is_empty() {
                            if let Some(doc) = self.active_doc_mut() {
                                let count = doc.content.matches(&q).count();
                                if count > 0 {
                                    let snap = doc.content.clone();
                                    doc.push_undo_snapshot(snap);
                                    doc.content = doc.content.replace(&q, &r);
                                    doc.is_dirty = true;
                                }
                                self.status_message =
                                    format!("Replaced {} occurrence(s) of '{}'.", count, q);
                            }
                        }
                    }
                    if ui.button("Close (X)").clicked() {
                        self.show_find_bar = false;
                    }
                });
            }

            ui.separator();

            // Multi-file Tab Bar
            ui.horizontal_wrapped(|ui| {
                let mut tab_to_close = None;
                let mut tab_to_activate = None;

                for (idx, doc) in self.documents.iter().enumerate() {
                    let is_active = idx == self.active_tab;
                    let dirty_mark = if doc.is_dirty { " *" } else { "" };
                    let label = format!("{}{}", doc.title, dirty_mark);

                    ui.horizontal(|ui| {
                        let btn =
                            egui::SelectableLabel::new(is_active, RichText::new(label).strong());
                        let resp = ui.add(btn);
                        if let Some(ref p) = doc.path {
                            resp.clone().on_hover_text(p.display().to_string());
                        }
                        if resp.clicked() {
                            tab_to_activate = Some(idx);
                        }
                        if ui
                            .small_button("x")
                            .on_hover_text("Close Tab (Ctrl+W)")
                            .clicked()
                        {
                            tab_to_close = Some(idx);
                        }
                    });
                    ui.separator();
                }

                if let Some(idx) = tab_to_activate {
                    self.active_tab = idx;
                    self.save_session();
                }
                if let Some(idx) = tab_to_close {
                    self.action_close_tab(idx);
                }
            });
        });

        // 2. Optional Right Sidebar: PSPad-style Clips / Template Palette
        if self.show_clips_panel {
            egui::SidePanel::right("clips_sidebar")
                .resizable(true)
                .default_width(235.0)
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.heading("Rust Clips (Ctrl+J)");
                        if ui.small_button("X").clicked() {
                            self.show_clips_panel = false;
                        }
                    });
                    ui.label(
                        RichText::new("Type a trigger in the editor and press Ctrl+J, or click any clip below:")
                            .small(),
                    );
                    ui.separator();

                    egui::ScrollArea::vertical().show(ui, |ui| {
                        for clip in BUILTIN_TEMPLATES {
                            ui.horizontal(|ui| {
                                if ui
                                    .button(RichText::new(format!("{:<6}", clip.trigger)).monospace().strong())
                                    .on_hover_text(clip.body)
                                    .clicked()
                                {
                                    if let Some(doc) = self.active_doc_mut() {
                                        let snap = doc.content.clone();
                                        doc.push_undo_snapshot(snap);
                                        let new_cursor = insert_template_at_cursor(
                                            &mut doc.content,
                                            doc.cursor_char_idx,
                                            clip,
                                        );
                                        doc.cursor_char_idx = new_cursor;
                                        doc.is_dirty = true;
                                        self.pending_selection_override = Some((new_cursor, new_cursor));
                                        self.status_message = format!("Inserted clip '{}'", clip.trigger);
                                    }
                                }
                                ui.label(clip.description);
                            });
                        }
                    });
                });
        }

        let active_brace_match: Option<BraceMatch> = self
            .active_doc()
            .and_then(|doc| find_matching_brace(&doc.content, doc.cursor_char_idx));

        // 3. Bottom Status Bar
        egui::TopBottomPanel::bottom("status_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if let Some(doc) = self.active_doc() {
                    let cursor_byte = char_to_byte_idx(&doc.content, doc.cursor_char_idx);
                    let (line, col) = byte_to_line_col(&doc.content, cursor_byte);
                    let total_lines = doc.content.lines().count().max(1);

                    ui.label(
                        RichText::new(format!(
                            "Ln {}, Col {}  (Lines: {})",
                            line, col, total_lines
                        ))
                        .monospace(),
                    );
                    ui.separator();

                    if let Some(bm) = active_brace_match {
                        let (open_l, open_c) = byte_to_line_col(&doc.content, bm.open_byte);
                        let (close_l, close_c) = byte_to_line_col(&doc.content, bm.close_byte);
                        ui.colored_label(
                            Color32::from_rgb(255, 195, 0),
                            format!(
                                "Matched '{}' (Ln {}, Col {}) <-> '{}' (Ln {}, Col {})",
                                bm.open_char, open_l, open_c, bm.close_char, close_l, close_c
                            ),
                        );
                    } else {
                        ui.label("Brace Match: —");
                    }

                    ui.separator();
                    ui.label(&self.status_message);
                }
            });
        });

        // 4. Collapsible Bottom PSPad-Style Log Panel
        if self.show_bottom_panel {
            egui::TopBottomPanel::bottom("bottom_dock_panel")
                .resizable(true)
                .default_height(210.0)
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        let err_count = self
                            .cargo_diagnostics
                            .iter()
                            .filter(|d| d.level == DiagnosticLevel::Error)
                            .count();
                        let warn_count = self
                            .cargo_diagnostics
                            .iter()
                            .filter(|d| d.level == DiagnosticLevel::Warning)
                            .count();

                        ui.selectable_value(
                            &mut self.bottom_tab,
                            BottomTab::CompilerDiagnostics,
                            format!("Compiler Errors ({} E / {} W)", err_count, warn_count),
                        );
                        ui.selectable_value(
                            &mut self.bottom_tab,
                            BottomTab::CargoRawLog,
                            format!("Cargo Output ({} lines)", self.cargo_raw_log.len()),
                        );
                        ui.selectable_value(
                            &mut self.bottom_tab,
                            BottomTab::SearchResults,
                            format!("Search Results ({})", self.multifile_results.len()),
                        );

                        ui.separator();
                        ui.label(
                            RichText::new(format!("Building in: {}", self.cargo_working_dir))
                                .monospace()
                                .small(),
                        );
                        if ui.small_button("Change Folder...").clicked() {
                            self.show_project_folder_modal = true;
                        }
                        if ui.small_button("Hide Panel").clicked() {
                            self.show_bottom_panel = false;
                        }
                    });
                    ui.separator();

                    match self.bottom_tab {
                        BottomTab::CompilerDiagnostics => {
                            let mut clicked_diag: Option<(usize, CompilerDiagnostic)> = None;

                            ui.columns(2, |cols| {
                                cols[0].label(RichText::new("Click any diagnostic to open file & jump to line:").small());
                                egui::ScrollArea::vertical()
                                    .id_salt("diag_list_scroll")
                                    .show(&mut cols[0], |ui| {
                                        if self.cargo_diagnostics.is_empty() {
                                            ui.label("No compiler diagnostics. Use Build -> Cargo Check (F9) or Cargo Build (Ctrl+F9).");
                                        }
                                        for (idx, diag) in self.cargo_diagnostics.iter().enumerate() {
                                            let (badge, color) = match diag.level {
                                                DiagnosticLevel::Error => ("ERROR", Color32::from_rgb(255, 100, 100)),
                                                DiagnosticLevel::Warning => ("WARN ", Color32::from_rgb(255, 200, 70)),
                                                DiagnosticLevel::Note => ("NOTE ", Color32::from_rgb(130, 190, 255)),
                                            };
                                            let code_str = diag
                                                .code
                                                .as_deref()
                                                .map(|c| format!("[{}] ", c))
                                                .unwrap_or_default();
                                            let label_text = format!(
                                                "{} {}{}:{}:{} — {}",
                                                badge,
                                                code_str,
                                                diag.display_file,
                                                diag.line_start,
                                                diag.col_start,
                                                diag.message
                                            );
                                            let is_sel = self.selected_diagnostic_idx == Some(idx);
                                            if ui
                                                .selectable_label(
                                                    is_sel,
                                                    RichText::new(label_text).monospace().color(color),
                                                )
                                                .clicked()
                                            {
                                                clicked_diag = Some((idx, diag.clone()));
                                            }
                                        }
                                    });

                                cols[1].label(RichText::new("Full Rustc Explanation:").small());
                                egui::ScrollArea::vertical()
                                    .id_salt("diag_rendered_scroll")
                                    .show(&mut cols[1], |ui| {
                                        if let Some(sel_idx) = self.selected_diagnostic_idx {
                                            if let Some(d) = self.cargo_diagnostics.get(sel_idx) {
                                                ui.add(
                                                    egui::Label::new(
                                                        RichText::new(&d.rendered).monospace(),
                                                    )
                                                    .selectable(true),
                                                );
                                            }
                                        } else {
                                            ui.label("Select an error or warning on the left to inspect its borrow-checker / compiler explanation.");
                                        }
                                    });
                            });

                            if let Some((idx, diag)) = clicked_diag {
                                self.selected_diagnostic_idx = Some(idx);
                                self.action_jump_to_diagnostic(&diag);
                            }
                        }
                        BottomTab::CargoRawLog => {
                            egui::ScrollArea::vertical()
                                .id_salt("cargo_raw_scroll")
                                .stick_to_bottom(true)
                                .show(ui, |ui| {
                                    let joined = self.cargo_raw_log.join("\n");
                                    ui.add(
                                        egui::Label::new(RichText::new(joined).monospace())
                                            .selectable(true),
                                    );
                                });
                        }
                        BottomTab::SearchResults => {
                            let mut clicked_match: Option<SearchMatch> = None;
                            egui::ScrollArea::vertical()
                                .id_salt("search_results_scroll")
                                .show(ui, |ui| {
                                    for m in &self.multifile_results {
                                        let row_text = format!(
                                            "{}:{}:{}   {}",
                                            m.file_label, m.line_number, m.col_number, m.line_preview
                                        );
                                        if ui
                                            .selectable_label(false, RichText::new(row_text).monospace())
                                            .clicked()
                                        {
                                            clicked_match = Some(m.clone());
                                        }
                                    }
                                });

                            if let Some(m) = clicked_match {
                                self.action_jump_to_search_match(&m);
                            }
                        }
                    }
                });
        }

        // 5. Central Code Editor Pane
        egui::CentralPanel::default().show(ctx, |ui| {
            let font_size = self.font_size;
            let dark_mode = self.dark_mode;
            let pending_selection = self.pending_selection_override.take();

            let Some(doc) = self.documents.get_mut(self.active_tab) else {
                return;
            };

            let font_id = FontId::new(font_size, FontFamily::Monospace);
            let line_count = doc.content.split('\n').count().max(1);
            let gutter_string: String = (1..=line_count)
                .map(|n| format!("{:>4}", n))
                .collect::<Vec<_>>()
                .join("\n");

            egui::ScrollArea::both()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.horizontal_top(|ui| {
                        ui.add(
                            egui::Label::new(
                                RichText::new(gutter_string)
                                    .font(font_id.clone())
                                    .color(Color32::from_gray(120)),
                            )
                            .selectable(false),
                        );

                        ui.separator();

                        let mut layouter = |ui: &egui::Ui, string: &str, wrap_width: f32| {
                            let mut layout_job = highlight_rust_code(
                                string,
                                font_id.clone(),
                                active_brace_match,
                                dark_mode,
                            );
                            layout_job.wrap.max_width = wrap_width;
                            ui.fonts(|f| f.layout_job(layout_job))
                        };

                        let pre_edit_snapshot = doc.content.clone();

                        let mut output: TextEditOutput =
                            egui::TextEdit::multiline(&mut doc.content)
                                .id(egui::Id::new(("rustpad_editor", self.active_tab)))
                                .font(font_id.clone())
                                .code_editor()
                                .lock_focus(true)
                                .desired_width(f32::INFINITY)
                                .desired_rows(40)
                                .layouter(&mut layouter)
                                .show(ui);

                        if output.response.changed() {
                            doc.push_undo_snapshot(pre_edit_snapshot);
                            doc.is_dirty = true;
                        }

                        if let Some((start_char, end_char)) = pending_selection {
                            let range =
                                CCursorRange::two(CCursor::new(start_char), CCursor::new(end_char));
                            output.state.cursor.set_char_range(Some(range));
                            output.state.clone().store(ui.ctx(), output.response.id);
                            output.response.request_focus();
                        }

                        if let Some(range) = output.state.cursor.char_range() {
                            doc.cursor_char_idx = range.primary.index;
                            doc.selection_char_range =
                                Some((range.primary.index, range.secondary.index));
                        }
                    });
                });
        });
    }
}
