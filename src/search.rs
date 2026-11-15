use crate::workspace::Document;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchScope {
    OpenFiles,
    Directory,
}

#[derive(Debug, Clone)]
pub struct SearchMatch {
    pub file_label: String,
    pub file_path: Option<PathBuf>,
    pub open_tab_idx: Option<usize>,
    pub line_number: usize,
    pub col_number: usize,
    pub char_offset: usize,
    pub match_char_len: usize,
    pub line_preview: String,
}

/// Finds the next or previous occurrence of `query` in `text`, supporting case-sensitivity
/// and wrap-around. Returns `(start_char_idx, end_char_idx, start_byte_idx)`.
pub fn find_in_text(
    text: &str,
    query: &str,
    current_char_idx: usize,
    case_sensitive: bool,
    backward: bool,
) -> Option<(usize, usize, usize)> {
    if query.is_empty() || text.is_empty() {
        return None;
    }

    let hay: String = if case_sensitive {
        text.to_string()
    } else {
        text.to_lowercase()
    };
    let needle: String = if case_sensitive {
        query.to_string()
    } else {
        query.to_lowercase()
    };

    let cursor_byte = text
        .char_indices()
        .nth(current_char_idx)
        .map(|(b, _)| b)
        .unwrap_or(text.len());

    // Map cursor_byte safely onto `hay`
    let hay_cursor_byte = hay
        .char_indices()
        .nth(current_char_idx)
        .map(|(b, _)| b)
        .unwrap_or(hay.len());

    let found_hay_byte = if !backward {
        // Search forward starting right after the current cursor position
        let next_start = hay[hay_cursor_byte..]
            .chars()
            .next()
            .map(|c| hay_cursor_byte + c.len_utf8())
            .unwrap_or(hay.len());

        hay[next_start..]
            .find(&needle)
            .map(|off| next_start + off)
            .or_else(|| hay.find(&needle)) // Wrap around to top
    } else {
        // Search backward before `hay_cursor_byte`
        hay[..hay_cursor_byte]
            .rfind(&needle)
            .or_else(|| hay.rfind(&needle)) // Wrap around to bottom
    }?;

    let start_char = hay[..found_hay_byte].chars().count();
    let match_chars = needle.chars().count();
    let end_char = start_char + match_chars;
    let orig_byte = text
        .char_indices()
        .nth(start_char)
        .map(|(b, _)| b)
        .unwrap_or(0);

    Some((start_char, end_char, orig_byte))
}

/// Searches across all open documents or recursively through a directory.
pub fn run_multifile_search(
    query: &str,
    case_sensitive: bool,
    scope: SearchScope,
    open_docs: &[Document],
    dir_path: &Path,
    extensions_csv: &str,
) -> Vec<SearchMatch> {
    let mut results = Vec::new();
    if query.is_empty() {
        return results;
    }

    let needle = if case_sensitive {
        query.to_string()
    } else {
        query.to_lowercase()
    };
    let match_char_len = query.chars().count();

    match scope {
        SearchScope::OpenFiles => {
            for (tab_idx, doc) in open_docs.iter().enumerate() {
                scan_text_for_matches(
                    &doc.content,
                    &needle,
                    case_sensitive,
                    match_char_len,
                    &doc.title,
                    doc.path.clone(),
                    Some(tab_idx),
                    &mut results,
                );
            }
        }
        SearchScope::Directory => {
            let exts: Vec<String> = extensions_csv
                .split(',')
                .map(|s| {
                    s.trim()
                        .trim_start_matches("*.")
                        .trim_start_matches('.')
                        .to_lowercase()
                })
                .filter(|s| !s.is_empty())
                .collect();

            let mut files = Vec::new();
            collect_project_files(dir_path, &exts, &mut files);

            for path in files {
                if let Ok(raw) = fs::read_to_string(&path) {
                    let content = raw.replace("\r\n", "\n");
                    let label = path
                        .strip_prefix(dir_path)
                        .unwrap_or(&path)
                        .display()
                        .to_string();
                    scan_text_for_matches(
                        &content,
                        &needle,
                        case_sensitive,
                        match_char_len,
                        &label,
                        Some(path),
                        None,
                        &mut results,
                    );
                }
            }
        }
    }

    results
}

fn scan_text_for_matches(
    content: &str,
    needle: &str,
    case_sensitive: bool,
    match_char_len: usize,
    file_label: &str,
    file_path: Option<PathBuf>,
    open_tab_idx: Option<usize>,
    out: &mut Vec<SearchMatch>,
) {
    let mut running_char_offset = 0usize;

    for (line_idx, line) in content.split('\n').enumerate() {
        let compare_line = if case_sensitive {
            line.to_string()
        } else {
            line.to_lowercase()
        };

        let mut search_from = 0usize;
        while let Some(pos) = compare_line[search_from..].find(needle) {
            let match_byte_in_line = search_from + pos;
            let col_chars = compare_line[..match_byte_in_line].chars().count();
            let char_offset = running_char_offset + col_chars;

            out.push(SearchMatch {
                file_label: file_label.to_string(),
                file_path: file_path.clone(),
                open_tab_idx,
                line_number: line_idx + 1,
                col_number: col_chars + 1,
                char_offset,
                match_char_len,
                line_preview: line.trim().chars().take(120).collect(),
            });

            let step = compare_line[match_byte_in_line..]
                .chars()
                .next()
                .map(|c| c.len_utf8())
                .unwrap_or(1);
            search_from = match_byte_in_line + step;
        }

        running_char_offset += line.chars().count() + 1; // +1 for '\n'
    }
}

fn collect_project_files(dir: &Path, exts: &[String], out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            // Skip target/, .git/, and hidden directories
            if name == "target" || name == ".git" || name.starts_with('.') {
                continue;
            }
            collect_project_files(&path, exts, out);
        } else if path.is_file() {
            if exts.is_empty() {
                out.push(path);
            } else if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                if exts.iter().any(|allowed| allowed == &ext.to_lowercase()) {
                    out.push(path);
                }
            }
        }
    }
}
