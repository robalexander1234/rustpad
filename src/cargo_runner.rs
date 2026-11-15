use serde::Deserialize;
/// Background Cargo build/check/clippy/test/run executor, `cargo new` creator,
/// and JSON diagnostic parser.
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CargoSubcommand {
    Check,
    Build,
    Clippy,
    Test,
    Run,
}

impl CargoSubcommand {
    pub fn as_arg(&self) -> &'static str {
        match self {
            CargoSubcommand::Check => "check",
            CargoSubcommand::Build => "build",
            CargoSubcommand::Clippy => "clippy",
            CargoSubcommand::Test => "test",
            CargoSubcommand::Run => "run",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            CargoSubcommand::Check => "cargo check (F9)",
            CargoSubcommand::Build => "cargo build (Ctrl+F9)",
            CargoSubcommand::Clippy => "cargo clippy",
            CargoSubcommand::Test => "cargo test (Shift+F9)",
            CargoSubcommand::Run => "cargo run (F5)",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiagnosticLevel {
    Error,
    Warning,
    Note,
}

#[derive(Debug, Clone)]
pub struct CompilerDiagnostic {
    pub level: DiagnosticLevel,
    pub code: Option<String>,
    pub message: String,
    pub file_path: PathBuf,
    pub display_file: String,
    pub line_start: usize,
    pub col_start: usize,
    pub byte_start: Option<usize>,
    pub byte_end: Option<usize>,
    pub rendered: String,
}

pub enum CargoEvent {
    RawLine(String),
    Diagnostic(CompilerDiagnostic),
    Finished { success: bool, status_text: String },
}

#[derive(Deserialize)]
struct CargoJsonLine {
    reason: Option<String>,
    message: Option<RustcMessage>,
}

#[derive(Deserialize)]
struct RustcMessage {
    level: String,
    message: String,
    code: Option<RustcErrorCode>,
    spans: Vec<RustcSpan>,
    rendered: Option<String>,
}

#[derive(Deserialize)]
struct RustcErrorCode {
    code: String,
}

#[derive(Deserialize)]
struct RustcSpan {
    file_name: String,
    byte_start: usize,
    byte_end: usize,
    line_start: usize,
    column_start: usize,
    is_primary: bool,
}

/// Walks upward from `start_path` to find the nearest directory containing `Cargo.toml`.
pub fn find_cargo_root(start_path: &Path) -> Option<PathBuf> {
    let mut current = if start_path.is_file() {
        start_path.parent()?.to_path_buf()
    } else {
        start_path.to_path_buf()
    };

    loop {
        if current.join("Cargo.toml").is_file() {
            return Some(current);
        }
        if !current.pop() {
            return None;
        }
    }
}

/// Runs `cargo new [--bin | --lib] <crate_name>` inside `parent_dir`.
/// Returns `Ok((project_dir, entry_rs_file))` on success, or `Err(error_message)`.
pub fn create_cargo_new_project(
    parent_dir: &Path,
    crate_name: &str,
    is_lib: bool,
) -> Result<(PathBuf, PathBuf), String> {
    let trimmed = crate_name.trim();
    if trimmed.is_empty() {
        return Err("Crate name cannot be empty.".to_string());
    }
    if trimmed.contains(char::is_whitespace) || trimmed.contains('/') || trimmed.contains('\\') {
        return Err("Crate name must be a valid single folder/crate identifier.".to_string());
    }
    if !parent_dir.is_dir() {
        return Err(format!(
            "Parent directory does not exist: {}",
            parent_dir.display()
        ));
    }

    let project_dir = parent_dir.join(trimmed);
    if project_dir.exists() {
        return Err(format!(
            "Target folder already exists: {}",
            project_dir.display()
        ));
    }

    let mut cmd = Command::new("cargo");
    cmd.arg("new");
    if is_lib {
        cmd.arg("--lib");
    } else {
        cmd.arg("--bin");
    }
    cmd.arg(trimmed);
    cmd.current_dir(parent_dir);

    let output = cmd
        .output()
        .map_err(|e| format!("Failed to execute `cargo new`: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("`cargo new` failed: {}", stderr.trim()));
    }

    let entry_file = if is_lib {
        project_dir.join("src").join("lib.rs")
    } else {
        project_dir.join("src").join("main.rs")
    };

    Ok((project_dir, entry_file))
}

/// Spawns a background thread running `cargo <cmd> --message-format=json` in `working_dir`.
pub fn spawn_cargo_command(cmd: CargoSubcommand, working_dir: PathBuf) -> Receiver<CargoEvent> {
    let (tx, rx) = mpsc::channel();

    thread::spawn(move || {
        let mut command = Command::new("cargo");
        command.arg(cmd.as_arg());
        command.arg("--message-format=json");
        command.current_dir(&working_dir);
        command.stdout(Stdio::piped());
        command.stderr(Stdio::piped());

        let _ = tx.send(CargoEvent::RawLine(format!(
            "=== Running `cargo {} --message-format=json` in {} ===",
            cmd.as_arg(),
            working_dir.display()
        )));

        let mut child = match command.spawn() {
            Ok(c) => c,
            Err(err) => {
                let _ = tx.send(CargoEvent::RawLine(format!(
                    "Failed to launch cargo: {}",
                    err
                )));
                let _ = tx.send(CargoEvent::Finished {
                    success: false,
                    status_text: format!("Failed to spawn cargo: {}", err),
                });
                return;
            }
        };

        if let Some(stderr) = child.stderr.take() {
            let tx_err = tx.clone();
            thread::spawn(move || {
                let reader = BufReader::new(stderr);
                for line in reader.lines().map_while(Result::ok) {
                    let _ = tx_err.send(CargoEvent::RawLine(line));
                }
            });
        }

        if let Some(stdout) = child.stdout.take() {
            let reader = BufReader::new(stdout);
            for line in reader.lines().map_while(Result::ok) {
                if let Ok(parsed) = serde_json::from_str::<CargoJsonLine>(&line) {
                    if parsed.reason.as_deref() == Some("compiler-message") {
                        if let Some(msg) = parsed.message {
                            if let Some(rendered) = &msg.rendered {
                                for rline in rendered.lines() {
                                    let _ = tx.send(CargoEvent::RawLine(rline.to_string()));
                                }
                            }

                            let chosen_span = msg
                                .spans
                                .iter()
                                .find(|s| s.is_primary)
                                .or_else(|| msg.spans.first());

                            if let Some(span) = chosen_span {
                                let level = match msg.level.as_str() {
                                    "error" | "error: internal compiler error" => {
                                        DiagnosticLevel::Error
                                    }
                                    "warning" => DiagnosticLevel::Warning,
                                    _ => DiagnosticLevel::Note,
                                };

                                let full_path = working_dir.join(&span.file_name);
                                let diag = CompilerDiagnostic {
                                    level,
                                    code: msg.code.map(|c| c.code),
                                    message: msg.message.clone(),
                                    file_path: full_path,
                                    display_file: span.file_name.clone(),
                                    line_start: span.line_start,
                                    col_start: span.column_start,
                                    byte_start: Some(span.byte_start),
                                    byte_end: Some(span.byte_end),
                                    rendered: msg.rendered.unwrap_or_default(),
                                };
                                let _ = tx.send(CargoEvent::Diagnostic(diag));
                            }
                        }
                    }
                } else {
                    let _ = tx.send(CargoEvent::RawLine(line));
                }
            }
        }

        match child.wait() {
            Ok(status) => {
                let ok = status.success();
                let summary = format!("cargo {} finished with {}", cmd.as_arg(), status);
                let _ = tx.send(CargoEvent::RawLine(format!("=== {} ===", summary)));
                let _ = tx.send(CargoEvent::Finished {
                    success: ok,
                    status_text: summary,
                });
            }
            Err(e) => {
                let _ = tx.send(CargoEvent::Finished {
                    success: false,
                    status_text: format!("Error waiting on cargo: {}", e),
                });
            }
        }
    });

    rx
}
