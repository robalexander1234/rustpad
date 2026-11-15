# RustPad v0.5 — Session Persistence, `cargo new`, Point-and-Shoot Member Help & `F1` Docs

A lightweight, PSPad-inspired programmer's editor in pure Rust (`eframe`/`egui`).

## New in v0.5
1. **Session Persistence (`rustpad_session.json`):**
   - Automatically remembers all open file paths, the active tab, each file's cursor position, the active Cargo project folder, and theme/font settings across restarts.
   - Saved automatically on exit and whenever files, tabs, or project folders change.
2. **`cargo new` Project Creator (`Ctrl+Shift+N`):**
   - Available via **File -> New Cargo Project (`cargo new`)...** and **Build -> New Cargo Project (`cargo new`)...**.
   - Pick the parent folder, enter a crate name, choose Binary (`--bin`) or Library (`--lib`), and click **Create Project**.
   - Automatically sets the Cargo Project Folder to the new crate and opens `src/main.rs` (or `src/lib.rs`) in a tab.
3. **Option A: Point-and-Shoot Member Function Browser (`Ctrl+.` or `Alt+M`):**
   - Place your cursor after `my_var.` (or `my_var.len`) and press **`Ctrl+.`** or **`Alt+M`**.
   - **Smart Heuristic Type Scanner:** Scans your current file for local declarations (`let s = "..."`, `let mut v = Vec::new()`, `let opt = Some(...)`, `fn foo(p: &Path)`, or local `struct` / `impl` blocks across all open tabs) and auto-selects the matching type category!
   - **Filter & Shoot:** Browse curated methods for `str / String`, `Vec / [T]`, `Option<T>`, `Result<T, E>`, `Iterator`, `Path / PathBuf`, `Numbers (f64 / usize / i32)`, `HashMap / HashSet`, or `Local impls`, filter by typing, and click or press `Enter` to shoot the method straight into your code with the cursor placed inside `(|)` when arguments are needed.
4. **Option B (`F1`): Instant Offline Rust Doc Lookup:**
   - Place your cursor on any identifier, type, or method (or highlight a query) and press **`F1`**.
   - Runs `rustup doc --std <symbol>` in the background to open the local offline Rust standard library documentation directly in your browser.
